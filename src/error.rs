use std::ffi::OsString;

use teloxide::prelude::*;
use thiserror::Error;

use crate::github::GitHubInfo;

#[derive(Error, Debug)]
pub enum Error {
    #[error("unclosed quote")]
    UnclosedQuote,
    #[error("bad escape")]
    BadEscape,
    #[error("{0}")]
    Clap(#[from] clap::Error),
    #[error("repository '{0}' already exists")]
    RepoExists(String),
    #[error("another task is running on repository '{0}', please wait")]
    AnotherTaskRunning(String),
    #[error("cache database error: {0}")]
    DB(#[from] rusqlite::Error),
    #[error("task join error: {0}")]
    TaskJoin(#[from] tokio::task::JoinError),
    #[error("invalid name: {0}")]
    Name(String),
    #[error("chat id {0} is not in allow list")]
    NotInAllowList(ChatId),
    #[error("git error: {0}")]
    Git(#[from] git2::Error),
    #[error("failed to clone git repository '{url}' into '{name}', output: {output:?}")]
    GitClone {
        url: String,
        name: String,
        output: std::process::Output,
    },
    #[error("failed to fetch git repository ''{name}', output: {output:?}")]
    GitFetch {
        name: String,
        output: std::process::Output,
    },
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("unknown commit: '{0}'")]
    UnknownCommit(String),
    #[error("unknown pull request: '{0}'")]
    UnknownPullRequest(u64),
    #[error("unknown branch: '{0}'")]
    UnknownBranch(String),
    #[error("unknown repository: '{0}'")]
    UnknownRepository(String),
    #[error("commit already exists: '{0}'")]
    CommitExists(String),
    #[error("pull request already exists: '{0}'")]
    PullRequestExists(u64),
    #[error("branch already exists: '{0}'")]
    BranchExists(String),
    #[error("invalid os string: '{0:?}'")]
    InvalidOsString(OsString),
    #[error("invalid chat directory: '{0}'")]
    InvalidChatDir(String),
    #[error("parse error: '{0}'")]
    ParseInt(#[from] std::num::ParseIntError),
    #[error("invalid regex: {0}")]
    Regex(#[from] regex::Error),
    #[error("internal error: invalid try lock")]
    TryLock,
    #[error("condition identifier already exists: '{0}'")]
    ConditionExists(String),
    #[error("unknown condition identifier: '{0}'")]
    UnknownCondition(String),
    #[error("github api error: '{0}'")]
    Octocrab(#[from] octocrab::Error),
    #[error("no merge commit: '{github_info}#{pr_id}'")]
    NoMergeCommit { github_info: GitHubInfo, pr_id: u64 },
    #[error("no associated github info for repo: '{0}'")]
    NoGitHubInfo(String),
    #[error("url parse error: '{0}'")]
    UrlParse(#[from] url::ParseError),
}

impl Error {
    pub async fn report(&self, bot: &Bot, msg: &Message) -> Result<(), teloxide::RequestError> {
        log::warn!("report error to chat {}: {:?}", msg.chat.id, self);
        bot.send_message(msg.chat.id, format!("{self}"))
            .reply_to_message_id(msg.id)
            .await?;
        Ok(())
    }
}
