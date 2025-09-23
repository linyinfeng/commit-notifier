use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use teloxide::{types::User, utils::markdown};
use url::Url;

use crate::error::Error;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatRepoSettings {
    #[serde(default, alias = "pull_requests")]
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
    pub fn subscribers_markdown(&self) -> String {
        let mut result = String::new();
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
#[serde(try_from = "SubscriberCompat")]
pub enum Subscriber {
    Telegram { markdown_mention: String },
}

impl TryFrom<SubscriberCompat> for Subscriber {
    type Error = Error;

    fn try_from(compat: SubscriberCompat) -> Result<Self, Self::Error> {
        match &compat {
            SubscriberCompat::Telegram {
                markdown_mention,
                username,
            } => match (markdown_mention, username) {
                (Some(mention), _) => Ok(Subscriber::Telegram {
                    markdown_mention: mention.clone(),
                }),
                (_, Some(username)) => Ok(Subscriber::Telegram {
                    markdown_mention: format!("@{username}"),
                }),
                (_, _) => Err(Error::InvalidSubscriber(compat)),
            },
        }
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Clone, Deserialize)]
pub enum SubscriberCompat {
    Telegram {
        markdown_mention: Option<String>,
        username: Option<String>, // field for compatibility
    },
}

impl Subscriber {
    pub fn from_tg_user(u: &User) -> Self {
        Self::Telegram {
            markdown_mention: markdown::user_mention_or_link(u),
        }
    }

    pub fn markdown(&self) -> &str {
        match self {
            Subscriber::Telegram { markdown_mention } => markdown_mention,
        }
    }
}
