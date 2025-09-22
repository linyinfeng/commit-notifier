use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::condition::Action;

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
    pub conditions: BTreeMap<String, Action>,
}

impl CommitCheckResult {
    pub fn conditions_of_action(&self, action: Action) -> BTreeSet<&String> {
        self.conditions
            .iter()
            .filter_map(|(condition, a)| if *a == action { Some(condition) } else { None })
            .collect()
    }
}

#[derive(Debug)]
pub struct BranchCheckResult {
    pub old: Option<String>,
    pub new: Option<String>,
}

#[derive(Debug)]
pub enum PRCheckResult {
    Merged(String),
    Closed,
    Waiting,
}
