use std::collections::BTreeSet;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter};
use std::path::Path;
use std::sync::LazyLock;

use fs4::fs_std::FileExt;
use regex::Regex;
use serde::Serialize;
use serde::de::DeserializeOwned;
use teloxide::types::ReplyParameters;
use teloxide::{payloads::SendMessage, prelude::*, requests::JsonRequest};

use crate::chat::settings::Subscriber;
use crate::error::Error;
use crate::github::GitHubInfo;
use crate::{CommandError, repo};

pub async fn report_error<T>(
    bot: &Bot,
    msg: &Message,
    result: Result<T, Error>,
) -> Result<Option<T>, teloxide::RequestError> {
    match result {
        Ok(r) => Ok(Some(r)),
        Err(e) => {
            // report normal errors to user
            e.report(bot, msg).await?;
            Ok(None)
        }
    }
}

pub async fn report_command_error<T>(
    bot: &Bot,
    msg: &Message,
    result: Result<T, CommandError>,
) -> Result<Option<T>, teloxide::RequestError> {
    match result {
        Ok(r) => Ok(Some(r)),
        Err(CommandError::Normal(e)) => {
            // report normal errors to user
            e.report(bot, msg).await?;
            Ok(None)
        }
        Err(CommandError::Teloxide(e)) => Err(e),
    }
}

pub fn reply_to_msg<T>(bot: &Bot, msg: &Message, text: T) -> JsonRequest<SendMessage>
where
    T: Into<String>,
{
    bot.send_message(msg.chat.id, text)
        .reply_parameters(ReplyParameters::new(msg.id))
}

pub fn empty_or_start_new_line(s: &str) -> String {
    let trimmed = s.trim().to_string();
    if trimmed.is_empty() {
        trimmed
    } else {
        let mut result = "\n".to_string();
        result.push_str(&trimmed);
        result
    }
}

pub fn read_json<P, T>(path: P) -> Result<T, Error>
where
    P: AsRef<Path> + fmt::Debug,
    T: Serialize + DeserializeOwned + Default,
{
    if !path.as_ref().is_file() {
        log::info!("auto create file: {path:?}");
        write_json::<_, T>(&path, &Default::default())?;
    }
    log::debug!("read from file: {path:?}");
    let file = File::open(path)?;
    // TODO lock_shared maybe added to the std lib in the future
    FileExt::lock_shared(&file)?; // close of file automatically release the lock
    let reader = BufReader::new(file);
    Ok(serde_json::from_reader(reader)?)
}

pub fn write_json<P, T>(path: P, rs: &T) -> Result<(), Error>
where
    P: AsRef<Path> + fmt::Debug,
    T: Serialize,
{
    log::debug!("write to file: {path:?}");
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    file.lock_exclusive()?;
    let writer = BufWriter::new(file);
    Ok(serde_json::to_writer_pretty(writer, rs)?)
}

pub fn modify_subscriber_set(
    set: &mut BTreeSet<Subscriber>,
    subscriber: Subscriber,
    unsubscribe: bool,
) -> Result<(), Error> {
    if unsubscribe {
        if !set.contains(&subscriber) {
            return Err(Error::NotSubscribed);
        }
        set.remove(&subscriber);
    } else {
        if set.contains(&subscriber) {
            return Err(Error::AlreadySubscribed);
        }
        set.insert(subscriber);
    }
    Ok(())
}

pub async fn resolve_repo_or_url_and_id(
    repo_or_url: String,
    pr_id: Option<u64>,
) -> Result<(String, u64), Error> {
    static GITHUB_URL_REGEX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"https://github\.com/([^/]+)/([^/]+)/(pull|issues)/(\d+)"#).unwrap()
    });
    match pr_id {
        Some(id) => Ok((repo_or_url, id)),
        None => {
            if let Some(captures) = GITHUB_URL_REGEX.captures(&repo_or_url) {
                let owner = &captures[1];
                let repo = &captures[2];
                let github_info = GitHubInfo::new(owner.to_string(), repo.to_string());
                log::trace!("PR/issue id to parse: {}", &captures[4]);
                let id: u64 = captures[4].parse().map_err(Error::ParseInt)?;
                let repos = repo::list().await?;
                let mut repos_found = Vec::new();
                for repo in repos {
                    let resources = repo::resources(&repo).await?;
                    let repo_github_info = &resources.settings.read().await.github_info;
                    if repo_github_info.as_ref() == Some(&github_info) {
                        repos_found.push(repo);
                    }
                }
                if repos_found.is_empty() {
                    return Err(Error::NoRepoHaveGitHubInfo(github_info));
                } else if repos_found.len() != 1 {
                    return Err(Error::MultipleReposHaveSameGitHubInfo(repos_found));
                } else {
                    let repo = repos_found.pop().unwrap();
                    return Ok((repo, id));
                }
            }
            Err(Error::UnsupportedPRIssueUrl(repo_or_url))
        }
    }
}
