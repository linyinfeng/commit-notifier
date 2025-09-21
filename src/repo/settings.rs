use serde::{Deserialize, Serialize};

use crate::github::GitHubInfo;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RepoSettings {
    pub branch_regex: String,
    #[serde(default)]
    pub github_info: Option<GitHubInfo>,
}
