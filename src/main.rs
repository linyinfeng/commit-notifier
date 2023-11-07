mod cache;
mod command;
mod condition;
mod error;
mod github;
mod options;
mod repo;
mod utils;

use std::env;
use std::fmt;
use std::str::FromStr;

use chrono::Utc;
use condition::GeneralCondition;
use cron::Schedule;
use error::Error;
use github::GitHubInfo;
use regex::Regex;
use repo::settings::BranchSettings;
use repo::settings::CommitSettings;
use repo::settings::ConditionSettings;
use repo::settings::NotifySettings;
use repo::settings::PullRequestSettings;
use repo::settings::Subscriber;
use repo::tasks::Task;
use repo::tasks::TaskGuard;
use repo::tasks::TaskGuardBare;
use repo::BranchCheckResult;
use teloxide::prelude::*;
use teloxide::types::ParseMode;
use teloxide::utils::command::BotCommands;
use teloxide::utils::markdown;
use tokio::time::sleep;
use url::Url;
use utils::reply_to_msg;

use crate::utils::push_empty_line;

#[derive(BotCommands, Clone, Debug)]
#[command(rename_rule = "lowercase", description = "Supported commands:")]
enum BCommand {
    #[command(description = "main and the only command.")]
    Notifier(String),
}

async fn answer(bot: Bot, msg: Message, bc: BCommand) -> ResponseResult<()> {
    log::debug!("message: {:?}", msg);
    log::trace!("bot command: {:?}", bc);
    let BCommand::Notifier(input) = bc;
    let result = match command::parse(input) {
        Ok(command) => {
            log::debug!("command: {:?}", command);
            let (bot, msg) = (bot.clone(), msg.clone());
            match command {
                command::Notifier::RepoAdd { name, url } => repo_add(bot, msg, name, url).await,
                command::Notifier::RepoEdit {
                    name,
                    branch_regex,
                    github_info,
                    clear_github_info,
                } => repo_edit(bot, msg, name, branch_regex, github_info, clear_github_info).await,
                command::Notifier::RepoRemove { name } => repo_remove(bot, msg, name).await,
                command::Notifier::CommitAdd {
                    repo,
                    hash,
                    comment,
                } => commit_add(bot, msg, repo, hash, comment, None).await,
                command::Notifier::CommitRemove { repo, hash } => {
                    commit_remove(bot, msg, repo, hash).await
                }
                command::Notifier::CommitCheck { repo, hash } => {
                    commit_check(bot, msg, repo, hash).await
                }
                command::Notifier::PrAdd { repo, pr, comment } => {
                    pr_add(bot, msg, repo, pr, comment).await
                }
                command::Notifier::PrRemove { repo, pr } => pr_remove(bot, msg, repo, pr).await,
                command::Notifier::PrCheck { repo, pr } => pr_check(bot, msg, repo, pr).await,
                command::Notifier::BranchAdd { repo, branch } => {
                    branch_add(bot, msg, repo, branch).await
                }
                command::Notifier::BranchRemove { repo, branch } => {
                    branch_remove(bot, msg, repo, branch).await
                }
                command::Notifier::BranchCheck { repo, branch } => {
                    branch_check(bot, msg, repo, branch).await
                }
                command::Notifier::ConditionAdd {
                    repo,
                    identifier,
                    kind,
                    expression,
                } => condition_add(bot, msg, repo, identifier, kind, expression).await,
                command::Notifier::ConditionRemove { repo, identifier } => {
                    condition_remove(bot, msg, repo, identifier).await
                }
                command::Notifier::ConditionTrigger { repo, identifier } => {
                    condition_trigger(bot, msg, repo, identifier).await
                }
                command::Notifier::List => list(bot, msg).await,
            }
        }
        Err(e) => Err(e.into()),
    };
    match result {
        Ok(()) => Ok(()),
        Err(CommandError::Normal(e)) => {
            // report normal errors to user
            e.report(&bot, &msg).await?;
            Ok(())
        }
        Err(CommandError::Teloxide(e)) => Err(e),
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

    octocrab_initialize();

    let bot = Bot::from_env();

    tokio::select! {
        _ = schedule(bot.clone()) => { },
        _ = BCommand::repl(bot, answer) => { },
    }
}

fn octocrab_initialize() {
    let builder = octocrab::Octocrab::builder();
    let with_token = match env::var("GITHUB_TOKEN") {
        Ok(token) => {
            log::info!("github token set using environment variable 'GITHUB_TOKEN'");
            builder.personal_token(token)
        }
        Err(e) => {
            log::info!("github token not set: {e}");
            builder
        }
    };
    let crab = with_token.build().unwrap();
    octocrab::initialise(crab);
}

async fn schedule(bot: Bot) {
    let expression = &options::get().cron;
    let schedule = Schedule::from_str(expression).expect("cron expression");
    for datetime in schedule.upcoming(Utc) {
        let now = Utc::now();
        let dur = match (datetime - now).to_std() {
            // duration is less than zero
            Err(_) => continue,
            Ok(std_dur) => std_dur,
        };
        log::info!("update is going to be triggered at '{datetime}', sleep '{dur:?}'");
        sleep(dur).await;
        log::info!("perform update '{datetime}'");
        if let Err(e) = update(bot.clone()).await {
            log::error!("teloxide error in update: {e}");
        }
        log::info!("finished update '{datetime}'");
    }
}

async fn update(bot: Bot) -> Result<(), teloxide::RequestError> {
    let chats = match repo::paths::chats() {
        Err(e) => {
            log::error!("failed to get chats: {e}");
            return Ok(());
        }
        Ok(cs) => cs,
    };

    for chat in chats {
        let repos = match repo::paths::repos(chat) {
            Err(e) => {
                log::error!("failed to get repos for chat {chat}: {e}");
                continue;
            }
            Ok(rs) => rs,
        };
        for repo in repos {
            log::info!("update ({chat}, {repo})");

            let task = repo::tasks::Task {
                chat,
                repo: repo.to_owned(),
            };
            let lock = match task.try_lock() {
                Ok(Some(l)) => l,
                Ok(None) => {
                    log::info!("another task running on ({chat}, {repo}), skip");
                    continue;
                }
                Err(e) => {
                    log::error!("failed to acquire task guard in update: {e}");
                    continue;
                }
            };

            if let Err(e) = repo::fetch(lock.clone()).await {
                log::warn!("failed to fetch ({chat}, {repo}), skip: {e}");
                continue;
            }

            // check pull requests of the repo
            // check before commit
            let pull_requests = {
                let resources = lock.resources.lock().unwrap();
                resources.settings.pull_requests.clone()
            };
            for (pr, settings) in pull_requests {
                let result = match repo::pr_check(lock.clone(), pr).await {
                    Err(e) => {
                        log::warn!("failed to check pr ({chat}, {repo}, {pr}): {e}");
                        continue;
                    }
                    Ok(r) => r,
                };
                log::info!("finished pr check ({chat}, {repo}, {pr})");
                if let Some(commit) = result {
                    let message = pr_merged_message(&repo, pr, &settings, &commit);
                    bot.send_message(chat, message)
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;
                }
            }

            // check branches of the repo
            let branches = {
                let resources = lock.resources.lock().unwrap();
                resources.settings.branches.clone()
            };
            for (branch, settings) in branches {
                let result = match repo::branch_check(lock.clone(), &branch).await {
                    Err(e) => {
                        log::warn!("failed to check branch ({chat}, {repo}, {branch}): {e}");
                        continue;
                    }
                    Ok(r) => r,
                };
                log::info!("finished branch check ({chat}, {repo}, {branch})");
                if result.new != result.old {
                    let message = branch_check_message(&repo, &branch, &settings, &result);
                    bot.send_message(chat, message)
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;
                }
            }

            // check commits of the repo
            let commits = {
                let resources = lock.resources.lock().unwrap();
                resources.settings.commits.clone()
            };
            for (commit, settings) in commits {
                let result = match repo::commit_check(lock.clone(), &commit).await {
                    Err(e) => {
                        log::warn!("failed to check commit ({chat}, {repo}, {commit}): {e}",);
                        continue;
                    }
                    Ok(r) => r,
                };
                log::info!("finished commit check ({chat}, {repo}, {commit})");
                if !result.new.is_empty() {
                    let message = commit_check_message(&repo, &commit, &settings, &result);
                    bot.send_message(chat, message)
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;
                }
            }
        }
    }

    Ok(())
}

async fn list(bot: Bot, msg: Message) -> Result<(), CommandError> {
    let chat = msg.chat.id;
    let mut result = String::new();

    let repos = repo::paths::repos(chat)?;
    for repo in repos {
        result.push('*');
        result.push_str(&markdown::escape(&repo));
        result.push_str("*\n");

        let lock = (Task {
            chat,
            repo: repo.clone(),
        })
        .try_lock()?
        .ok_or(error::Error::AnotherTaskRunning(repo))?;
        let resources = lock.resources.lock().unwrap();

        result.push_str("  *commits*:\n");
        let commits = &resources.settings.commits;
        if commits.is_empty() {
            result.push_str("  \\(nothing\\)\n");
        }
        for (commit, settings) in commits {
            result.push_str(&format!(
                "  \\- `{}`\n    {}\n",
                markdown::escape(commit),
                settings.notify.description_markdown()
            ));
        }
        result.push_str("  *pull requests*:\n");
        let pull_requests = &resources.settings.pull_requests;
        if pull_requests.is_empty() {
            result.push_str("  \\(nothing\\)\n");
        }
        for (pr, settings) in pull_requests {
            result.push_str(&format!(
                "  \\- `{pr}`\n    {}\n",
                settings.notify.description_markdown()
            ));
        }
        result.push_str("  *branches*:\n");
        let branches = &resources.settings.branches;
        if branches.is_empty() {
            result.push_str("  \\(nothing\\)\n");
        }
        for (branch, settings) in branches {
            result.push_str(&format!(
                "  \\- `{}`\n    {}\n",
                markdown::escape(branch),
                settings.notify.description_markdown()
            ));
        }
        result.push_str("  *conditions*:\n");
        let conditions = &resources.settings.conditions;
        if conditions.is_empty() {
            result.push_str("  \\(nothing\\)\n");
        }
        for condition in conditions.keys() {
            result.push_str(&format!("  \\- `{}`\n", markdown::escape(condition)));
        }

        result.push('\n');
    }
    if result.is_empty() {
        result.push_str("(nothing)\n");
    }
    reply_to_msg(&bot, &msg, result)
        .parse_mode(ParseMode::MarkdownV2)
        .await?;

    Ok(())
}

async fn repo_add(bot: Bot, msg: Message, name: String, url: String) -> Result<(), CommandError> {
    let chat = msg.chat.id;
    let lock = prepare_lock_bare(chat, &name)?;
    if repo::exists(chat, &name)? {
        return Err(error::Error::RepoExists(name).into());
    }
    reply_to_msg(&bot, &msg, format!("start cloning into '{name}'")).await?;
    repo::create(lock, &url).await?;

    let github_info = Url::parse(&url)
        .ok()
        .and_then(|u| GitHubInfo::parse_from_url(u).ok());
    let settings = {
        let lock = prepare_lock(chat, &name)?;
        let settings = {
            let mut resources = lock.resources.lock().unwrap();
            resources.settings.github_info = github_info;
            resources.settings.clone()
        };
        lock.save_resources()?;
        settings
    };

    reply_to_msg(
        &bot,
        &msg,
        format!("repository '{name}' added, settings:\n{settings:#?}"),
    )
    .await?;
    Ok(())
}

async fn repo_edit(
    bot: Bot,
    msg: Message,
    name: String,
    branch_regex: Option<String>,
    github_info: Option<GitHubInfo>,
    clear_github_info: bool,
) -> Result<(), CommandError> {
    let lock = prepare_lock(msg.chat.id, &name)?;
    let new_settings = {
        let mut resources = lock.resources.lock().unwrap();
        if let Some(r) = branch_regex {
            // ensure regex is valid
            let _: Regex = Regex::new(&r).map_err(error::Error::from)?;
            resources.settings.branch_regex = r;
        }
        if let Some(info) = github_info {
            resources.settings.github_info = Some(info);
        }
        if clear_github_info {
            resources.settings.github_info = None;
        }
        resources.settings.clone()
    };
    lock.save_resources()?;
    reply_to_msg(
        &bot,
        &msg,
        format!("repository '{name}' edited, current settings:\n{new_settings:#?}"),
    )
    .await?;
    Ok(())
}

async fn repo_remove(bot: Bot, msg: Message, name: String) -> Result<(), CommandError> {
    let chat = msg.chat.id;
    let lock = prepare_lock_bare(chat, &name)?;
    if !repo::exists(chat, &name)? {
        return Err(error::Error::UnknownRepository(name).into());
    }
    repo::remove(lock).await?;
    reply_to_msg(&bot, &msg, format!("repository '{name}' removed")).await?;
    Ok(())
}

async fn commit_add(
    bot: Bot,
    msg: Message,
    repo: String,
    hash: String,
    comment: String,
    url: Option<Url>,
) -> Result<(), CommandError> {
    let lock = prepare_lock(msg.chat.id, &repo)?;
    let subscribers = subscriber_from_msg(&msg).into_iter().collect();
    let settings = CommitSettings {
        url,
        notify: NotifySettings {
            comment,
            subscribers,
        },
    };
    match repo::commit_add(lock, &hash, settings).await {
        Ok(()) => {
            reply_to_msg(&bot, &msg, format!("commit {hash} added")).await?;
        }
        Err(Error::CommitExists(_)) => {
            // do nothing
        }
        Err(e) => return Err(e.into()),
    }
    commit_check(bot, msg, repo, hash).await
}

async fn commit_remove(
    bot: Bot,
    msg: Message,
    repo: String,
    hash: String,
) -> Result<(), CommandError> {
    let lock = prepare_lock(msg.chat.id, &repo)?;
    repo::commit_remove(lock, &hash).await?;
    reply_to_msg(&bot, &msg, format!("commit {hash} removed")).await?;
    Ok(())
}

async fn commit_check(
    bot: Bot,
    msg: Message,
    repo: String,
    hash: String,
) -> Result<(), CommandError> {
    let lock = prepare_lock(msg.chat.id, &repo)?;
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
    let result = repo::commit_check(lock, &hash).await?;
    let reply = commit_check_message(&repo, &hash, &commit_settings, &result);
    reply_to_msg(&bot, &msg, reply)
        .parse_mode(ParseMode::MarkdownV2)
        .await?;

    Ok(())
}

async fn pr_add(
    bot: Bot,
    msg: Message,
    repo: String,
    pr_id: u64,
    optional_comment: Option<String>,
) -> Result<(), CommandError> {
    let lock = prepare_lock(msg.chat.id, &repo)?;
    let github_info = {
        let resources = lock.resources.lock().unwrap();
        resources
            .settings
            .github_info
            .clone()
            .ok_or(Error::NoGitHubInfo(repo.clone()))?
    };
    let url_str = format!("https://github.com/{github_info}/pull/{pr_id}");
    let url = Url::parse(&url_str).map_err(Error::UrlParse)?;
    let subscribers = subscriber_from_msg(&msg).into_iter().collect();
    let comment = optional_comment.unwrap_or_default();
    let settings = PullRequestSettings {
        url,
        notify: NotifySettings {
            comment,
            subscribers,
        },
    };
    repo::pr_add(lock, pr_id, settings).await?;
    // lock already dropped in repo::pr_add
    pr_check(bot, msg, repo, pr_id).await
}

async fn pr_check(bot: Bot, msg: Message, repo: String, pr_id: u64) -> Result<(), CommandError> {
    let lock = prepare_lock(msg.chat.id, &repo)?;
    match repo::pr_check(lock, pr_id).await? {
        Some(commit) => {
            reply_to_msg(
                &bot,
                &msg,
                format!("pr {pr_id} has been merged, commit `{commit}` added"),
            )
            .await?;
            // lock already dropped in repo::pr_check
            commit_check(bot, msg, repo, commit).await?;
        }
        None => {
            reply_to_msg(&bot, &msg, format!("pr {pr_id} has not been merged yet")).await?;
        }
    }
    Ok(())
}

async fn pr_remove(bot: Bot, msg: Message, repo: String, pr_id: u64) -> Result<(), CommandError> {
    let lock = prepare_lock(msg.chat.id, &repo)?;
    repo::pr_remove(lock, pr_id).await?;
    reply_to_msg(&bot, &msg, format!("pr {pr_id} removed")).await?;
    Ok(())
}

async fn branch_add(
    bot: Bot,
    msg: Message,
    repo: String,
    branch: String,
) -> Result<(), CommandError> {
    let lock = prepare_lock(msg.chat.id, &repo)?;
    let settings = BranchSettings {
        notify: Default::default(),
    };
    repo::branch_add(lock, &branch, settings).await?;
    branch_check(bot, msg, repo, branch).await
}

async fn branch_remove(
    bot: Bot,
    msg: Message,
    repo: String,
    branch: String,
) -> Result<(), CommandError> {
    let lock = prepare_lock(msg.chat.id, &repo)?;
    repo::branch_remove(lock, &branch).await?;
    reply_to_msg(&bot, &msg, format!("branch {branch} removed")).await?;
    Ok(())
}

async fn branch_check(
    bot: Bot,
    msg: Message,
    repo: String,
    branch: String,
) -> Result<(), CommandError> {
    let lock = prepare_lock(msg.chat.id, &repo)?;
    repo::fetch(lock.clone()).await?;
    let branch_settings = {
        let resources = lock.resources.lock().unwrap();
        resources
            .settings
            .branches
            .get(&branch)
            .ok_or_else(|| error::Error::UnknownBranch(branch.clone()))?
            .clone()
    };
    let result = repo::branch_check(lock, &branch).await?;
    let reply = branch_check_message(&repo, &branch, &branch_settings, &result);
    reply_to_msg(&bot, &msg, reply)
        .parse_mode(ParseMode::MarkdownV2)
        .await?;

    Ok(())
}

async fn condition_add(
    bot: Bot,
    msg: Message,
    repo: String,
    identifier: String,
    kind: condition::Kind,
    expr: String,
) -> Result<(), CommandError> {
    let lock = prepare_lock(msg.chat.id, &repo)?;
    let settings = ConditionSettings {
        condition: GeneralCondition::parse(kind, &expr)?,
    };
    repo::condition_add(lock, &identifier, settings).await?;
    reply_to_msg(&bot, &msg, format!("condition {identifier} added")).await?;
    condition_trigger(bot, msg, repo, identifier).await
}

async fn condition_remove(
    bot: Bot,
    msg: Message,
    repo: String,
    identifier: String,
) -> Result<(), CommandError> {
    let lock = prepare_lock(msg.chat.id, &repo)?;
    repo::condition_remove(lock, &identifier).await?;
    reply_to_msg(&bot, &msg, format!("condition {identifier} removed")).await?;
    Ok(())
}

async fn condition_trigger(
    bot: Bot,
    msg: Message,
    repo: String,
    identifier: String,
) -> Result<(), CommandError> {
    let lock = prepare_lock(msg.chat.id, &repo)?;
    let result = repo::condition_trigger(lock, &identifier).await?;
    reply_to_msg(
        &bot,
        &msg,
        condition_check_message(&repo, &identifier, &result),
    )
    .parse_mode(ParseMode::MarkdownV2)
    .await?;
    Ok(())
}

fn prepare_lock(chat: ChatId, repo: &str) -> Result<TaskGuard, error::Error> {
    let task = repo::tasks::Task {
        chat,
        repo: repo.to_owned(),
    };
    match task.try_lock() {
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

fn prepare_lock_bare(chat: ChatId, repo: &str) -> Result<TaskGuardBare, error::Error> {
    let task = repo::tasks::Task {
        chat,
        repo: repo.to_owned(),
    };
    match task.try_lock_bare()? {
        Some(lock) => Ok(lock),
        None => {
            log::info!("ignored command from {} on '{}'", chat, repo);
            Err(error::Error::AnotherTaskRunning(repo.to_owned()))
        }
    }
}

fn commit_check_message(
    repo: &str,
    commit: &str,
    settings: &CommitSettings,
    result: &repo::CommitCheckResult,
) -> String {
    let auto_remove_msg = match &result.removed_by_condition {
        None => String::new(),
        Some(condition) => format!(
            "\n*auto removed* by condition: `{}`",
            markdown::escape(condition)
        ),
    };
    format!(
        "{repo}/`{commit}`{notify}

*new* branches containing this commit:
{new}

*all* branches containing this commit:
{all}
{auto_remove_msg}
",
        repo = markdown::escape(repo),
        commit = markdown::escape(commit),
        notify = push_empty_line(&settings.notify.notify_markdown()),
        new = markdown_list(result.new.iter()),
        all = markdown_list(result.all.iter())
    )
}

fn pr_merged_message(
    repo: &str,
    pr: u64,
    settings: &PullRequestSettings,
    commit: &String,
) -> String {
    format!(
        "{repo}/{pr}
        merged as `{commit}`{notify}
",
        notify = push_empty_line(&settings.notify.notify_markdown()),
    )
}

fn branch_check_message(
    repo: &str,
    branch: &str,
    settings: &BranchSettings,
    result: &BranchCheckResult,
) -> String {
    let status = if result.old == result.new {
        format!(
            "{}
\\(not changed\\)
",
            markdown_optional_commit(result.new.as_deref())
        )
    } else {
        format!(
            "{old} \u{2192}
{new}
",
            old = markdown_optional_commit(result.old.as_deref()),
            new = markdown_optional_commit(result.new.as_deref()),
        )
    };
    format!(
        "{repo}/`{branch}`
{status}{notify}
",
        repo = markdown::escape(repo),
        branch = markdown::escape(branch),
        notify = push_empty_line(&settings.notify.notify_markdown()),
    )
}

fn condition_check_message(
    repo: &str,
    identifier: &str,
    result: &repo::ConditionCheckResult,
) -> String {
    format!(
        "{repo}/`{identifier}`

branches removed by this condition:
{removed}
",
        repo = markdown::escape(repo),
        identifier = markdown::escape(identifier),
        removed = markdown_list(result.removed.iter()),
    )
}

fn markdown_optional_commit(commit: Option<&str>) -> String {
    match &commit {
        None => "\\(nothing\\)".to_owned(),
        Some(c) => markdown::code_inline(&markdown::escape(c)),
    }
}

fn markdown_list<Iter, T>(items: Iter) -> String
where
    Iter: Iterator<Item = T>,
    T: fmt::Display,
{
    let mut result = String::new();
    for item in items {
        result.push_str(&format!("\\- `{}`\n", markdown::escape(&item.to_string())));
    }
    if result.is_empty() {
        "\u{2205}".to_owned() // the empty set symbol
    } else {
        assert_eq!(result.pop(), Some('\n'));
        result
    }
}

fn subscriber_from_msg(msg: &Message) -> Option<Subscriber> {
    match msg.from() {
        None => None,
        Some(u) => u.username.as_ref().map(|name| Subscriber::Telegram {
            username: name.to_string(),
        }),
    }
}
