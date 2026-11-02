//! Torrent table preferences kept in a small local JSON file: the sort column
//! and direction, and any column widths that differ from the defaults.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

const FILE_NAME: &str = "table.json";

/// Smallest width a stored column may take, so a bad file cannot hide a column.
pub const MIN_COLUMN_WIDTH: f32 = 40.0;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TablePrefs {
    /// Sort column id and whether the order is ascending.
    pub sort: Option<(String, bool)>,
    pub widths: BTreeMap<String, f32>,
}

impl TablePrefs {
    /// `<config dir>/rtbit-fluent/table.json`, or `None` when no home is known.
    #[must_use]
    pub fn default_path() -> Option<PathBuf> {
        let base = if cfg!(windows) {
            std::env::var_os("APPDATA").map(PathBuf::from)
        } else {
            std::env::var_os("XDG_CONFIG_HOME")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config"))
                })
        }?;
        Some(base.join("rtbit-fluent").join(FILE_NAME))
    }

    /// Read the file; a missing or unreadable file gives the defaults.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .map(|value| Self::from_json(&value))
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(&self.to_json())?)
    }

    fn to_json(&self) -> Value {
        let sort = self
            .sort
            .as_ref()
            .map(|(column, ascending)| json!({ "column": column, "ascending": ascending }));
        json!({ "sort": sort, "widths": self.widths })
    }

    fn from_json(value: &Value) -> Self {
        let sort = value.get("sort").and_then(|sort| {
            let column = sort.get("column")?.as_str()?.to_owned();
            let ascending = sort.get("ascending")?.as_bool()?;
            Some((column, ascending))
        });
        let widths = value
            .get("widths")
            .and_then(Value::as_object)
            .map(|map| {
                map.iter()
                    .filter_map(|(id, width)| {
                        let width = width.as_f64()? as f32;
                        (width.is_finite() && width >= MIN_COLUMN_WIDTH)
                            .then(|| (id.clone(), width))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self { sort, widths }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("rtbit-fluent-{name}-{}", std::process::id()))
    }

    #[test]
    fn round_trips_through_the_file() {
        let dir = temp_path("roundtrip");
        let path = dir.join("nested").join(FILE_NAME);
        let mut prefs = TablePrefs {
            sort: Some(("speed".to_owned(), false)),
            ..Default::default()
        };
        prefs.widths.insert("name".to_owned(), 411.5);
        prefs.save(&path).unwrap();
        assert_eq!(TablePrefs::load(&path), prefs);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_or_corrupt_files_give_defaults() {
        let dir = temp_path("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        assert_eq!(TablePrefs::load(&path), TablePrefs::default());
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(TablePrefs::load(&path), TablePrefs::default());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn implausible_widths_are_dropped() {
        let value = json!({ "widths": { "name": 5, "size": 120, "eta": "wide" } });
        let prefs = TablePrefs::from_json(&value);
        assert_eq!(prefs.widths.len(), 1);
        assert_eq!(prefs.widths["size"], 120.0);
    }
}
