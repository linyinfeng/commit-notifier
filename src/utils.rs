use std::collections::BTreeSet;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter};
use std::path::Path;
use std::sync::LazyLock;

use regex::{Captures, Regex};
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

static TG_USER_ID_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"tg://user/\?id=(\d+)").unwrap());

/// Extract the user id from a `tg://user/?id=...` markdown link, if present.
pub fn tg_user_link_id(s: &str) -> Option<u64> {
    TG_USER_ID_REGEX
        .captures(s)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
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
    file.lock_shared()?; // close of file automatically release the lock
    let reader = BufReader::new(file);
    Ok(serde_json::from_reader(reader)?)
}

pub fn read_json_strict<P, T>(path: P) -> Result<T, Error>
where
    P: AsRef<Path> + fmt::Debug,
    T: Serialize + DeserializeOwned,
{
    log::debug!("read from file: {path:?}");
    let file = File::open(path)?;
    file.lock_shared()?;
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
    file.lock()?;
    let writer: BufWriter<File> = BufWriter::new(file);
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

    let captures = GITHUB_URL_REGEX.captures(&repo_or_url);
    match (captures, pr_id) {
        (Some(captures), explicit_id) => {
            log::trace!("PR/issue id to parse: {}", &captures[4]);
            let id = match explicit_id {
                Some(id) => id,
                None => captures[4].parse().map_err(Error::ParseInt)?,
            };
            Ok((repo_of_github_url(&captures).await?, id))
        }
        // the id is given explicitly, so the first argument must be a plain
        // repository name: an URL here would become a chat repository name
        (None, Some(id)) => {
            repo::name::validate(&repo_or_url)?;
            Ok((repo_or_url, id))
        }
        (None, None) => Err(Error::UnsupportedPRIssueUrl(repo_or_url)),
    }
}

async fn repo_of_github_url(captures: &Captures<'_>) -> Result<String, Error> {
    let github_info = GitHubInfo::new(captures[1].to_owned(), captures[2].to_owned());
    let mut repos_found = Vec::new();
    for candidate in repo::list().await? {
        let resources = repo::resources(&candidate).await?;
        let repo_github_info = &resources.settings.read().await.github_info;
        if repo_github_info.as_ref() == Some(&github_info) {
            repos_found.push(candidate);
        }
    }
    match repos_found.len() {
        0 => Err(Error::NoRepoHaveGitHubInfo(github_info)),
        1 => Ok(repos_found.pop().expect("length checked above")),
        _ => Err(Error::MultipleReposHaveSameGitHubInfo(repos_found)),
    }
}
