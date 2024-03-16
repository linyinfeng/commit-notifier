mod cache;
mod command;
mod condition;
mod error;
mod github;
mod options;
mod repo;
mod utils;

use std::collections::BTreeSet;
use std::env;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

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
use repo::BranchCheckResult;
use serde::Deserialize;
use serde::Serialize;
use teloxide::dispatching::dialogue::GetChatId;
use teloxide::prelude::*;
use teloxide::types::InlineKeyboardButton;
use teloxide::types::InlineKeyboardButtonKind;
use teloxide::types::InlineKeyboardMarkup;
use teloxide::types::ParseMode;
use teloxide::update_listeners;
use teloxide::utils::command::BotCommands;
use teloxide::utils::markdown;
use tokio::time::sleep;
use url::Url;
use utils::reply_to_msg;

use crate::repo::tasks::Resources;
use crate::repo::tasks::ResourcesMap;
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
                command::Notifier::CommitSubscribe {
                    repo,
                    hash,
                    unsubscribe,
                } => commit_subscribe(bot, msg, repo, hash, unsubscribe).await,
                command::Notifier::PrAdd { repo, pr, comment } => {
                    pr_add(bot, msg, repo, pr, comment).await
                }
                command::Notifier::PrRemove { repo, pr } => pr_remove(bot, msg, repo, pr).await,
                command::Notifier::PrCheck { repo, pr } => pr_check(bot, msg, repo, pr).await,
                command::Notifier::PrSubscribe {
                    repo,
                    pr,
                    unsubscribe,
                } => pr_subscribe(bot, msg, repo, pr, unsubscribe).await,
                command::Notifier::BranchAdd { repo, branch } => {
                    branch_add(bot, msg, repo, branch).await
                }
                command::Notifier::BranchRemove { repo, branch } => {
                    branch_remove(bot, msg, repo, branch).await
                }
                command::Notifier::BranchCheck { repo, branch } => {
                    branch_check(bot, msg, repo, branch).await
                }
                command::Notifier::BranchSubscribe {
                    repo,
                    branch,
                    unsubscribe,
                } => branch_subscribe(bot, msg, repo, branch, unsubscribe).await,
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

async fn handle_callback_query(bot: Bot, query: CallbackQuery) -> ResponseResult<()> {
    match handle_callback_query_command_result(&bot, &query).await {
        Ok(msg) => match get_chat_id_and_username_from_query(&query) {
            Ok((chat_id, username)) => {
                bot.send_message(chat_id, format!("@{username} {msg}"))
                    .await?;
                Ok(())
            }
            Err(e) => {
                log::error!("callback query error: {e}");
                Ok(())
            }
        },
        Err(CommandError::Normal(e)) => match get_chat_id_and_username_from_query(&query) {
            Ok((chat_id, username)) => e.report_to_user(&bot, chat_id, &username).await,
            Err(_e) => {
                log::error!("callback query error: {e}");
                Ok(())
            }
        },
        Err(CommandError::Teloxide(e)) => Err(e),
    }
}

async fn handle_callback_query_command_result(
    _bot: &Bot,
    query: &CallbackQuery,
) -> Result<String, CommandError> {
    log::debug!("query = {query:?}");
    let (chat_id, username) = get_chat_id_and_username_from_query(query)?;
    let subscriber = Subscriber::Telegram { username };
    let _msg = query
        .message
        .as_ref()
        .ok_or(Error::SubscribeCallbackNoMsgId)?;
    let data = query.data.as_ref().ok_or(Error::SubscribeCallbackNoData)?;
    let SubscribeTerm(kind, repo, id, subscribe) =
        serde_json::from_str(data).map_err(Error::Serde)?;
    let unsubscribe = subscribe == 0;
    match kind.as_str() {
        "b" => {
            let resources = resources_helper_chat(chat_id, &repo).await?;
            {
                let mut settings = resources.settings.write().await;
                let subscribers = &mut settings
                    .branches
                    .get_mut(&id)
                    .ok_or_else(|| Error::UnknownBranch(id.clone()))?
                    .notify
                    .subscribers;
                modify_subscriber_set(subscribers, subscriber, unsubscribe)?;
            }
            resources.save_settings().await?;
        }
        "c" => {
            let resources = resources_helper_chat(chat_id, &repo).await?;
            {
                let mut settings = resources.settings.write().await;
                let subscribers = &mut settings
                    .commits
                    .get_mut(&id)
                    .ok_or_else(|| Error::UnknownCommit(id.clone()))?
                    .notify
                    .subscribers;
                modify_subscriber_set(subscribers, subscriber, unsubscribe)?;
            }
            resources.save_settings().await?;
        }
        "p" => {
            let pr_id: u64 = id.parse().map_err(Error::ParseInt)?;
            let resources = resources_helper_chat(chat_id, &repo).await?;
            {
                let mut settings = resources.settings.write().await;
                let subscribers = &mut settings
                    .pull_requests
                    .get_mut(&pr_id)
                    .ok_or_else(|| Error::UnknownPullRequest(pr_id))?
                    .notify
                    .subscribers;
                modify_subscriber_set(subscribers, subscriber, unsubscribe)?;
            }
            resources.save_settings().await?;
        }
        _ => Err(Error::SubscribeCallbackDataInvalidKind(kind))?,
    }
    if unsubscribe {
        Ok(format!("unsubscribed from {repo}/{id}"))
    } else {
        Ok(format!("subscribed to {repo}/{id}"))
    }
}

fn get_chat_id_and_username_from_query(query: &CallbackQuery) -> Result<(ChatId, String), Error> {
    let chat_id = query.chat_id().ok_or(Error::SubscribeCallbackNoChatId)?;
    let username = query
        .from
        .username
        .as_ref()
        .ok_or(Error::SubscribeCallbackNoUsername)?
        .clone();
    Ok((chat_id, username))
}

enum CommandError {
    Normal(Error),
    Teloxide(teloxide::RequestError),
}
impl From<Error> for CommandError {
    fn from(e: Error) -> Self {
        CommandError::Normal(e)
    }
}
impl From<teloxide::RequestError> for CommandError {
    fn from(e: teloxide::RequestError) -> Self {
        CommandError::Teloxide(e)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct SubscribeTerm(String, String, String, usize);

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
    let command_handler = teloxide::filter_command::<BCommand, _>().endpoint(answer);
    let message_handler = Update::filter_message().branch(command_handler);
    let callback_handler = Update::filter_callback_query().endpoint(handle_callback_query);
    let handler = dptree::entry()
        .branch(message_handler)
        .branch(callback_handler);
    let mut dispatcher = Dispatcher::builder(bot.clone(), handler)
        .enable_ctrlc_handler()
        .build();

    let update_listener = update_listeners::polling_default(bot.clone()).await;
    tokio::select! {
        _ = schedule(bot.clone()) => { },
        _ = dispatcher.dispatch_with_listener(
            update_listener,
            LoggingErrorHandler::with_custom_text("An error from the update listener"),
        ) => { },
    }

    log::info!("cleaning up resources");
    if let Err(e) = ResourcesMap::clear().await {
        log::error!("failed to clear resources map: {e}");
    }
    log::info!("exit");
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

            let resources = match ResourcesMap::get(&task).await {
                Ok(r) => r,
                Err(e) => {
                    log::warn!("failed to open resources of ({chat}, {repo}), skip: {e}");
                    continue;
                }
            };

            if let Err(e) = repo::fetch(resources.clone()).await {
                log::warn!("failed to fetch ({chat}, {repo}), skip: {e}");
                continue;
            }

            // check pull requests of the repo
            // check before commit
            let pull_requests = {
                let settings = resources.settings.read().await;
                settings.pull_requests.clone()
            };
            for (pr, settings) in pull_requests {
                let result = match repo::pr_check(resources.clone(), pr).await {
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
                let settings = resources.settings.read().await;
                settings.branches.clone()
            };
            for (branch, settings) in branches {
                let result = match repo::branch_check(resources.clone(), &branch).await {
                    Err(e) => {
                        log::warn!("failed to check branch ({chat}, {repo}, {branch}): {e}");
                        continue;
                    }
                    Ok(r) => r,
                };
                log::info!("finished branch check ({chat}, {repo}, {branch})");
                if result.new != result.old {
                    let message = branch_check_message(&repo, &branch, &settings, &result);
                    let markup = subscribe_button_markup("b", &repo, &branch);
                    let mut send = bot
                        .send_message(chat, message)
                        .parse_mode(ParseMode::MarkdownV2);
                    match markup {
                        Ok(m) => {
                            send = send.reply_markup(m);
                        }
                        Err(e) => {
                            log::error!(
                                "failed to create markup for ({chat}, {repo}, {branch}): {e}"
                            );
                        }
                    }
                    send.await?;
                }
            }

            // check commits of the repo
            let commits = {
                let settings = resources.settings.read().await;
                settings.commits.clone()
            };
            for (commit, settings) in commits {
                let result = match repo::commit_check(resources.clone(), &commit).await {
                    Err(e) => {
                        log::warn!("failed to check commit ({chat}, {repo}, {commit}): {e}",);
                        continue;
                    }
                    Ok(r) => r,
                };
                log::info!("finished commit check ({chat}, {repo}, {commit})");
                if !result.new.is_empty() {
                    let message = commit_check_message(&repo, &commit, &settings, &result);
                    let mut send = bot
                        .send_message(chat, message)
                        .parse_mode(ParseMode::MarkdownV2);
                    if result.removed_by_condition.is_none() {
                        let markup = subscribe_button_markup("c", &repo, &commit);
                        match markup {
                            Ok(m) => {
                                send = send.reply_markup(m);
                            }
                            Err(e) => {
                                log::error!(
                                    "failed to create markup for ({chat}, {repo}, {commit}): {e}"
                                );
                            }
                        }
                    }
                    send.await?;
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

        let task = Task {
            chat,
            repo: repo.clone(),
        };
        let resources = ResourcesMap::get(&task).await?;
        let settings = {
            let locked = resources.settings.read().await;
            locked.clone()
        };

        result.push_str("  *commits*:\n");
        let commits = &settings.commits;
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
        let pull_requests = &settings.pull_requests;
        if pull_requests.is_empty() {
            result.push_str("  \\(nothing\\)\n");
        }
        for (pr, settings) in pull_requests {
            result.push_str(&format!(
                "  \\- `{pr}`\n    {}\n",
                markdown::escape(settings.url.as_str())
            ));
        }
        result.push_str("  *branches*:\n");
        let branches = &settings.branches;
        if branches.is_empty() {
            result.push_str("  \\(nothing\\)\n");
        }
        for branch in branches.keys() {
            result.push_str(&format!("  \\- `{}`\n", markdown::escape(branch)));
        }
        result.push_str("  *conditions*:\n");
        let conditions = &settings.conditions;
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
    if repo::exists(chat, &name)? {
        return Err(Error::RepoExists(name).into());
    }
    reply_to_msg(&bot, &msg, format!("start cloning into '{name}'")).await?;

    let task = Task {
        chat,
        repo: name.clone(),
    };
    let path = task.paths()?;
    repo::create(&name, path.repo, &url).await?;

    let resources = ResourcesMap::get(&task).await?;

    let github_info = Url::parse(&url)
        .ok()
        .and_then(|u| GitHubInfo::parse_from_url(u).ok());
    let settings = {
        let mut locked = resources.settings.write().await;
        locked.github_info = github_info;
        locked.clone()
    };
    resources.save_settings().await?;

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
    let resources = resources_helper(&msg, &name).await?;
    let new_settings = {
        let mut locked = resources.settings.write().await;
        if let Some(r) = branch_regex {
            // ensure regex is valid
            let _: Regex = Regex::new(&r).map_err(Error::from)?;
            locked.branch_regex = r;
        }
        if let Some(info) = github_info {
            locked.github_info = Some(info);
        }
        if clear_github_info {
            locked.github_info = None;
        }
        locked.clone()
    };
    resources.save_settings().await?;
    reply_to_msg(
        &bot,
        &msg,
        format!("repository '{name}' edited, current settings:\n{new_settings:#?}"),
    )
    .await?;
    Ok(())
}

async fn repo_remove(bot: Bot, msg: Message, name: String) -> Result<(), CommandError> {
    let resources = resources_helper(&msg, &name).await?;
    if !repo::exists(resources.task.chat, &name)? {
        return Err(Error::UnknownRepository(name).into());
    }
    repo::remove(resources).await?;
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
    let resources = resources_helper(&msg, &repo).await?;
    let subscribers = subscriber_from_msg(&msg).into_iter().collect();
    let settings = CommitSettings {
        url,
        notify: NotifySettings {
            comment,
            subscribers,
        },
    };
    match repo::commit_add(resources, &hash, settings).await {
        Ok(()) => {
            reply_to_msg(&bot, &msg, format!("commit {hash} added")).await?;
            commit_check(bot, msg, repo, hash).await?;
        }
        Err(Error::CommitExists(_)) => {
            commit_subscribe(bot.clone(), msg.clone(), repo.clone(), hash.clone(), false).await?;
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

async fn commit_remove(
    bot: Bot,
    msg: Message,
    repo: String,
    hash: String,
) -> Result<(), CommandError> {
    let resources = resources_helper(&msg, &repo).await?;
    repo::commit_remove(resources, &hash).await?;
    reply_to_msg(&bot, &msg, format!("commit {hash} removed")).await?;
    Ok(())
}

async fn commit_check(
    bot: Bot,
    msg: Message,
    repo: String,
    hash: String,
) -> Result<(), CommandError> {
    let resources = resources_helper(&msg, &repo).await?;
    repo::fetch(resources.clone()).await?;
    let commit_settings = {
        let settings = resources.settings.read().await;
        settings
            .commits
            .get(&hash)
            .ok_or_else(|| Error::UnknownCommit(hash.clone()))?
            .clone()
    };
    let result = repo::commit_check(resources, &hash).await?;
    let reply = commit_check_message(&repo, &hash, &commit_settings, &result);
    let mut send = reply_to_msg(&bot, &msg, reply).parse_mode(ParseMode::MarkdownV2);
    if result.removed_by_condition.is_none() {
        match subscribe_button_markup("c", &repo, &hash) {
            Ok(m) => {
                send = send.reply_markup(m);
            }
            Err(e) => {
                log::error!(
                    "failed to create markup for ({chat}, {repo}, {hash}): {e}",
                    chat = msg.chat.id
                );
            }
        }
    }
    send.await?;
    Ok(())
}

async fn commit_subscribe(
    bot: Bot,
    msg: Message,
    repo: String,
    hash: String,
    unsubscribe: bool,
) -> Result<(), CommandError> {
    let resources = resources_helper(&msg, &repo).await?;
    let subscriber = subscriber_from_msg(&msg).ok_or(Error::NoSubscriber)?;
    {
        let mut settings = resources.settings.write().await;
        let subscribers = &mut settings
            .commits
            .get_mut(&hash)
            .ok_or_else(|| Error::UnknownCommit(hash.clone()))?
            .notify
            .subscribers;
        modify_subscriber_set(subscribers, subscriber, unsubscribe)?;
    }
    resources.save_settings().await?;
    reply_to_msg(&bot, &msg, "done").await?;
    Ok(())
}

async fn pr_add(
    bot: Bot,
    msg: Message,
    repo: String,
    pr_id: u64,
    optional_comment: Option<String>,
) -> Result<(), CommandError> {
    let resources = resources_helper(&msg, &repo).await?;
    let github_info = {
        let settings = resources.settings.read().await;
        settings
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
    match repo::pr_add(resources, pr_id, settings).await {
        Ok(()) => {
            reply_to_msg(&bot, &msg, format!("pr {pr_id} added")).await?;
            pr_check(bot, msg, repo, pr_id).await?;
        }
        Err(Error::PullRequestExists(_)) => {
            pr_subscribe(bot.clone(), msg.clone(), repo.clone(), pr_id, false).await?;
        }
        Err(e) => return Err(e.into()),
    };
    Ok(())
}

async fn pr_check(bot: Bot, msg: Message, repo: String, pr_id: u64) -> Result<(), CommandError> {
    let resources = resources_helper(&msg, &repo).await?;
    match repo::pr_check(resources, pr_id).await {
        Ok(Some(commit)) => {
            reply_to_msg(
                &bot,
                &msg,
                format!("pr {pr_id} has been merged\ncommit `{commit}` added"),
            )
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
            commit_check(bot, msg, repo, commit).await?;
        }
        Ok(None) => {
            reply_to_msg(&bot, &msg, format!("pr {pr_id} has not been merged yet")).await?;
        }
        Err(Error::CommitExists(commit)) => {
            commit_subscribe(bot, msg, repo, commit, false).await?;
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

async fn pr_remove(bot: Bot, msg: Message, repo: String, pr_id: u64) -> Result<(), CommandError> {
    let resources = resources_helper(&msg, &repo).await?;
    repo::pr_remove(resources, pr_id).await?;
    reply_to_msg(&bot, &msg, format!("pr {pr_id} removed")).await?;
    Ok(())
}

async fn pr_subscribe(
    bot: Bot,
    msg: Message,
    repo: String,
    pr_id: u64,
    unsubscribe: bool,
) -> Result<(), CommandError> {
    let resources = resources_helper(&msg, &repo).await?;
    let subscriber = subscriber_from_msg(&msg).ok_or(Error::NoSubscriber)?;
    {
        let mut settings = resources.settings.write().await;
        let subscribers = &mut settings
            .pull_requests
            .get_mut(&pr_id)
            .ok_or_else(|| Error::UnknownPullRequest(pr_id))?
            .notify
            .subscribers;
        modify_subscriber_set(subscribers, subscriber, unsubscribe)?;
    }
    resources.save_settings().await?;
    reply_to_msg(&bot, &msg, "done").await?;
    Ok(())
}

async fn branch_add(
    bot: Bot,
    msg: Message,
    repo: String,
    branch: String,
) -> Result<(), CommandError> {
    let resources = resources_helper(&msg, &repo).await?;
    let settings = BranchSettings {
        notify: Default::default(),
    };
    match repo::branch_add(resources, &branch, settings).await {
        Ok(()) => {
            branch_check(bot, msg, repo, branch).await?;
        }
        Err(Error::BranchExists(_)) => {
            branch_subscribe(
                bot.clone(),
                msg.clone(),
                repo.clone(),
                branch.clone(),
                false,
            )
            .await?;
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

async fn branch_remove(
    bot: Bot,
    msg: Message,
    repo: String,
    branch: String,
) -> Result<(), CommandError> {
    let resources = resources_helper(&msg, &repo).await?;
    repo::branch_remove(resources, &branch).await?;
    reply_to_msg(&bot, &msg, format!("branch {branch} removed")).await?;
    Ok(())
}

async fn branch_check(
    bot: Bot,
    msg: Message,
    repo: String,
    branch: String,
) -> Result<(), CommandError> {
    let resources = resources_helper(&msg, &repo).await?;
    repo::fetch(resources.clone()).await?;
    let branch_settings = {
        let settings = resources.settings.read().await;
        settings
            .branches
            .get(&branch)
            .ok_or_else(|| Error::UnknownBranch(branch.clone()))?
            .clone()
    };
    let result = repo::branch_check(resources, &branch).await?;
    let reply = branch_check_message(&repo, &branch, &branch_settings, &result);

    let mut send = reply_to_msg(&bot, &msg, reply).parse_mode(ParseMode::MarkdownV2);
    match subscribe_button_markup("b", &repo, &branch) {
        Ok(m) => {
            send = send.reply_markup(m);
        }
        Err(e) => {
            log::error!(
                "failed to create markup for ({chat}, {repo}, {branch}): {e}",
                chat = msg.chat.id
            );
        }
    }
    send.await?;

    Ok(())
}

async fn branch_subscribe(
    bot: Bot,
    msg: Message,
    repo: String,
    branch: String,
    unsubscribe: bool,
) -> Result<(), CommandError> {
    let resources = resources_helper(&msg, &repo).await?;
    let subscriber = subscriber_from_msg(&msg).ok_or(Error::NoSubscriber)?;
    {
        let mut settings = resources.settings.write().await;
        let subscribers = &mut settings
            .branches
            .get_mut(&branch)
            .ok_or_else(|| Error::UnknownBranch(branch.clone()))?
            .notify
            .subscribers;
        modify_subscriber_set(subscribers, subscriber, unsubscribe)?;
    }
    resources.save_settings().await?;
    reply_to_msg(&bot, &msg, "done").await?;
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
    let resources = resources_helper(&msg, &repo).await?;
    let settings = ConditionSettings {
        condition: GeneralCondition::parse(kind, &expr)?,
    };
    repo::condition_add(resources, &identifier, settings).await?;
    reply_to_msg(&bot, &msg, format!("condition {identifier} added")).await?;
    condition_trigger(bot, msg, repo, identifier).await
}

async fn condition_remove(
    bot: Bot,
    msg: Message,
    repo: String,
    identifier: String,
) -> Result<(), CommandError> {
    let resources = resources_helper(&msg, &repo).await?;
    repo::condition_remove(resources, &identifier).await?;
    reply_to_msg(&bot, &msg, format!("condition {identifier} removed")).await?;
    Ok(())
}

async fn condition_trigger(
    bot: Bot,
    msg: Message,
    repo: String,
    identifier: String,
) -> Result<(), CommandError> {
    let resources = resources_helper(&msg, &repo).await?;
    let result = repo::condition_trigger(resources, &identifier).await?;
    reply_to_msg(
        &bot,
        &msg,
        condition_check_message(&repo, &identifier, &result),
    )
    .parse_mode(ParseMode::MarkdownV2)
    .await?;
    Ok(())
}

async fn resources_helper(msg: &Message, repo: &str) -> Result<Arc<Resources>, Error> {
    let task = Task {
        chat: msg.chat.id,
        repo: repo.to_string(),
    };
    ResourcesMap::get(&task).await
}

async fn resources_helper_chat(chat: ChatId, repo: &str) -> Result<Arc<Resources>, Error> {
    let task = Task {
        chat,
        repo: repo.to_string(),
    };
    ResourcesMap::get(&task).await
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
        "{repo}/`{commit}`{url}{notify}

*new* branches containing this commit:
{new}

*all* branches containing this commit:
{all}
{auto_remove_msg}
",
        repo = markdown::escape(repo),
        commit = markdown::escape(commit),
        url = settings
            .url
            .as_ref()
            .map(|u| format!("\n{}", markdown::escape(u.as_str())))
            .unwrap_or_default(),
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
\\(not changed\\)",
            markdown_optional_commit(result.new.as_deref())
        )
    } else {
        format!(
            "{old} \u{2192}
{new}",
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

fn modify_subscriber_set(
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

fn subscribe_button_markup(
    kind: &str,
    repo: &str,
    id: &str,
) -> Result<InlineKeyboardMarkup, Error> {
    let mut item = SubscribeTerm(kind.to_owned(), repo.to_owned(), id.to_owned(), 1);
    let subscribe_data = serde_json::to_string(&item)?;
    item.3 = 0;
    let unsubscribe_data = serde_json::to_string(&item)?;
    let subscribe_len = subscribe_data.as_bytes().len();
    if subscribe_len > 64 {
        return Err(Error::SubscribeTermSizeExceeded(
            subscribe_len,
            subscribe_data,
        ));
    }
    let unsubscribe_len = unsubscribe_data.as_bytes().len();
    if unsubscribe_len > 64 {
        return Err(Error::SubscribeTermSizeExceeded(
            unsubscribe_len,
            unsubscribe_data,
        ));
    }
    let subscribe_button = InlineKeyboardButton::new(
        "Subscribe",
        InlineKeyboardButtonKind::CallbackData(subscribe_data),
    );
    let unsubscribe_button = InlineKeyboardButton::new(
        "Unsubscribe",
        InlineKeyboardButtonKind::CallbackData(unsubscribe_data),
    );
    Ok(InlineKeyboardMarkup::new([[
        subscribe_button,
        unsubscribe_button,
    ]]))
}
