use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    pub branch_regex: String,
    pub commits: BTreeMap<String, CommitSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommitSettings {
    pub comment: String,
}
