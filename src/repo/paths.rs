use std::{path::PathBuf, sync::LazyLock};

use regex::Regex;

use crate::{error::Error, options};

#[derive(Debug, Clone)]
pub struct RepoPaths {
    pub outer: PathBuf,
    pub repo: PathBuf,
    pub settings: PathBuf,
    pub cache: PathBuf,
}

pub static GLOBAL_REPO_OUTER: LazyLock<PathBuf> =
    LazyLock::new(|| options::get().working_dir.join("repositories"));

static NAME_RE: once_cell::sync::Lazy<Regex> =
    once_cell::sync::Lazy::new(|| Regex::new("^[a-zA-Z0-9_\\-]*$").unwrap());

impl RepoPaths {
    pub fn new(name: &str) -> Result<RepoPaths, Error> {
        if !NAME_RE.is_match(name) {
            return Err(Error::Name(name.to_string()));
        }

        let outer = GLOBAL_REPO_OUTER.join(name);
        Ok(Self {
            outer: outer.clone(),
            repo: outer.join("repo"),
            settings: outer.join("settings.json"),
            cache: outer.join("cache.sqlite"),
        })
    }
}
