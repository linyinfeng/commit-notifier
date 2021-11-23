use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct Results {
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
