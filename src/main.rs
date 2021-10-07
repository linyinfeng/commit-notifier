mod cache;
mod error;
mod options;
mod repo;

use std::fmt;
use std::str::FromStr;

use chrono::Utc;
use cron::Schedule;
use regex::Regex;
use repo::TaskGuard;
use teloxide::prelude::*;
use teloxide::types::Me;
use teloxide::types::ParseMode;
use teloxide::utils::command::BotCommand;
use teloxide::utils::command::ParseError;
use teloxide::utils::markdown;
use tokio::time::sleep;

#[derive(BotCommand)]
#[command(rename = "lowercase", description = "Supported commands:")]
enum Command {
    #[command(description = "display help text.")]
    Help,
    #[command(description = "add a repository.", parse_with = "split")]
    RepoAdd { name: String, url: String },
    #[command(description = "remove a repository.")]
    RepoRemove(String),
    #[command(description = "add a commit.", parse_with = "parse_commit_add")]
    CommitAdd {
        name: String,
        hash: String,
        comment: String,
    },
    #[command(description = "remove a commit.", parse_with = "split")]
    CommitRemove { name: String, hash: String },
    #[command(description = "fire a check immediately.", parse_with = "split")]
    Check { name: String, hash: String },
    #[command(description = "list repositories and commits.")]
    List,
}

async fn answer(
    cx: UpdateWithCx<AutoSend<Bot>, Message>,
    command: Command,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match command {
        Command::Help => {
            cx.reply_to(Command::descriptions()).await?;
        }
        Command::List => list(cx).await?,
        Command::RepoAdd { name, url } => repo_add(cx, name, url).await?,
        Command::RepoRemove(name) => repo_remove(cx, name).await?,
        Command::CommitAdd {
            name,
            hash,
            comment,
        } => commit_add(cx, name, hash, comment).await?,
        Command::CommitRemove { name, hash } => commit_remove(cx, name, hash).await?,
        Command::Check { name, hash } => check(cx, name, hash).await?,
    };

    Ok(())
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
    let chats = match repo::get_chats() {
        Err(e) => {
            log::error!("failed to get chats: {}", e);
            return Ok(());
        }
        Ok(cs) => cs,
    };

    for chat in chats {
        let repos = match repo::get_repos(chat) {
            Err(e) => {
                log::error!("failed to get repos for chat {}: {}", chat, e);
                continue;
            }
            Ok(rs) => rs,
        };
        for repo in repos {
            log::info!("update ({}, {})", chat, repo);

            let lock = match repo::lock_task(repo::Task {
                chat,
                name: repo.to_owned(),
            }) {
                Some(l) => l,
                None => {
                    log::info!("another task running on ({}, {}), skip", chat, repo);
                    continue;
                }
            };

            if let Err(e) = repo::fetch(&lock).await {
                log::warn!("failed to fetch ({}, {}), skip: {}", chat, repo, e);
                continue;
            }

            let commits = match repo::get_commits(chat, &repo) {
                Err(e) => {
                    log::error!("failed to get commits for ({}, {}): {}", chat, repo, e);
                    continue;
                }
                Ok(cs) => cs,
            };

            for (commit, info) in commits {
                let result = match repo::check(&lock, &commit).await {
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

async fn list(cx: UpdateWithCx<AutoSend<Bot>, Message>) -> Result<(), teloxide::RequestError> {
    let chat = cx.chat_id();

    let mut result = String::new();

    let repos = match repo::get_repos(chat) {
        Err(e) => {
            e.report(&cx).await?;
            return Ok(());
        }
        Ok(rs) => rs,
    };
    for repo in repos {
        result.push_str(&repo);
        result.push('\n');

        let commits = match repo::get_commits(chat, &repo) {
            Err(e) => {
                e.report(&cx).await?;
                return Ok(());
            }
            Ok(cs) => cs,
        };
        if commits.is_empty() {
            result.push_str("(nothing)\n");
        }
        for (commit, info) in commits {
            result.push_str(&format!("- {}\n   {}\n", commit, info.comment));
        }

        result.push('\n');
    }
    cx.reply_to(result).await?;

    Ok(())
}

async fn repo_add(
    cx: UpdateWithCx<AutoSend<Bot>, Message>,
    name: String,
    url: String,
) -> Result<(), teloxide::RequestError> {
    let chat = cx.chat_id();
    let exists = match repo::exists(chat, &name) {
        Ok(e) => e,
        Err(e) => {
            e.report(&cx).await?;
            return Ok(());
        }
    };

    if exists {
        cx.reply_to(format!("repository '{}' already exists", name))
            .await?;
        return Ok(());
    }

    let lock = match prepare_lock(&cx, &name).await? {
        Some(l) => l,
        None => return Ok(()),
    };
    cx.reply_to(format!("clone into '{}'", name)).await?;
    match repo::create(&lock, &url).await {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            cx.reply_to(format!(
                "repository '{}' added\nstdout: {}\nstderr: {}",
                name, stdout, stderr
            ))
            .await?;
        }
        Err(e) => e.report(&cx).await?,
    }

    Ok(())
}

async fn repo_remove(
    cx: UpdateWithCx<AutoSend<Bot>, Message>,
    name: String,
) -> Result<(), teloxide::RequestError> {
    let chat = cx.chat_id();
    let exists = match repo::exists(chat, &name) {
        Ok(e) => e,
        Err(e) => {
            e.report(&cx).await?;
            return Ok(());
        }
    };

    if !exists {
        cx.reply_to(format!("repository '{}' does not exists", name))
            .await?;
        return Ok(());
    }

    let lock = match prepare_lock(&cx, &name).await? {
        Some(l) => l,
        None => return Ok(()),
    };

    match repo::remove(&lock).await {
        Err(e) => e.report(&cx).await?,
        Ok(()) => {
            cx.reply_to(format!("repository '{}' removed", name))
                .await?;
        }
    }

    Ok(())
}

async fn commit_add(
    cx: UpdateWithCx<AutoSend<Bot>, Message>,
    name: String,
    hash: String,
    comment: String,
) -> Result<(), teloxide::RequestError> {
    let lock = match prepare_lock(&cx, &name).await? {
        Some(l) => l,
        None => return Ok(()),
    };

    let info = repo::CommitInfo { comment };

    if let Err(e) = repo::commit_add(&lock, &hash, info).await {
        e.report(&cx).await?;
        return Ok(());
    }

    cx.reply_to(format!("commit {} added", hash)).await?;

    drop(lock);

    check(cx, name, hash).await
}

async fn commit_remove(
    cx: UpdateWithCx<AutoSend<Bot>, Message>,
    name: String,
    hash: String,
) -> Result<(), teloxide::RequestError> {
    let lock = match prepare_lock(&cx, &name).await? {
        Some(l) => l,
        None => return Ok(()),
    };

    if let Err(e) = repo::commit_remove(&lock, &hash).await {
        e.report(&cx).await?;
        return Ok(());
    }

    cx.reply_to(format!("commit {} removed", hash)).await?;

    Ok(())
}

async fn check(
    cx: UpdateWithCx<AutoSend<Bot>, Message>,
    name: String,
    hash: String,
) -> Result<(), teloxide::RequestError> {
    let lock = match prepare_lock(&cx, &name).await? {
        Some(l) => l,
        None => return Ok(()),
    };

    if let Err(e) = repo::fetch(&lock).await {
        e.report(&cx).await?;
        return Ok(());
    };

    let info = match repo::commit_info(&lock, &hash).await {
        Ok(i) => i,
        Err(e) => {
            e.report(&cx).await?;
            return Ok(());
        }
    };

    let result = match repo::check(&lock, &hash).await {
        Ok(rs) => rs,
        Err(e) => {
            e.report(&cx).await?;
            return Ok(());
        }
    };

    let reply = check_message(&name, &hash, &info, &result);
    cx.reply_to(reply).parse_mode(ParseMode::MarkdownV2).await?;

    Ok(())
}

async fn prepare_lock(
    cx: &UpdateWithCx<AutoSend<Bot>, Message>,
    name: &str,
) -> Result<Option<TaskGuard>, teloxide::RequestError> {
    let chat = cx.chat_id();
    match repo::lock_task(repo::Task {
        chat,
        name: name.to_owned(),
    }) {
        Some(lock) => Ok(Some(lock)),
        None => {
            log::info!("ignored command from {} on '{}'", chat, name);
            cx.reply_to(format!("another task on '{}' is running", name))
                .await?;
            Ok(None)
        }
    }
}

fn check_message(
    repo: &str,
    commit: &str,
    info: &repo::CommitInfo,
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
        comment = markdown::escape(&info.comment),
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

static COMMIT_ADD_RE: once_cell::sync::Lazy<Regex> =
    once_cell::sync::Lazy::new(|| Regex::new("([a-zA-Z0-9_\\-]*) ([a-z0-9]*) (.*)").unwrap());

fn parse_commit_add(input: String) -> Result<(String, String, String), ParseError> {
    log::info!("parse raw input: {}", input);
    let error = Err(ParseError::IncorrectFormat(
        error::Error::WrongCommandInput(input.clone()).into(),
    ));
    let mut lines: Vec<_> = input.lines().collect();
    if lines.is_empty() {
        return error;
    }
    let captures = match COMMIT_ADD_RE.captures(lines[0]) {
        Some(cap) => cap,
        None => return error,
    };

    let get = |n| captures.get(n).unwrap().as_str();
    lines[0] = get(3);
    let comment: String = lines.into_iter().map(|l| format!("{}\n", l)).collect();
    let comment_trimmed = comment.trim().to_owned();
    let result = (get(1).to_owned(), get(2).to_owned(), comment_trimmed);

    log::info!("parse success: {:?}", result);

    Ok(result)
}
