use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct ChatRepoResults {
    pub commits: BTreeMap<String, CommitResults>,
    pub branches: BTreeMap<String, BranchResults>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommitResults {
    pub branches: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BranchResults {
    pub commit: Option<String>,
}

#[derive(Debug)]
pub struct CommitCheckResult {
    pub all: BTreeSet<String>,
    pub new: BTreeSet<String>,
    pub removed_by_condition: Option<String>,
}

#[derive(Debug)]
pub struct BranchCheckResult {
    pub old: Option<String>,
    pub new: Option<String>,
}

#[derive(Debug)]
pub struct ConditionCheckResult {
    pub removed: Vec<String>,
}
