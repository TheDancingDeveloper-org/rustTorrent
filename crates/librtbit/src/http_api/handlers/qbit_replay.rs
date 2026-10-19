//! Client-replay tests for the WebUI API v2 compat layer.
//!
//! Each fixture in `qbit_replay/` is a recorded request sequence from a real
//! client (Sonarr's download-client proxy, the Python `qbittorrent-api`
//! library), replayed in order against the in-process compat router. Each step
//! asserts what that client reads from the response, so a regression that would
//! break the client fails here even when every endpoint still matches its
//! declared shape in `qbit_parity_spec.json`.
//!
//! The same fixtures drive the differential harness
//! (`scripts/qbit-diff/qbit_diff.py`), which replays them against a real
//! qBittorrent container and a running rustTorrent and compares the responses
//! field by field. Step fields:
//!
//! - `method`, `path` (relative to `/api/v2`), `query`, `form`, `multipart`
//!   (`torrent: true` attaches `qbit_replay/fixture.torrent`, plus `fields`);
//! - `live_only`: skipped here (e.g. `auth/login`, which needs a peer address);
//! - `expect`: `status` or `status_in`, `text` (exact body), `eventually`
//!   (retry until the step's assertions pass), and `json`, a list of
//!   `{path, op}` checks where `path` is dotted (`0.hash`, `""` for the root)
//!   and `op` is one of `eq`, `one_of`, `len`, `exists`, `contains`;
//! - `known_gap`: `{status, note}` for an unfixed divergence from upstream;
//!   the replay asserts `known_gap.status` instead of `expect.status`;
//! - `diff`: only read by the differential harness.
//!
//! `{{save_path}}`, `{{username}}` and `{{password}}` are substituted before a
//! step is sent.

use std::time::Duration;

use http::StatusCode;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::qbit_parity::{ProbeResponse, make_router, multipart_add_body, send};

const FIXTURE_TORRENT: &[u8] = include_bytes!("qbit_replay/fixture.torrent");

const FIXTURES: &[(&str, &str)] = &[
    ("sonarr", include_str!("qbit_replay/sonarr.json")),
    (
        "qbittorrent_api",
        include_str!("qbit_replay/qbittorrent_api.json"),
    ),
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    name: String,
    #[allow(dead_code)]
    client: String,
    #[allow(dead_code)]
    #[serde(default)]
    notes: String,
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    name: String,
    method: String,
    path: String,
    #[serde(default)]
    query: Map<String, Value>,
    #[serde(default)]
    form: Map<String, Value>,
    multipart: Option<Multipart>,
    #[serde(default)]
    live_only: bool,
    expect: Expect,
    #[allow(dead_code)]
    diff: Option<Value>,
    known_gap: Option<KnownGap>,
}

/// A recorded divergence from upstream that is not fixed yet. The replay
/// asserts the current status instead of `expect.status`, so fixing the gap
/// fails this test until the entry is removed.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KnownGap {
    status: u16,
    #[allow(dead_code)]
    note: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Multipart {
    #[serde(default)]
    torrent: bool,
    #[serde(default)]
    fields: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Expect {
    status: Option<u16>,
    status_in: Option<Vec<u16>>,
    text: Option<String>,
    #[serde(default)]
    eventually: bool,
    #[serde(default)]
    json: Vec<JsonCheck>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JsonCheck {
    path: String,
    eq: Option<Value>,
    one_of: Option<Vec<Value>>,
    len: Option<usize>,
    exists: Option<bool>,
    contains: Option<Value>,
}

struct Vars {
    save_path: String,
}

impl Vars {
    fn apply(&self, s: &str) -> String {
        s.replace("{{save_path}}", &self.save_path)
            .replace("{{username}}", "admin")
            .replace("{{password}}", "adminadmin")
    }

    fn pairs(&self, map: &Map<String, Value>) -> Vec<(String, String)> {
        map.iter()
            .map(|(k, v)| {
                let v = match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                (k.clone(), self.apply(&v))
            })
            .collect()
    }
}

/// Resolves a dotted path (`""` is the root; numeric segments index arrays).
fn lookup<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    if path.is_empty() {
        return Some(root);
    }
    path.split('.').try_fold(root, |v, seg| match v {
        Value::Array(items) => seg.parse::<usize>().ok().and_then(|i| items.get(i)),
        Value::Object(map) => map.get(seg),
        _ => None,
    })
}

fn json_eq(actual: &Value, expected: &Value) -> bool {
    match (actual.as_f64(), expected.as_f64()) {
        (Some(a), Some(e)) => (a - e).abs() < 1e-9,
        _ => actual == expected,
    }
}

fn check_json(body: &Value, check: &JsonCheck) -> Result<(), String> {
    let found = lookup(body, &check.path);
    let path = &check.path;
    if let Some(want) = check.exists
        && found.is_some() != want
    {
        return Err(format!("`{path}` exists={} (want {want})", found.is_some()));
    }
    let needs_value = check.eq.is_some()
        || check.one_of.is_some()
        || check.len.is_some()
        || check.contains.is_some();
    if !needs_value {
        return Ok(());
    }
    let Some(v) = found else {
        return Err(format!("`{path}` is absent"));
    };
    if let Some(want) = &check.eq
        && !json_eq(v, want)
    {
        return Err(format!("`{path}` = {v} (want {want})"));
    }
    if let Some(options) = &check.one_of
        && !options.iter().any(|o| json_eq(v, o))
    {
        return Err(format!("`{path}` = {v} (want one of {options:?})"));
    }
    if let Some(want) = check.len {
        let len = match v {
            Value::Array(a) => a.len(),
            Value::Object(o) => o.len(),
            _ => return Err(format!("`{path}` = {v} has no length")),
        };
        if len != want {
            return Err(format!("`{path}` has length {len} (want {want}): {v}"));
        }
    }
    if let Some(want) = &check.contains {
        let ok = match (v, want) {
            (Value::Array(a), w) => a.iter().any(|x| json_eq(x, w)),
            (Value::String(s), Value::String(w)) => s.contains(w.as_str()),
            _ => false,
        };
        if !ok {
            return Err(format!("`{path}` = {v} does not contain {want}"));
        }
    }
    Ok(())
}

fn check_step(step: &Step, response: &ProbeResponse) -> Vec<String> {
    let mut errors = Vec::new();
    let status = response.status.as_u16();
    let expected_status = match &step.known_gap {
        Some(gap) => Some(gap.status),
        None => step.expect.status,
    };
    if let Some(want) = expected_status
        && status != want
    {
        errors.push(format!("status {status} (want {want})"));
    }
    if let Some(allowed) = &step.expect.status_in
        && !allowed.contains(&status)
    {
        errors.push(format!("status {status} (want one of {allowed:?})"));
    }
    let body = String::from_utf8_lossy(&response.body);
    if let Some(want) = &step.expect.text
        && body != want.as_str()
    {
        errors.push(format!("body {body:?} (want {want:?})"));
    }
    if !step.expect.json.is_empty() {
        match serde_json::from_slice::<Value>(&response.body) {
            Ok(json) => errors.extend(
                step.expect
                    .json
                    .iter()
                    .filter_map(|c| check_json(&json, c).err()),
            ),
            Err(e) => errors.push(format!("body is not JSON ({e}): {body:?}")),
        }
    }
    errors
}

fn build_request(step: &Step, vars: &Vars) -> http::Request<axum::body::Body> {
    let mut uri = step.path.clone();
    let query = vars.pairs(&step.query);
    if !query.is_empty() {
        uri.push('?');
        uri.push_str(&serde_urlencoded::to_string(&query).unwrap());
    }
    let builder = http::Request::builder()
        .method(step.method.as_str())
        .uri(uri);
    if let Some(multipart) = &step.multipart {
        let boundary = "qbitReplayBoundary";
        let fields = vars.pairs(&multipart.fields);
        let fields: Vec<(&str, &str)> = fields
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert!(
            multipart.torrent,
            "{}: multipart without a torrent",
            step.name
        );
        return builder
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(multipart_add_body(boundary, FIXTURE_TORRENT, &fields).into())
            .unwrap();
    }
    if step.method == "POST" {
        return builder
            .header("content-type", "application/x-www-form-urlencoded")
            .body(
                serde_urlencoded::to_string(vars.pairs(&step.form))
                    .unwrap()
                    .into(),
            )
            .unwrap();
    }
    builder.body(axum::body::Body::empty()).unwrap()
}

async fn replay(fixture: &Fixture) {
    let (router, _session, output) = make_router().await;
    let vars = Vars {
        save_path: output.path().to_string_lossy().into_owned(),
    };
    for (i, step) in fixture.steps.iter().enumerate() {
        if step.live_only {
            continue;
        }
        let attempts = if step.expect.eventually { 400 } else { 1 };
        let mut errors = Vec::new();
        for attempt in 0..attempts {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            let response = send(&router, build_request(step, &vars)).await;
            errors = check_step(step, &response);
            if errors.is_empty() {
                break;
            }
        }
        assert!(
            errors.is_empty(),
            "{} step {i} ({}): {} {}: {errors:?}",
            fixture.name,
            step.name,
            step.method,
            step.path,
        );
    }
}

fn parse(name: &str, raw: &str) -> Fixture {
    let fixture: Fixture = serde_json::from_str(raw)
        .unwrap_or_else(|e| panic!("qbit_replay/{name}.json is malformed: {e}"));
    assert_eq!(fixture.name, name);
    fixture
}

#[test]
fn fixtures_are_well_formed() {
    for (name, raw) in FIXTURES {
        let fixture = parse(name, raw);
        for step in &fixture.steps {
            assert!(
                step.path.starts_with('/'),
                "{name}/{}: path must start with /",
                step.name
            );
            assert!(
                matches!(step.method.as_str(), "GET" | "POST"),
                "{name}/{}: unsupported method",
                step.name
            );
            assert!(
                step.expect.status.is_some() || step.expect.status_in.is_some(),
                "{name}/{}: every step pins a status",
                step.name
            );
        }
    }
}

#[test]
fn lookup_and_checks() {
    let body: Value = serde_json::json!([{ "hash": "ab", "size": 2, "tags": "x, y" }]);
    let check = |path: &str, extra: Value| -> Result<(), String> {
        let mut c = serde_json::json!({ "path": path });
        c.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        check_json(&body, &serde_json::from_value(c).unwrap())
    };
    assert!(check("", serde_json::json!({ "len": 1 })).is_ok());
    assert!(check("0.size", serde_json::json!({ "eq": 2.0 })).is_ok());
    assert!(check("0.hash", serde_json::json!({ "eq": "cd" })).is_err());
    assert!(check("0.tags", serde_json::json!({ "contains": "y" })).is_ok());
    assert!(check("0.nope", serde_json::json!({ "exists": false })).is_ok());
    assert!(check("1.hash", serde_json::json!({ "exists": true })).is_err());
}

#[tokio::test]
async fn replay_sonarr() {
    let (name, raw) = FIXTURES[0];
    replay(&parse(name, raw)).await;
}

#[tokio::test]
async fn replay_qbittorrent_api() {
    let (name, raw) = FIXTURES[1];
    replay(&parse(name, raw)).await;
}

#[test]
fn fixture_status_codes_are_valid() {
    for (name, raw) in FIXTURES {
        for step in parse(name, raw).steps {
            for code in step
                .expect
                .status
                .iter()
                .chain(step.expect.status_in.iter().flatten())
            {
                assert!(StatusCode::from_u16(*code).is_ok(), "{name}/{}", step.name);
            }
        }
    }
}
