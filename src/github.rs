use std::fmt::{self, Display};

use octocrab::models::pulls::PullRequest;
use serde::{Deserialize, Serialize};

use crate::error::Error;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GitHubInfo {
    owner: String,
    repo: String,
}

impl Display for GitHubInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.repo)
    }
}

pub fn parse_github_info(s: &str) -> Result<GitHubInfo, String> {
    let v: Vec<_> = s.split('/').collect();
    if v.len() != 2 {
        Err("invalid github info format, 'owner/repo' required".to_string())
    } else {
        Ok(GitHubInfo {
            owner: v[0].to_string(),
            repo: v[1].to_string(),
        })
    }
}

pub async fn get_pr(info: &GitHubInfo, pr_id: u64) -> Result<PullRequest, Error> {
    Ok(octocrab::instance()
        .pulls(&info.owner, &info.repo)
        .get(pr_id)
        .await?)
}
