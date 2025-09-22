use std::fmt::{self, Display};

use octocrab::models::pulls::PullRequest;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::Error;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GitHubInfo {
    pub owner: String,
    pub repo: String,
}

impl Display for GitHubInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.repo)
    }
}

impl PartialEq for GitHubInfo {
    fn eq(&self, other: &Self) -> bool {
        // for backward compatibility
        self.owner.eq_ignore_ascii_case(&other.owner) && self.repo.eq_ignore_ascii_case(&other.repo)
    }
}

impl Eq for GitHubInfo {}

pub static GITHUB_PATH_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new("^/([a-zA-Z0-9_.-]+)/([a-zA-Z0-9_.-]+?)(\\.git)?$").unwrap());

impl GitHubInfo {
    pub fn new(owner: String, repo: String) -> Self {
        Self { owner, repo }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        let v: Vec<_> = s.split('/').collect();
        if v.len() != 2 {
            Err("invalid github info format, 'owner/repo' required".to_string())
        } else {
            Ok(Self::new(v[0].to_string(), v[1].to_string()))
        }
    }

    pub fn parse_from_url(url: Url) -> Result<Self, Url> {
        log::debug!("parse github info from url: {url:?}");
        let host = url.host().ok_or_else(|| url.clone())?;
        if host != url::Host::Domain("github.com") {
            return Err(url);
        }
        let captures = GITHUB_PATH_RE
            .captures(url.path())
            .ok_or_else(|| url.clone())?;
        let owner = captures
            .get(1)
            .ok_or_else(|| url.clone())?
            .as_str()
            .to_string();
        let repo = captures
            .get(2)
            .ok_or_else(|| url.clone())?
            .as_str()
            .to_string();
        Ok(Self::new(owner, repo))
    }
}

pub async fn is_merged(info: &GitHubInfo, pr_id: u64) -> Result<bool, Error> {
    Ok(octocrab::instance()
        .pulls(&info.owner, &info.repo)
        .is_merged(pr_id)
        .await
        .map_err(Box::new)?)
}

pub async fn get_pr(info: &GitHubInfo, pr_id: u64) -> Result<PullRequest, Error> {
    Ok(octocrab::instance()
        .pulls(&info.owner, &info.repo)
        .get(pr_id)
        .await
        .map_err(Box::new)?)
}
