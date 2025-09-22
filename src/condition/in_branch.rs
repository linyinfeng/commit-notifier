use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::chat::results::CommitCheckResult;
use crate::condition::{Action, Condition};
use crate::error::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InBranchCondition {
    #[serde(with = "serde_regex")]
    pub branch_regex: Regex,
}

impl Condition for InBranchCondition {
    fn check(&self, check_results: &CommitCheckResult) -> Action {
        if check_results
            .all
            .iter()
            .any(|b| self.branch_regex.is_match(b))
        {
            Action::Remove
        } else {
            Action::None
        }
    }
}

impl InBranchCondition {
    pub fn parse(s: &str) -> Result<Self, Error> {
        Ok(InBranchCondition {
            branch_regex: Regex::new(&format!("^{s}$"))?,
        })
    }
}
