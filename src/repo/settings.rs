use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    pub branch_regex: String,
    pub commits: BTreeMap<String, CommitSettings>,
    pub branches: BTreeMap<String, BranchSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommitSettings {
    pub comment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BranchSettings {
    // currently nothing
}
