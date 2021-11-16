use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct Results {
    pub data: BTreeMap<String, CommitResults>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommitResults {
    pub branches: BTreeSet<String>,
}
