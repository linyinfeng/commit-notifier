use serde::{Deserialize, Serialize};

use crate::condition::Condition;
use crate::error::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InBranchCondition {
    pub branch: String,
}

impl Condition for InBranchCondition {
    fn meet(&self, result: &crate::repo::results::CommitResults) -> bool {
        result.branches.contains(&self.branch)
    }
}

impl InBranchCondition {
    pub fn parse(s: &str) -> Result<Self, Error> {
        Ok(InBranchCondition {
            branch: s.to_string(),
        })
    }
}
