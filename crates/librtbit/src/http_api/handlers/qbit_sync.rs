//! RID delta engine for the WebUI API v2 `sync/*` endpoints.
//!
//! Each `sync/maindata` (or `sync/torrentPeers`) response is a JSON snapshot
//! tagged with a fresh response id (`rid`). The snapshot is kept for a while;
//! a client that polls again with that `rid` gets only what changed since it,
//! the same partial-update encoding the upstream server uses:
//!
//! - a map of objects (`torrents`, `categories`, `peers`) carries new entries
//!   in full and changed entries with only their changed fields; keys that
//!   vanished are listed in `<name>_removed`;
//! - an array (`tags`) carries the added elements; removed ones go in
//!   `<name>_removed`;
//! - a flat object (`server_state`) carries its changed fields;
//! - a scalar is sent when it changed;
//! - anything unchanged is omitted.
//!
//! A `rid` of 0, an unknown or expired `rid`, or one taken from a different
//! scope (e.g. another torrent's peers) yields a `full_update`. Snapshots are
//! shared across clients — rids are globally unique, so no per-client state is
//! needed and the engine works the same with or without SID authentication.

use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use serde_json::{Map, Value};

/// Snapshots retained across all scopes. Each polling client pins at most one
/// at a time, so this bounds memory while covering many concurrent clients.
const MAX_SNAPSHOTS: usize = 64;
/// How long a snapshot stays usable as a delta base.
const SNAPSHOT_TTL: Duration = Duration::from_secs(300);

struct Snapshot {
    rid: u64,
    scope: String,
    data: Map<String, Value>,
    taken: Instant,
}

pub(crate) struct SyncSnapshots {
    inner: Mutex<Inner>,
}

struct Inner {
    last_rid: u64,
    snapshots: VecDeque<Snapshot>,
}

impl Default for SyncSnapshots {
    fn default() -> Self {
        Self {
            inner: Mutex::new(Inner {
                last_rid: 0,
                snapshots: VecDeque::new(),
            }),
        }
    }
}

impl SyncSnapshots {
    /// Records `current` under a new rid and returns the response body: a
    /// delta against the snapshot `client_rid` names when it is still held
    /// for `scope`, otherwise a full update.
    pub(crate) fn respond(
        &self,
        scope: &str,
        client_rid: u64,
        current: Map<String, Value>,
    ) -> Map<String, Value> {
        let now = Instant::now();
        let mut inner = self.inner.lock();
        inner
            .snapshots
            .retain(|s| now.duration_since(s.taken) < SNAPSHOT_TTL);

        let base = (client_rid != 0)
            .then(|| {
                inner
                    .snapshots
                    .iter()
                    .find(|s| s.rid == client_rid && s.scope == scope)
            })
            .flatten();
        let mut body = match base {
            Some(base) => diff(&base.data, &current),
            None => {
                let mut full = current.clone();
                full.insert("full_update".into(), Value::Bool(true));
                full
            }
        };

        inner.last_rid += 1;
        let rid = inner.last_rid;
        body.insert("rid".into(), rid.into());
        if inner.snapshots.len() >= MAX_SNAPSHOTS {
            inner.snapshots.pop_front();
        }
        inner.snapshots.push_back(Snapshot {
            rid,
            scope: scope.to_owned(),
            data: current,
            taken: now,
        });
        body
    }
}

/// Encodes `current` as a partial update relative to `prev` (see the module
/// docs for the encoding).
pub(crate) fn diff(prev: &Map<String, Value>, current: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for (key, cur) in current {
        let Some(old) = prev.get(key) else {
            out.insert(key.clone(), cur.clone());
            continue;
        };
        match (old, cur) {
            (Value::Object(old), Value::Object(cur)) => {
                let mut changed = Map::new();
                for (k, v) in cur {
                    match (old.get(k), v) {
                        (None, _) => {
                            changed.insert(k.clone(), v.clone());
                        }
                        (Some(Value::Object(o)), Value::Object(c)) => {
                            let fields = changed_fields(o, c);
                            if !fields.is_empty() {
                                changed.insert(k.clone(), Value::Object(fields));
                            }
                        }
                        (Some(o), _) if o != v => {
                            changed.insert(k.clone(), v.clone());
                        }
                        _ => {}
                    }
                }
                if !changed.is_empty() {
                    out.insert(key.clone(), Value::Object(changed));
                }
                let removed: Vec<Value> = old
                    .keys()
                    .filter(|k| !cur.contains_key(*k))
                    .map(|k| Value::String(k.clone()))
                    .collect();
                if !removed.is_empty() {
                    out.insert(format!("{key}_removed"), Value::Array(removed));
                }
            }
            (Value::Array(old), Value::Array(cur)) => {
                let added: Vec<Value> = cur.iter().filter(|v| !old.contains(v)).cloned().collect();
                let removed: Vec<Value> =
                    old.iter().filter(|v| !cur.contains(v)).cloned().collect();
                if !added.is_empty() {
                    out.insert(key.clone(), Value::Array(added));
                }
                if !removed.is_empty() {
                    out.insert(format!("{key}_removed"), Value::Array(removed));
                }
            }
            (old, cur) if old != cur => {
                out.insert(key.clone(), cur.clone());
            }
            _ => {}
        }
    }
    out
}

fn changed_fields(old: &Map<String, Value>, cur: &Map<String, Value>) -> Map<String, Value> {
    cur.iter()
        .filter(|(k, v)| old.get(*k) != Some(*v))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn obj(v: Value) -> Map<String, Value> {
        match v {
            Value::Object(m) => m,
            _ => panic!("not an object"),
        }
    }

    #[test]
    fn diff_encodes_partial_updates() {
        let prev = obj(json!({
            "torrents": {
                "a": {"name": "A", "progress": 0.1, "state": "downloading"},
                "b": {"name": "B", "progress": 1.0},
            },
            "categories": {"movies": {"name": "movies", "savePath": ""}},
            "tags": ["hd", "old"],
            "server_state": {"dl_info_speed": 10, "queueing": false},
            "show_flags": true,
        }));
        let cur = obj(json!({
            "torrents": {
                "a": {"name": "A", "progress": 0.5, "state": "downloading"},
                "c": {"name": "C", "progress": 0.0},
            },
            "categories": {"movies": {"name": "movies", "savePath": ""}},
            "tags": ["hd", "new"],
            "server_state": {"dl_info_speed": 20, "queueing": false},
            "show_flags": true,
        }));
        assert_eq!(
            Value::Object(diff(&prev, &cur)),
            json!({
                "torrents": {
                    "a": {"progress": 0.5},
                    "c": {"name": "C", "progress": 0.0},
                },
                "torrents_removed": ["b"],
                "tags": ["new"],
                "tags_removed": ["old"],
                "server_state": {"dl_info_speed": 20},
            })
        );
        assert!(diff(&cur, &cur).is_empty());
    }

    #[test]
    fn snapshots_hand_out_deltas_per_scope() {
        let store = SyncSnapshots::default();
        let s1 = obj(json!({"torrents": {"a": {"progress": 0.1}}, "tags": []}));
        let s2 = obj(json!({"torrents": {"a": {"progress": 0.2}}, "tags": []}));

        let first = store.respond("maindata", 0, s1.clone());
        assert_eq!(first["full_update"], true);
        let rid = first["rid"].as_u64().unwrap();

        let second = store.respond("maindata", rid, s2.clone());
        assert_eq!(
            Value::Object(second.clone()),
            json!({"rid": rid + 1, "torrents": {"a": {"progress": 0.2}}})
        );

        // nothing changed: only the rid comes back
        let third = store.respond("maindata", rid + 1, s2.clone());
        assert_eq!(Value::Object(third), json!({"rid": rid + 2}));

        // an unknown rid, or one from another scope, forces a full update
        assert_eq!(
            store.respond("maindata", 9_999, s2.clone())["full_update"],
            true
        );
        assert_eq!(
            store.respond("peers:x", rid, s2.clone())["full_update"],
            true
        );

        // old snapshots are evicted once the cap is reached
        for _ in 0..MAX_SNAPSHOTS {
            store.respond("maindata", 0, s1.clone());
        }
        assert_eq!(store.respond("maindata", rid, s2)["full_update"], true);
    }
}
