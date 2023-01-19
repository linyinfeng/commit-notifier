use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::condition::GeneralCondition;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    pub branch_regex: String,
    pub commits: BTreeMap<String, CommitSettings>,
    pub branches: BTreeMap<String, BranchSettings>,
    pub conditions: BTreeMap<String, ConditionSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommitSettings {
    pub comment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BranchSettings {
    // currently nothing
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConditionSettings {
    pub condition: GeneralCondition,
}
