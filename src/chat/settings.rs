use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use teloxide::{types::User, utils::markdown};
use url::Url;

use crate::{error::Error, utils::tg_user_link_id};

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
    pub closed_at: Option<DateTime<Utc>>,
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
    #[serde(default, deserialize_with = "deserialize_subscribers")]
    pub subscribers: BTreeSet<Subscriber>,
}

/// Drop subscribers that can not be converted (legacy entries without a
/// recoverable user id) instead of failing to load the whole settings file.
fn deserialize_subscribers<'de, D>(de: D) -> Result<BTreeSet<Subscriber>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let mut subscribers = BTreeSet::new();
    for compat in Vec::<SubscriberCompat>::deserialize(de)? {
        match Subscriber::try_from(compat) {
            Ok(subscriber) => {
                subscribers.insert(subscriber);
            }
            Err(e) => log::warn!("drop subscriber: {e}"),
        }
    }
    Ok(subscribers)
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
    Telegram {
        user_id: u64,
        markdown_mention: String,
    },
}

impl SubscriberCompat {
    /// Resolve the telegram user id: the stored field, or recovered from a
    /// `tg://user/?id=...` mention link (legacy format).
    pub fn resolve_user_id(&self) -> Option<u64> {
        match self {
            // stored user id takes precedence
            SubscriberCompat::Telegram {
                user_id: Some(id), ..
            } => Some(*id),
            // legacy format: recover the id from the mention link
            SubscriberCompat::Telegram {
                user_id: None,
                markdown_mention: Some(mention),
                ..
            } => tg_user_link_id(mention),
            SubscriberCompat::Telegram {
                user_id: None,
                markdown_mention: None,
                ..
            } => None,
        }
    }
}

impl TryFrom<SubscriberCompat> for Subscriber {
    type Error = Error;

    fn try_from(compat: SubscriberCompat) -> Result<Self, Self::Error> {
        let user_id = compat
            .resolve_user_id()
            .ok_or_else(|| Error::InvalidSubscriber(compat.clone()))?;
        match &compat {
            SubscriberCompat::Telegram {
                markdown_mention,
                username,
                ..
            } => match (markdown_mention, username) {
                (Some(mention), _) => Ok(Subscriber::Telegram {
                    user_id,
                    markdown_mention: mention.clone(),
                }),
                (_, Some(username)) => Ok(Subscriber::Telegram {
                    user_id,
                    markdown_mention: format!("@{}", markdown::escape(username)),
                }),
                (_, _) => Err(Error::InvalidSubscriber(compat)),
            },
        }
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Clone, Deserialize)]
pub enum SubscriberCompat {
    Telegram {
        user_id: Option<u64>,
        markdown_mention: Option<String>,
        username: Option<String>, // field for compatibility
    },
}

impl Subscriber {
    pub fn from_tg_user(user: &User) -> Self {
        Self::Telegram {
            user_id: user.id.0,
            markdown_mention: markdown::link(
                user.url().as_str(),
                &markdown::escape(&user.first_name),
            ),
        }
    }

    pub fn markdown(&self) -> &str {
        match self {
            Subscriber::Telegram {
                markdown_mention, ..
            } => markdown_mention,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscriber_compat() {
        // legacy format without user_id: id recovered from the mention link
        let s: Subscriber =
            serde_json::from_str(r#"{"Telegram": {"markdown_mention": "[A](tg://user/?id=123)"}}"#)
                .unwrap();
        assert_eq!(
            s,
            Subscriber::Telegram {
                user_id: 123,
                markdown_mention: "[A](tg://user/?id=123)".to_string(),
            }
        );
        // new format round-trips
        let s: Subscriber =
            serde_json::from_str(r#"{"Telegram": {"user_id": 123, "markdown_mention": "@alice"}}"#)
                .unwrap();
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            r#"{"Telegram":{"user_id":123,"markdown_mention":"@alice"}}"#
        );
    }

    #[test]
    fn unresolvable_subscriber_is_dropped() {
        // legacy @username entry without user_id can not be resolved: dropped
        // instead of failing the whole file
        let s: NotifySettings = serde_json::from_str(
            r#"{"subscribers": [{"Telegram": {"markdown_mention": "@alice"}}]}"#,
        )
        .unwrap();
        assert!(s.subscribers.is_empty());
    }
}
