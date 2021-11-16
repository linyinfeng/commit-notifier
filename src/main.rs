mod cache;
mod command;
mod error;
mod options;
mod repo;

use std::fmt;
use std::str::FromStr;

use chrono::Utc;
use cron::Schedule;
use regex::Regex;
use repo::tasks::Task;
use repo::tasks::TaskGuard;
use repo::tasks::TaskGuardBare;
use teloxide::prelude::*;
use teloxide::types::Me;
use teloxide::types::ParseMode;
use teloxide::utils::command::BotCommand;
use teloxide::utils::markdown;
use tokio::time::sleep;

use crate::repo::settings::CommitSettings;

#[derive(BotCommand)]
#[command(rename = "lowercase", description = "Supported commands:")]
enum BCommand {
    #[command(description = "main and the only command.")]
    Notifier(String),
}

async fn answer(
    cx: UpdateWithCx<AutoSend<Bot>, Message>,
    bc: BCommand,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let BCommand::Notifier(input) = bc;
    let result = match command::parse(input) {
        Ok(command) => match command {
            command::Notifier::RepoAdd { name, url } => repo_add(&cx, name, url).await,
            command::Notifier::RepoEdit { name, branch_regex } => {
                repo_edit(&cx, name, branch_regex).await
            }
            command::Notifier::RepoRemove { name } => repo_remove(&cx, name).await,
            command::Notifier::CommitAdd {
                repo,
                hash,
                comment,
            } => commit_add(&cx, repo, hash, comment).await,
            command::Notifier::CommitRemove { repo, hash } => commit_remove(&cx, repo, hash).await,
            command::Notifier::Check { repo, hash } => check(&cx, repo, hash).await,
            command::Notifier::List => list(&cx).await,
        },
        Err(e) => Err(e.into()),
    };
    match result {
        Ok(()) => Ok(()),
        Err(CommandError::Normal(e)) => {
            // report normal errors to user
            e.report(&cx).await?;
            Ok(())
        }
        Err(CommandError::Teloxide(e)) => Err(Box::new(e)),
    }
}

enum CommandError {
    Normal(error::Error),
    Teloxide(teloxide::RequestError),
}
impl From<error::Error> for CommandError {
    fn from(e: error::Error) -> Self {
        CommandError::Normal(e)
    }
}
impl From<teloxide::RequestError> for CommandError {
    fn from(e: teloxide::RequestError) -> Self {
        CommandError::Teloxide(e)
    }
}

#[tokio::main]
async fn main() {
    run().await;
}

async fn run() {
    pretty_env_logger::init();

    options::initialize();
    log::info!("config = {:?}", options::get());

    let bot = Bot::from_env().auto_send();
    let Me { user: bot_user, .. } = bot.get_me().await.unwrap();
    let bot_name = bot_user.username.expect("bots must have usernames");

    tokio::select! {
        _ = schedule(bot.clone()) => { },
        _ = teloxide::commands_repl(bot, bot_name, answer) => { },
    }
}

async fn schedule(bot: AutoSend<Bot>) {
    let expression = &options::get().cron;
    let schedule = Schedule::from_str(expression).expect("cron expression");
    for datetime in schedule.upcoming(Utc) {
        let now = Utc::now();
        let dur = match (datetime - now).to_std() {
            // duration is less than zero
            Err(_) => continue,
            Ok(std_dur) => std_dur,
        };
        log::info!(
            "update is going to be triggered at '{}', sleep '{:?}'",
            datetime,
            dur
        );
        sleep(dur).await;
        log::info!("perform update '{}'", datetime);
        if let Err(e) = update(bot.clone()).await {
            log::error!("teloxide error in update: {}", e);
        }
        log::info!("finished update '{}'", datetime);
    }
}

async fn update(bot: AutoSend<Bot>) -> Result<(), teloxide::RequestError> {
    let chats = match repo::paths::chats() {
        Err(e) => {
            log::error!("failed to get chats: {}", e);
            return Ok(());
        }
        Ok(cs) => cs,
    };

    for chat in chats {
        let repos = match repo::paths::repos(chat) {
            Err(e) => {
                log::error!("failed to get repos for chat {}: {}", chat, e);
                continue;
            }
            Ok(rs) => rs,
        };
        for repo in repos {
            log::info!("update ({}, {})", chat, repo);

            let task = repo::tasks::Task {
                chat,
                repo: repo.to_owned(),
            };
            let lock = match task.lock() {
                Ok(Some(l)) => l,
                Ok(None) => {
                    log::info!("another task running on ({}, {}), skip", chat, repo);
                    continue;
                }
                Err(e) => {
                    log::error!("failed to acquire task guard in update: {}", e);
                    continue;
                }
            };

            if let Err(e) = repo::fetch(lock.clone()).await {
                log::warn!("failed to fetch ({}, {}), skip: {}", chat, repo, e);
                continue;
            }

            let commits = {
                let resources = lock.resources.lock().unwrap();
                resources.settings.commits.clone()
            };
            for (commit, info) in commits {
                let result = match repo::check(lock.clone(), &commit).await {
                    Err(e) => {
                        log::warn!("failed to check ({}, {}, {}): {}", chat, repo, commit, e);
                        continue;
                    }
                    Ok(r) => r,
                };

                log::info!("check ({}, {}, {}) finished", chat, repo, commit);
                if !result.new.is_empty() {
                    let message = check_message(&repo, &commit, &info, &result);
                    bot.send_message(chat, message)
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;
                }
            }
        }
    }

    Ok(())
}

async fn list(cx: &UpdateWithCx<AutoSend<Bot>, Message>) -> Result<(), CommandError> {
    let chat = cx.chat_id();
    let mut result = String::new();

    let repos = repo::paths::repos(chat)?;
    for repo in repos {
        result.push_str(&repo);
        result.push('\n');

        let lock = (Task {
            chat,
            repo: repo.clone(),
        })
        .lock()?
        .ok_or(error::Error::AnotherTaskRunning(repo))?;
        let resources = lock.resources.lock().unwrap();
        let commits = &resources.settings.commits;
        if commits.is_empty() {
            result.push_str("(nothing)\n");
        }
        for (commit, settings) in commits {
            result.push_str(&format!("- {}\n   {}\n", commit, settings.comment));
        }

        result.push('\n');
    }
    if result.is_empty() {
        result.push_str("(nothing)\n");
    }
    cx.reply_to(result).await?;

    Ok(())
}

async fn repo_add(
    cx: &UpdateWithCx<AutoSend<Bot>, Message>,
    name: String,
    url: String,
) -> Result<(), CommandError> {
    let lock = prepare_lock_bare(cx, &name)?;
    let chat = cx.chat_id();
    if repo::exists(chat, &name)? {
        return Err(error::Error::RepoExists(name).into());
    }
    cx.reply_to(format!("start clone into '{}'", name)).await?;
    let output = repo::create(lock, &url).await?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    cx.reply_to(format!(
        "repository '{}' added\nstdout:\n{}\nstderr:\n{}",
        name, stdout, stderr
    ))
    .await?;
    Ok(())
}

async fn repo_edit(
    cx: &UpdateWithCx<AutoSend<Bot>, Message>,
    name: String,
    branch_regex: Option<String>,
) -> Result<(), CommandError> {
    let lock = prepare_lock(cx, &name)?;
    let current_settings = {
        let mut resources = lock.resources.lock().unwrap();
        if let Some(r) = branch_regex {
            // ensure regex is valid
            let _: Regex = Regex::new(&r).map_err(error::Error::from)?;
            resources.settings.branch_regex = r;
        }
        resources.settings.clone()
    };
    lock.save_resources()?;
    cx.reply_to(format!(
        "repository '{}' edited, current settings:\n{:#?}",
        name, current_settings
    ))
    .await?;
    Ok(())
}

async fn repo_remove(
    cx: &UpdateWithCx<AutoSend<Bot>, Message>,
    name: String,
) -> Result<(), CommandError> {
    let lock = prepare_lock_bare(cx, &name)?;
    let chat = cx.chat_id();
    if !repo::exists(chat, &name)? {
        return Err(error::Error::UnknownRepository(name).into());
    }
    repo::remove(lock).await?;
    cx.reply_to(format!("repository '{}' removed", name))
        .await?;
    Ok(())
}

async fn commit_add(
    cx: &UpdateWithCx<AutoSend<Bot>, Message>,
    repo: String,
    hash: String,
    comment: String,
) -> Result<(), CommandError> {
    let lock = prepare_lock(cx, &repo)?;
    let info = CommitSettings { comment };
    repo::commit_add(lock, &hash, info).await?;
    cx.reply_to(format!("commit {} added", hash)).await?;
    check(cx, repo, hash).await
}

async fn commit_remove(
    cx: &UpdateWithCx<AutoSend<Bot>, Message>,
    repo: String,
    hash: String,
) -> Result<(), CommandError> {
    let lock = prepare_lock(cx, &repo)?;
    repo::commit_remove(lock, &hash).await?;
    cx.reply_to(format!("commit {} removed", hash)).await?;
    Ok(())
}

async fn check(
    cx: &UpdateWithCx<AutoSend<Bot>, Message>,
    repo: String,
    hash: String,
) -> Result<(), CommandError> {
    let lock = prepare_lock(cx, &repo)?;
    repo::fetch(lock.clone()).await?;
    let commit_settings = {
        let resources = lock.resources.lock().unwrap();
        resources
            .settings
            .commits
            .get(&hash)
            .ok_or_else(|| error::Error::UnknownCommit(hash.clone()))?
            .clone()
    };
    let result = repo::check(lock, &hash).await?;
    let reply = check_message(&repo, &hash, &commit_settings, &result);
    cx.reply_to(reply).parse_mode(ParseMode::MarkdownV2).await?;

    Ok(())
}

fn prepare_lock(
    cx: &UpdateWithCx<AutoSend<Bot>, Message>,
    repo: &str,
) -> Result<TaskGuard, error::Error> {
    let chat = cx.chat_id();
    let task = repo::tasks::Task {
        chat,
        repo: repo.to_owned(),
    };
    match task.lock() {
        Ok(Some(lock)) => Ok(lock),
        Ok(None) => {
            log::info!("ignored command from {} on '{}'", chat, repo);
            Err(error::Error::AnotherTaskRunning(repo.to_owned()))
        }
        Err(e) => {
            log::error!("failed to acquire task guard in update: {}", e);
            Err(e)
        }
    }
}

fn prepare_lock_bare(
    cx: &UpdateWithCx<AutoSend<Bot>, Message>,
    repo: &str,
) -> Result<TaskGuardBare, error::Error> {
    let chat = cx.chat_id();
    let task = repo::tasks::Task {
        chat,
        repo: repo.to_owned(),
    };
    match task.lock_bare() {
        Some(lock) => Ok(lock),
        None => {
            log::info!("ignored command from {} on '{}'", chat, repo);
            Err(error::Error::AnotherTaskRunning(repo.to_owned()))
        }
    }
}

fn check_message(
    repo: &str,
    commit: &str,
    settings: &CommitSettings,
    result: &repo::CheckResult,
) -> String {
    format!(
        "
{repo}/`{commit}`

*comment*:
{comment}

*new* branches containing this commit:
{new}

*all* branches containing this commit:
{all}
",
        repo = markdown::escape(repo),
        commit = markdown::escape(commit),
        comment = markdown::escape(&settings.comment),
        new = markdown_list(result.new.iter()),
        all = markdown_list(result.all.iter())
    )
}

fn markdown_list<Iter, T>(s: Iter) -> String
where
    Iter: Iterator<Item = T>,
    T: fmt::Display,
{
    let mut res: String = s
        .map(|t| format!("{}", t))
        .map(|t| format!("\\- {}\n", markdown::escape(&t)))
        .collect();
    if res.is_empty() {
        "\u{2205}".to_owned() // the empty set symbol
    } else {
        assert_eq!(res.pop(), Some('\n'));
        res
    }
}
