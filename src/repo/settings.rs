use std::collections::BTreeMap;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::{condition::GeneralCondition, github::GitHubInfo};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoSettings {
    #[serde(with = "serde_regex", default = "default_branch_regex")]
    pub branch_regex: Regex,
    #[serde(default)]
    pub github_info: Option<GitHubInfo>,
    #[serde(default)]
    pub conditions: BTreeMap<String, ConditionSettings>,
}

fn default_branch_regex() -> Regex {
    Regex::new("^$").unwrap()
}

impl Default for RepoSettings {
    fn default() -> Self {
        Self {
            branch_regex: default_branch_regex(),
            github_info: Default::default(),
            conditions: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConditionSettings {
    pub condition: GeneralCondition,
}
