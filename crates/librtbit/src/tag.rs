use std::collections::BTreeSet;

use parking_lot::RwLock;

/// Session-wide tag registry. Tags are plain labels; a torrent's own tag set
/// lives on its shared state, and every tag a torrent carries is also
/// registered here so it shows up in listings.
#[derive(Debug, Default)]
pub struct TagManager {
    tags: RwLock<BTreeSet<String>>,
}

/// Trim, drop empty entries and deduplicate a caller-supplied tag list.
pub fn normalize_tags<I, S>(tags: I) -> BTreeSet<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    tags.into_iter()
        .map(|tag| tag.as_ref().trim().to_owned())
        .filter(|tag| !tag.is_empty())
        .collect()
}

impl TagManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_tags(tags: BTreeSet<String>) -> Self {
        Self {
            tags: RwLock::new(tags),
        }
    }

    /// All registered tags, sorted.
    pub fn list(&self) -> Vec<String> {
        self.tags.read().iter().cloned().collect()
    }

    /// Register tags. Returns true if any tag was new.
    pub fn create<'a>(&self, tags: impl IntoIterator<Item = &'a String>) -> bool {
        let mut all = self.tags.write();
        let mut changed = false;
        for tag in tags {
            changed |= all.insert(tag.clone());
        }
        changed
    }

    /// Unregister tags. Returns true if any tag was removed.
    pub fn delete<'a>(&self, tags: impl IntoIterator<Item = &'a String>) -> bool {
        let mut all = self.tags.write();
        let mut changed = false;
        for tag in tags {
            changed |= all.remove(tag);
        }
        changed
    }

    pub fn exists(&self, tag: &str) -> bool {
        self.tags.read().contains(tag)
    }

    /// Rename a registered tag. Returns false when `from` is not registered or
    /// `to` already is. Does not touch per-torrent sets.
    pub fn rename(&self, from: &str, to: &str) -> bool {
        let mut all = self.tags.write();
        if !all.remove(from) {
            return false;
        }
        if !all.insert(to.to_owned()) {
            all.insert(from.to_owned());
            return false;
        }
        true
    }

    pub fn snapshot(&self) -> BTreeSet<String> {
        self.tags.read().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_trims_and_dedups() {
        let got = normalize_tags([" tv", "", "hd ", "tv", "  "]);
        assert_eq!(got.into_iter().collect::<Vec<_>>(), vec!["hd", "tv"]);
    }

    #[test]
    fn create_and_delete_report_changes() {
        let mgr = TagManager::new();
        let tags = normalize_tags(["a", "b"]);
        assert!(mgr.create(&tags));
        assert!(!mgr.create(&tags));
        assert_eq!(mgr.list(), vec!["a", "b"]);
        assert!(mgr.delete(&normalize_tags(["a", "zz"])));
        assert!(!mgr.exists("a"));
        assert!(!mgr.delete(&normalize_tags(["zz"])));
    }

    #[test]
    fn rename_moves_a_registered_tag() {
        let mgr = TagManager::new();
        mgr.create(&normalize_tags(["a"]));
        assert!(!mgr.rename("missing", "b"));
        assert!(!mgr.rename("a", "a") || mgr.exists("a"));
        assert!(mgr.rename("a", "b"));
        assert!(!mgr.exists("a"));
        assert!(mgr.exists("b"));
        mgr.create(&normalize_tags(["c"]));
        assert!(!mgr.rename("b", "c"));
        assert!(mgr.exists("b"));
    }
}
