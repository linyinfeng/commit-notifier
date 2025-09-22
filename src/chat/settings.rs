use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use teloxide::utils::markdown;
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatRepoSettings {
    #[serde(default, alias="pull_requests")]
    pub pr_issues: BTreeMap<u64, PRIssueSettings>,
    #[serde(default)]
    pub commits: BTreeMap<String, CommitSettings>,
    #[serde(default)]
    pub branches: BTreeMap<String, BranchSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitSettings {
    pub url: Option<Url>,
    #[serde(flatten)]
    pub notify: NotifySettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PRIssueSettings {
    pub url: Url,
    #[serde(flatten)]
    pub notify: NotifySettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BranchSettings {
    #[serde(flatten)]
    pub notify: NotifySettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NotifySettings {
    #[serde(default)]
    pub comment: String,
    #[serde(default)]
    pub subscribers: BTreeSet<Subscriber>,
}

impl NotifySettings {
    pub fn notify_markdown(&self) -> String {
        let mut result = String::new();
        let comment = self.comment.trim();
        if !comment.is_empty() {
            result.push_str("*comment*:\n");
            result.push_str(&markdown::escape(self.comment.trim()));
        }
        if !self.subscribers.is_empty() {
            if !result.is_empty() {
                result.push_str("\n\n");
            }
            result.push_str("*subscribers*: ");
            result.push_str(
                &self
                    .subscribers
                    .iter()
                    .map(Subscriber::markdown)
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        result
    }

    pub fn description_markdown(&self) -> String {
        markdown::escape(self.comment.trim().lines().next().unwrap_or_default())
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Clone, Serialize, Deserialize)]
pub enum Subscriber {
    Telegram { username: String },
}

impl Subscriber {
    fn markdown(&self) -> String {
        match self {
            Subscriber::Telegram { username } => format!("@{}", markdown::escape(username)),
        }
    }
}
