use std::ffi::OsString;

use teloxide::prelude::*;
use teloxide::types::ReplyParameters;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::github::GitHubInfo;

#[derive(Error, Debug)]
pub enum Error {
    #[error("unknown resource: {0}")]
    UnknownResource(String),
    #[error("unclosed quote")]
    UnclosedQuote,
    #[error("bad escape")]
    BadEscape,
    #[error("{0}")]
    Clap(#[from] clap::Error),
    #[error("repository '{0}' already exists")]
    RepoExists(String),
    #[error("create db connection pool error: {0}")]
    CreatePool(#[from] deadpool_sqlite::CreatePoolError),
    #[error("db connection pool error: {0}")]
    Pool(#[from] deadpool_sqlite::PoolError),
    #[error("db error: {0}")]
    DB(#[from] rusqlite::Error),
    // `InteractError` is not `Sync`
    // wrap it with `Mutex`
    #[error("db interact error: {0:?}")]
    DBInteract(Mutex<deadpool_sqlite::InteractError>),
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
    #[error("unknown PR/issue: '{0}'")]
    UnknownPRIssue(u64),
    #[error("unknown branch: '{0}'")]
    UnknownBranch(String),
    #[error("unknown repository: '{0}'")]
    UnknownRepository(String),
    #[error("commit already exists: '{0}'")]
    CommitExists(String),
    #[error("PR/issue already exists: '{0}'")]
    PRIssueExists(u64),
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
    #[error("condition identifier already exists: '{0}'")]
    ConditionExists(String),
    #[error("unknown condition identifier: '{0}'")]
    UnknownCondition(String),
    #[error("github api error: '{0}'")]
    Octocrab(#[from] Box<octocrab::Error>),
    #[error("no merge commit: '{github_info}#{pr_id}'")]
    NoMergeCommit { github_info: GitHubInfo, pr_id: u64 },
    #[error("no associated github info for repo: '{0}'")]
    NoGitHubInfo(String),
    #[error("url parse error: '{0}'")]
    UrlParse(#[from] url::ParseError),
    #[error("can not get subscriber from message")]
    NoSubscriber,
    #[error("already subscribed")]
    AlreadySubscribed,
    #[error("not subscribed")]
    NotSubscribed,
    #[error("subscribe term serialize size exceeded: length = {0}, string = {1}")]
    SubscribeTermSizeExceeded(usize, String),
    #[error("can not determine chat id from subscribe callback query")]
    SubscribeCallbackNoChatId,
    #[error("can not determine message id from subscribe callback query")]
    SubscribeCallbackNoMsgId,
    #[error("can not determine username from subscribe callback query")]
    SubscribeCallbackNoUsername,
    #[error("can not get data from subscribe callback query")]
    SubscribeCallbackNoData,
    #[error("invalid kind '{0}' in subscribe callback data")]
    SubscribeCallbackDataInvalidKind(String),
    #[error("ambiguous, multiple repos have same github info: {0:?}")]
    MultipleReposHaveSameGitHubInfo(Vec<String>),
    #[error("no repository is associated with the github info: {0:?}")]
    NoRepoHaveGitHubInfo(GitHubInfo),
    #[error("unsupported PR/issue url: {0}")]
    UnsupportedPRIssueUrl(String),
    #[error("not in an admin chat")]
    NotAdminChat,
}

impl Error {
    pub async fn report(&self, bot: &Bot, msg: &Message) -> Result<(), teloxide::RequestError> {
        log::warn!("report error to chat {}: {:?}", msg.chat.id, self);
        bot.send_message(msg.chat.id, format!("{self}"))
            .reply_parameters(ReplyParameters::new(msg.id))
            .await?;
        Ok(())
    }
}
