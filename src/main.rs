mod chat;
mod command;
mod condition;
mod error;
mod github;
mod message;
mod options;
mod repo;
mod resources;
mod update;
mod utils;

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::env;
use std::fmt;
use std::str::FromStr;
use std::sync::LazyLock;

use chrono::Utc;
use cron::Schedule;
use error::Error;
use github::GitHubInfo;
use regex::Regex;
use serde::Deserialize;
use serde::Serialize;
use teloxide::dispatching::dialogue::GetChatId;
use teloxide::payloads;
use teloxide::payloads::SendMessage;
use teloxide::prelude::*;
use teloxide::requests::JsonRequest;
use teloxide::sugar::request::RequestLinkPreviewExt;
use teloxide::types::InlineKeyboardButton;
use teloxide::types::InlineKeyboardButtonKind;
use teloxide::types::InlineKeyboardMarkup;
use teloxide::types::ParseMode;
use teloxide::update_listeners;
use teloxide::utils::command::BotCommands;
use teloxide::utils::markdown;
use tokio::time::sleep;
use url::Url;

use crate::chat::results::PRIssueCheckResult;
use crate::chat::settings::BranchSettings;
use crate::chat::settings::CommitSettings;
use crate::chat::settings::NotifySettings;
use crate::chat::settings::PRIssueSettings;
use crate::chat::settings::Subscriber;
use crate::condition::Action;
use crate::condition::GeneralCondition;
use crate::condition::in_branch::InBranchCondition;
use crate::message::branch_check_message;
use crate::message::commit_check_message;
use crate::message::pr_issue_id_pretty;
use crate::message::subscriber_from_msg;
use crate::repo::pr_issue_url;
use crate::repo::settings::ConditionSettings;
use crate::update::update_and_report_error;
use crate::utils::modify_subscriber_set;
use crate::utils::reply_to_msg;

#[derive(BotCommands, Clone, Debug)]
#[command(rename_rule = "lowercase", description = "Supported commands:")]
enum BCommand {
    #[command(description = "main and the only command.")]
    Notifier(String),
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

    log::info!("cleaning up resources for chats");
    if let Err(e) = chat::resources::RESOURCES_MAP.clear().await {
        log::error!("failed to clear resources map for chats: {e}");
    }
    log::info!("cleaning up resources for repositories");
    if let Err(e) = repo::resources::RESOURCES_MAP.clear().await {
        log::error!("failed to clear resources map for repositories: {e}");
    }
    log::info!("exit");
}

async fn answer(bot: Bot, msg: Message, bc: BCommand) -> ResponseResult<()> {
    log::trace!("message: {msg:?}");
    log::trace!("bot command: {bc:?}");
    let BCommand::Notifier(input) = bc;
    let result = match command::parse(input) {
        Ok(command) => {
            log::debug!("command: {command:?}");
            let (bot, msg) = (bot.clone(), msg.clone());
            match command {
                command::Notifier::ChatId => return_chat_id(bot, msg).await,
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
                command::Notifier::PrAdd {
                    repo_or_url,
                    id,
                    comment,
                } => pr_issue_add(bot, msg, repo_or_url, id, comment).await,
                command::Notifier::PrRemove { repo, id } => {
                    pr_issue_remove(bot, msg, repo, id).await
                }
                command::Notifier::PrCheck { repo, id } => pr_issue_check(bot, msg, repo, id).await,
                command::Notifier::PrSubscribe {
                    repo,
                    id,
                    unsubscribe,
                } => pr_issue_subscribe(bot, msg, repo, id, unsubscribe).await,
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

#[derive(Serialize, Deserialize, Clone, Debug)]
struct SubscribeTerm(String, String, String, usize);

async fn handle_callback_query(bot: Bot, query: CallbackQuery) -> ResponseResult<()> {
    let result = handle_callback_query_command_result(&bot, &query).await;
    let (message, alert) = match result {
        Ok(msg) => (msg, false),
        Err(CommandError::Normal(e)) => (format!("{e}"), true),
        Err(CommandError::Teloxide(e)) => return Err(e),
    };
    let answer = payloads::AnswerCallbackQuery::new(query.id)
        .text(message)
        .show_alert(alert);
    <Bot as Requester>::AnswerCallbackQuery::new(bot, answer).await?;
    Ok(())
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
            let resources = chat::resources_chat_repo(chat_id, repo.clone()).await?;
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
            let resources = chat::resources_chat_repo(chat_id, repo.clone()).await?;
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
            let resources = chat::resources_chat_repo(chat_id, repo.clone()).await?;
            {
                let mut settings = resources.settings.write().await;
                let subscribers = &mut settings
                    .pr_issues
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
impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CommandError::Normal(e) => write!(f, "{e}"),
            CommandError::Teloxide(e) => write!(f, "{e}"),
        }
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
    // always update once on startup
    if let Err(e) = update_and_report_error(bot.clone()).await {
        log::error!("teloxide error in update: {e}");
    }

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
        if let Err(e) = update_and_report_error(bot.clone()).await {
            log::error!("teloxide error in update: {e}");
        }
        log::info!("finished update '{datetime}'");
    }
}

async fn list(bot: Bot, msg: Message) -> Result<(), CommandError> {
    let options = options::get();
    let chat = msg.chat.id;
    if ChatId(options.admin_chat_id) == chat {
        list_for_admin(bot.clone(), &msg).await?;
    }
    list_for_normal(bot, &msg).await
}

async fn list_for_admin(bot: Bot, msg: &Message) -> Result<(), CommandError> {
    log::info!("list for admin");
    let mut result = String::new();

    let repos = repo::list().await?;
    for repo in repos {
        result.push('*');
        result.push_str(&markdown::escape(&repo));
        result.push_str("*\n");
    }
    if result.is_empty() {
        result.push_str("(nothing)\n");
    }
    reply_to_msg(&bot, msg, result)
        .parse_mode(ParseMode::MarkdownV2)
        .disable_link_preview(true)
        .await?;

    Ok(())
}

async fn list_for_normal(bot: Bot, msg: &Message) -> Result<(), CommandError> {
    let chat = msg.chat.id;
    log::info!("list for chat: {chat}");
    let mut result = String::new();

    let repos = chat::repos(chat).await?;
    for repo in repos {
        result.push('*');
        result.push_str(&markdown::escape(&repo));
        result.push_str("*\n");

        let resources = chat::resources_chat_repo(chat, repo).await?;
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
        let pr_issues = &settings.pr_issues;
        if pr_issues.is_empty() {
            result.push_str("  \\(nothing\\)\n");
        }
        for (id, settings) in pr_issues {
            result.push_str(&format!(
                "  \\- `{id}`\n    {}\n",
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

        result.push('\n');
    }
    if result.is_empty() {
        result.push_str("\\(nothing\\)\n");
    }
    reply_to_msg(&bot, msg, result)
        .parse_mode(ParseMode::MarkdownV2)
        .disable_link_preview(true)
        .await?;

    Ok(())
}

async fn return_chat_id(bot: Bot, msg: Message) -> Result<(), CommandError> {
    reply_to_msg(&bot, &msg, format!("{}", msg.chat.id)).await?;
    Ok(())
}

async fn repo_add(bot: Bot, msg: Message, name: String, url: String) -> Result<(), CommandError> {
    ensure_admin_chat(&msg)?;
    let _output = repo::create(&name, &url).await?;
    let resources = repo::resources(&name).await?;
    let github_info = Url::parse(&url)
        .ok()
        .and_then(|u| GitHubInfo::parse_from_url(u).ok());

    let settings = {
        let mut locked = resources.settings.write().await;
        if let Some(info) = &github_info {
            let repository = octocrab::instance()
                .repos(&info.owner, &info.repo)
                .get()
                .await
                .map_err(|e| Error::Octocrab(Box::new(e)))?;
            if let Some(default_branch) = repository.default_branch {
                let default_regex_str = format!("^({})$", regex::escape(&default_branch));
                let default_regex = Regex::new(&default_regex_str).map_err(Error::from)?;
                let default_condition = ConditionSettings {
                    condition: GeneralCondition::InBranch(InBranchCondition {
                        branch_regex: default_regex.clone(),
                    }),
                };
                locked.branch_regex = default_regex;
                locked.conditions = {
                    let mut map = BTreeMap::new();
                    map.insert(format!("in-{default_branch}"), default_condition);
                    map
                };
            }
        }
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
    ensure_admin_chat(&msg)?;
    let resources = repo::resources(&name).await?;
    let new_settings = {
        let mut locked = resources.settings.write().await;
        if let Some(r) = branch_regex {
            let regex = Regex::new(&format!("^({r})$")).map_err(Error::from)?;
            locked.branch_regex = regex;
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
    ensure_admin_chat(&msg)?;
    repo::remove(&name).await?;
    reply_to_msg(&bot, &msg, format!("repository '{name}' removed")).await?;
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
    ensure_admin_chat(&msg)?;
    let resources = repo::resources(&repo).await?;
    let settings = ConditionSettings {
        condition: GeneralCondition::parse(kind, &expr)?,
    };
    repo::condition_add(&resources, &identifier, settings).await?;
    reply_to_msg(&bot, &msg, format!("condition {identifier} added")).await?;
    Ok(())
}

async fn condition_remove(
    bot: Bot,
    msg: Message,
    repo: String,
    identifier: String,
) -> Result<(), CommandError> {
    ensure_admin_chat(&msg)?;
    let resources = repo::resources(&repo).await?;
    repo::condition_remove(&resources, &identifier).await?;
    reply_to_msg(&bot, &msg, format!("condition {identifier} removed")).await?;
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
    let resources = chat::resources_msg_repo(&msg, repo.clone()).await?;
    let _guard = resources.commit_lock(hash.clone()).await;
    let subscribers = subscriber_from_msg(&msg).into_iter().collect();
    let settings = CommitSettings {
        url,
        notify: NotifySettings {
            comment,
            subscribers,
        },
    };
    match chat::commit_add(&resources, &hash, settings).await {
        Ok(()) => {
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
    let resources = chat::resources_msg_repo(&msg, repo.clone()).await?;
    let _guard = resources.commit_lock(hash.clone()).await;
    chat::commit_remove(&resources, &hash).await?;
    reply_to_msg(&bot, &msg, format!("commit {hash} removed")).await?;
    Ok(())
}

async fn commit_check(
    bot: Bot,
    msg: Message,
    repo: String,
    hash: String,
) -> Result<(), CommandError> {
    let resources = chat::resources_msg_repo(&msg, repo.clone()).await?;
    let _guard = resources.commit_lock(hash.clone()).await;
    let repo_resources = repo::resources(&repo).await?;
    let commit_settings = {
        let settings = resources.settings.read().await;
        settings
            .commits
            .get(&hash)
            .ok_or_else(|| Error::UnknownCommit(hash.clone()))?
            .clone()
    };
    let result = chat::commit_check(&resources, &repo_resources, &hash).await?;
    let reply = commit_check_message(&repo, &hash, &commit_settings, &result);
    let mut send = reply_to_msg(&bot, &msg, reply)
        .parse_mode(ParseMode::MarkdownV2)
        .disable_link_preview(true);
    let remove_conditions: BTreeSet<&String> = result.conditions_of_action(Action::Remove);
    if remove_conditions.is_empty() {
        send = try_attach_subscribe_button_markup(msg.chat.id, send, "c", &repo, &hash);
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
    let resources = chat::resources_msg_repo(&msg, repo.clone()).await?;
    let _guard = resources.commit_lock(hash.clone()).await;
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

async fn pr_issue_add(
    bot: Bot,
    msg: Message,
    repo_or_url: String,
    optional_id: Option<u64>,
    optional_comment: Option<String>,
) -> Result<(), CommandError> {
    let (repo, id) = resolve_pr_repo_or_url(repo_or_url, optional_id).await?;
    let resources = chat::resources_msg_repo(&msg, repo.clone()).await?;
    let repo_resources = repo::resources(&repo).await?;
    let url = pr_issue_url(&repo_resources, id).await?;
    let subscribers = subscriber_from_msg(&msg).into_iter().collect();
    let comment = optional_comment.unwrap_or_default();
    let settings = PRIssueSettings {
        url,
        notify: NotifySettings {
            comment,
            subscribers,
        },
    };
    match chat::pr_issue_add(&resources, &repo_resources, id, settings).await {
        Ok(()) => {
            pr_issue_check(bot, msg, repo, id).await?;
        }
        Err(Error::PullRequestExists(_)) => {
            pr_issue_subscribe(bot.clone(), msg.clone(), repo.clone(), id, false).await?;
        }
        Err(e) => return Err(e.into()),
    };
    Ok(())
}

async fn resolve_pr_repo_or_url(
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

async fn pr_issue_check(bot: Bot, msg: Message, repo: String, id: u64) -> Result<(), CommandError> {
    let resources = chat::resources_msg_repo(&msg, repo.clone()).await?;
    let repo_resources = repo::resources(&repo).await?;
    match chat::pr_issue_check(&resources, &repo_resources, id).await {
        Ok(result) => {
            let pretty_id = pr_issue_id_pretty(&repo_resources, id).await?;
            match result {
                PRIssueCheckResult::Merged(commit) => {
                    reply_to_msg(
                        &bot,
                        &msg,
                        format!(
                            "{pretty_id} has been merged \\(and removed\\)\ncommit `{commit}` added"
                        ),
                    )
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
                    commit_check(bot, msg, repo, commit).await
                }
                PRIssueCheckResult::Closed => {
                    reply_to_msg(
                        &bot,
                        &msg,
                        format!("{pretty_id} has been closed \\(and removed\\)"),
                    )
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
                    Ok(())
                }
                PRIssueCheckResult::Waiting => {
                    let mut send = reply_to_msg(
                        &bot,
                        &msg,
                        format!("{pretty_id} has not been merged/closed yet"),
                    )
                    .parse_mode(ParseMode::MarkdownV2);
                    send = try_attach_subscribe_button_markup(
                        msg.chat.id,
                        send,
                        "p",
                        &repo,
                        &id.to_string(),
                    );
                    send.await?;
                    Ok(())
                }
            }
        }
        Err(Error::CommitExists(commit)) => commit_subscribe(bot, msg, repo, commit, false).await,
        Err(e) => Err(e.into()),
    }
}

async fn pr_issue_remove(
    bot: Bot,
    msg: Message,
    repo: String,
    id: u64,
) -> Result<(), CommandError> {
    let resources = chat::resources_msg_repo(&msg, repo.clone()).await?;
    let repo_resources = repo::resources(&repo).await?;
    chat::pr_issue_remove(&resources, id).await?;
    let pretty_id = pr_issue_id_pretty(&repo_resources, id).await?;
    reply_to_msg(&bot, &msg, format!("{pretty_id} removed")).await?;
    Ok(())
}

async fn pr_issue_subscribe(
    bot: Bot,
    msg: Message,
    repo: String,
    pr_id: u64,
    unsubscribe: bool,
) -> Result<(), CommandError> {
    let resources = chat::resources_msg_repo(&msg, repo).await?;
    let subscriber = subscriber_from_msg(&msg).ok_or(Error::NoSubscriber)?;
    {
        let mut settings = resources.settings.write().await;
        let subscribers = &mut settings
            .pr_issues
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
    let resources = chat::resources_msg_repo(&msg, repo.clone()).await?;
    let _guard = resources.branch_lock(branch.clone()).await;
    let settings = BranchSettings {
        notify: Default::default(),
    };
    match chat::branch_add(&resources, &branch, settings).await {
        Ok(()) => branch_check(bot, msg, repo, branch).await,
        Err(Error::BranchExists(_)) => branch_subscribe(bot, msg, repo, branch, false).await,
        Err(e) => Err(e.into()),
    }
}

async fn branch_remove(
    bot: Bot,
    msg: Message,
    repo: String,
    branch: String,
) -> Result<(), CommandError> {
    let resources = chat::resources_msg_repo(&msg, repo).await?;
    let _guard = resources.branch_lock(branch.clone()).await;
    chat::branch_remove(&resources, &branch).await?;
    reply_to_msg(&bot, &msg, format!("branch {branch} removed")).await?;
    Ok(())
}

async fn branch_check(
    bot: Bot,
    msg: Message,
    repo: String,
    branch: String,
) -> Result<(), CommandError> {
    let resources = chat::resources_msg_repo(&msg, repo.clone()).await?;
    let _guard = resources.branch_lock(branch.clone()).await;
    let repo_resources = repo::resources(&repo).await?;
    let branch_settings = {
        let settings = resources.settings.read().await;
        settings
            .branches
            .get(&branch)
            .ok_or_else(|| Error::UnknownBranch(branch.clone()))?
            .clone()
    };
    let result = chat::branch_check(&resources, &repo_resources, &branch).await?;
    let reply = branch_check_message(&repo, &branch, &branch_settings, &result);

    let mut send = reply_to_msg(&bot, &msg, reply).parse_mode(ParseMode::MarkdownV2);
    send = try_attach_subscribe_button_markup(msg.chat.id, send, "b", &repo, &branch);
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
    let resources = chat::resources_msg_repo(&msg, repo).await?;
    let _guard = resources.branch_lock(branch.clone()).await;
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

fn ensure_admin_chat(msg: &Message) -> Result<(), CommandError> {
    let options = options::get();
    if msg.chat_id().map(|id| id.0) == Some(options.admin_chat_id) {
        Ok(())
    } else {
        Err(Error::NotAdminChat.into())
    }
}

fn try_attach_subscribe_button_markup(
    chat: ChatId,
    send: JsonRequest<SendMessage>,
    kind: &str,
    repo: &str,
    id: &str,
) -> JsonRequest<SendMessage> {
    match subscribe_button_markup(kind, repo, id) {
        Ok(m) => send.reply_markup(m),
        Err(e) => {
            log::error!("failed to create markup for ({chat}, {repo}, {id}): {e}");
            send
        }
    }
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
    let subscribe_len = subscribe_data.len();
    if subscribe_len > 64 {
        return Err(Error::SubscribeTermSizeExceeded(
            subscribe_len,
            subscribe_data,
        ));
    }
    let unsubscribe_len = unsubscribe_data.len();
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
