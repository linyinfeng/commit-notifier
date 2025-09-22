use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::chat::results::CommitCheckResult;
use crate::condition::{Action, Condition};
use crate::error::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuppressFromToCondition {
    #[serde(with = "serde_regex")]
    pub from_regex: Regex,
    #[serde(with = "serde_regex")]
    pub to_regex: Regex,
}

impl Condition for SuppressFromToCondition {
    fn check(&self, check_results: &CommitCheckResult) -> Action {
        let mut old = check_results.all.difference(&check_results.new);
        if old.any(|old_branch| self.from_regex.is_match(old_branch))
            && check_results
                .new
                .iter()
                .any(|new_branch| self.to_regex.is_match(new_branch))
        {
            Action::SuppressNotification
        } else {
            Action::None
        }
    }
}

impl SuppressFromToCondition {
    pub fn parse(s: &str) -> Result<Self, Error> {
        Ok(serde_json::from_str(s)?)
    }
}
