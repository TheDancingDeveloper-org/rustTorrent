use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::bail;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::session::ShareLimits;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentCategory {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ratio_limit: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seeding_time_limit: Option<std::time::Duration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inactive_seeding_time_limit: Option<std::time::Duration>,
}

impl TorrentCategory {
    pub fn share_limits(&self) -> ShareLimits {
        ShareLimits {
            max_ratio: self.ratio_limit,
            max_seeding_time: self.seeding_time_limit,
            max_inactive_seeding_time: self.inactive_seeding_time_limit,
        }
    }
}

#[derive(Debug)]
pub struct CategoryManager {
    categories: RwLock<HashMap<String, TorrentCategory>>,
}

impl Default for CategoryManager {
    fn default() -> Self {
        Self::new()
    }
}

impl CategoryManager {
    pub fn new() -> Self {
        Self {
            categories: RwLock::new(HashMap::new()),
        }
    }

    pub fn from_categories(cats: HashMap<String, TorrentCategory>) -> Self {
        Self {
            categories: RwLock::new(cats),
        }
    }

    pub fn list(&self) -> HashMap<String, TorrentCategory> {
        self.categories.read().clone()
    }

    pub fn create(
        &self,
        name: String,
        save_path: Option<PathBuf>,
        limits: ShareLimits,
    ) -> anyhow::Result<()> {
        let mut cats = self.categories.write();
        if cats.contains_key(&name) {
            bail!("category '{name}' already exists");
        }
        cats.insert(
            name.clone(),
            TorrentCategory {
                name,
                save_path,
                ratio_limit: limits.max_ratio,
                seeding_time_limit: limits.max_seeding_time,
                inactive_seeding_time_limit: limits.max_inactive_seeding_time,
            },
        );
        Ok(())
    }

    pub fn edit(
        &self,
        name: &str,
        save_path: Option<PathBuf>,
        limits: ShareLimits,
    ) -> anyhow::Result<()> {
        let mut cats = self.categories.write();
        let cat = cats.get_mut(name);
        match cat {
            Some(cat) => {
                cat.save_path = save_path;
                cat.ratio_limit = limits.max_ratio;
                cat.seeding_time_limit = limits.max_seeding_time;
                cat.inactive_seeding_time_limit = limits.max_inactive_seeding_time;
                Ok(())
            }
            None => bail!("category '{name}' does not exist"),
        }
    }

    pub fn remove(&self, name: &str) -> anyhow::Result<()> {
        let mut cats = self.categories.write();
        if cats.remove(name).is_none() {
            bail!("category '{name}' does not exist");
        }
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<TorrentCategory> {
        self.categories.read().get(name).cloned()
    }

    pub fn exists(&self, name: &str) -> bool {
        self.categories.read().contains_key(name)
    }

    pub fn snapshot(&self) -> HashMap<String, TorrentCategory> {
        self.categories.read().clone()
    }
}
