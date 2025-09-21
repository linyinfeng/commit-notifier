use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::chat::results::CommitResults;
use crate::condition::Condition;
use crate::error::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InBranchCondition {
    pub branch_regex: String,
}

impl Condition for InBranchCondition {
    fn meet(&self, result: &CommitResults) -> bool {
        let regex = self.regex().unwrap();
        result.branches.iter().any(|b| regex.is_match(b))
    }
}

impl InBranchCondition {
    pub fn parse(s: &str) -> Result<Self, Error> {
        let result = InBranchCondition {
            branch_regex: s.to_string(),
        };
        let _ = result.regex()?;
        Ok(result)
    }

    pub fn regex(&self) -> Result<Regex, Error> {
        Ok(Regex::new(&format!("^{}$", self.branch_regex))?)
    }
}
