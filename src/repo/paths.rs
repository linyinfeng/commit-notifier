use std::{path::PathBuf, sync::LazyLock};

use crate::options;

#[derive(Debug, Clone)]
pub struct RepoPaths {
    pub outer: PathBuf,
    pub repo: PathBuf,
    pub settings: PathBuf,
    pub cache: PathBuf,
}

pub static GLOBAL_REPO_OUTER: LazyLock<PathBuf> =
    LazyLock::new(|| options::get().working_dir.join("repositories"));

impl RepoPaths {
    pub fn new(name: &str) -> RepoPaths {
        let outer = GLOBAL_REPO_OUTER.join(name);
        Self {
            outer: outer.clone(),
            repo: outer.join("repo"),
            settings: outer.join("settings.json"),
            cache: outer.join("cache.sqlite"),
        }
    }
}
