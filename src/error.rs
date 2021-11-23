use std::ffi::OsString;

use teloxide::prelude::*;
use thiserror::Error;

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
    NotInAllowList(i64),
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
    #[error("unknown repository: '{0}'")]
    UnknownRepository(String),
    #[error("commit already exists: '{0}'")]
    CommitExists(String),
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
}

impl Error {
    pub async fn report(
        &self,
        cx: &UpdateWithCx<AutoSend<Bot>, Message>,
    ) -> Result<(), teloxide::RequestError> {
        log::warn!("report error to chat {}: {:?}", cx.chat_id(), self);
        cx.reply_to(format!("{}", self)).await?;

        Ok(())
    }
}
