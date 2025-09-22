mod chat;
mod command;
mod condition;
mod error;
mod github;
mod message;
mod options;
mod repo;
mod resources;
mod utils;

use std::env;
use std::str::FromStr;

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

use crate::chat::settings::CommitSettings;
use crate::chat::settings::NotifySettings;
use crate::message::commit_check_message;
use crate::message::subscriber_from_msg;
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
    log::info!("exit");
}

async fn answer(bot: Bot, msg: Message, bc: BCommand) -> ResponseResult<()> {
    log::debug!("message: {msg:?}");
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
                    pr,
                    comment,
                } => pr_add(bot, msg, repo_or_url, pr, comment).await,
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
    todo!()
    // log::debug!("query = {query:?}");
    // let (chat_id, username) = get_chat_id_and_username_from_query(query)?;
    // let subscriber = Subscriber::Telegram { username };
    // let _msg = query
    //     .message
    //     .as_ref()
    //     .ok_or(Error::SubscribeCallbackNoMsgId)?;
    // let data = query.data.as_ref().ok_or(Error::SubscribeCallbackNoData)?;
    // let SubscribeTerm(kind, repo, id, subscribe) =
    //     serde_json::from_str(data).map_err(Error::Serde)?;
    // let unsubscribe = subscribe == 0;
    // match kind.as_str() {
    //     "b" => {
    //         let resources = resources_helper_chat(chat_id, &repo).await?;
    //         {
    //             let mut settings = resources.settings.write().await;
    //             let subscribers = &mut settings
    //                 .branches
    //                 .get_mut(&id)
    //                 .ok_or_else(|| Error::UnknownBranch(id.clone()))?
    //                 .notify
    //                 .subscribers;
    //             modify_subscriber_set(subscribers, subscriber, unsubscribe)?;
    //         }
    //         resources.save_settings().await?;
    //     }
    //     "c" => {
    //         let resources = resources_helper_chat(chat_id, &repo).await?;
    //         {
    //             let mut settings = resources.settings.write().await;
    //             let subscribers = &mut settings
    //                 .commits
    //                 .get_mut(&id)
    //                 .ok_or_else(|| Error::UnknownCommit(id.clone()))?
    //                 .notify
    //                 .subscribers;
    //             modify_subscriber_set(subscribers, subscriber, unsubscribe)?;
    //         }
    //         resources.save_settings().await?;
    //     }
    //     "p" => {
    //         let pr_id: u64 = id.parse().map_err(Error::ParseInt)?;
    //         let resources = resources_helper_chat(chat_id, &repo).await?;
    //         {
    //             let mut settings = resources.settings.write().await;
    //             let subscribers = &mut settings
    //                 .pull_requests
    //                 .get_mut(&pr_id)
    //                 .ok_or_else(|| Error::UnknownPullRequest(pr_id))?
    //                 .notify
    //                 .subscribers;
    //             modify_subscriber_set(subscribers, subscriber, unsubscribe)?;
    //         }
    //         resources.save_settings().await?;
    //     }
    //     _ => Err(Error::SubscribeCallbackDataInvalidKind(kind))?,
    // }
    // if unsubscribe {
    //     Ok(format!("unsubscribed from {repo}/{id}"))
    // } else {
    //     Ok(format!("subscribed to {repo}/{id}"))
    // }
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
        if let Err(e) = update_and_report_error(bot.clone()).await {
            log::error!("teloxide error in update: {e}");
        }
        log::info!("finished update '{datetime}'");
    }
}

async fn update_and_report_error(bot: Bot) -> Result<(), teloxide::RequestError> {
    match update(bot.clone()).await {
        Ok(r) => Ok(r),
        Err(CommandError::Normal(e)) => {
            log::error!("update error: {e}");
            let options = options::get();
            bot.send_message(ChatId(options.admin_chat_id), format!("update error: {e}"))
                .await?;
            Ok(())
        }
        Err(CommandError::Teloxide(e)) => Err(e),
    }
}

async fn update(bot: Bot) -> Result<(), CommandError> {
    log::info!("updating repositories...");
    let repos = repo::list().await?;
    for repo in repos {
        let resources = repo::resources(&repo).await?;
        repo::fetch_and_update_cache(&resources).await?;
    }
    Ok(())
}

async fn list(bot: Bot, msg: Message) -> Result<(), CommandError> {
    let options = options::get();
    let chat = msg.chat.id;
    if ChatId(options.admin_chat_id) == chat {
        list_for_admin(bot, msg).await
    } else {
        list_for_normal(bot, msg).await
    }
}

async fn list_for_admin(bot: Bot, msg: Message) -> Result<(), CommandError> {
    let chat = msg.chat.id;
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
    reply_to_msg(&bot, &msg, result)
        .parse_mode(ParseMode::MarkdownV2)
        .await?;

    Ok(())
}

async fn list_for_normal(bot: Bot, msg: Message) -> Result<(), CommandError> {
    todo!()
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
    todo!()
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
            // ensure regex is valid
            let _: Regex = Regex::new(&format!("^{r}$")).map_err(Error::from)?;
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
    ensure_admin_chat(&msg)?;
    repo::remove(&name).await?;
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
    let resources = chat::resources_msg_repo(&msg, repo.clone()).await?;
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
    let repo_resources = repo::resources(&repo).await?;
    let commit_settings = {
        let settings = resources.settings.read().await;
        settings
            .commits
            .get(&hash)
            .ok_or_else(|| Error::UnknownCommit(hash.clone()))?
            .clone()
    };
    repo::fetch_and_update_cache(&repo_resources).await?;
    let result = chat::commit_check(resources, &repo_resources, &hash).await?;
    let reply = commit_check_message(&repo, &hash, &commit_settings, &result);
    let mut send = reply_to_msg(&bot, &msg, reply)
        .parse_mode(ParseMode::MarkdownV2)
        .disable_link_preview(true);
    if result.removed_by_condition.is_none() {
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
    todo!()
}

async fn pr_add(
    bot: Bot,
    msg: Message,
    repo_or_url: String,
    pr_id: Option<u64>,
    optional_comment: Option<String>,
) -> Result<(), CommandError> {
    todo!()
}

async fn resolve_pr_repo_or_url(
    chat: ChatId,
    repo_or_url: String,
    pr_id: Option<u64>,
) -> Result<(String, u64), Error> {
    todo!()
}

async fn pr_check(bot: Bot, msg: Message, repo: String, pr_id: u64) -> Result<(), CommandError> {
    todo!()
}

async fn pr_remove(bot: Bot, msg: Message, repo: String, pr_id: u64) -> Result<(), CommandError> {
    todo!()
}

async fn pr_subscribe(
    bot: Bot,
    msg: Message,
    repo: String,
    pr_id: u64,
    unsubscribe: bool,
) -> Result<(), CommandError> {
    todo!()
}

async fn branch_add(
    bot: Bot,
    msg: Message,
    repo: String,
    branch: String,
) -> Result<(), CommandError> {
    todo!()
}

async fn branch_remove(
    bot: Bot,
    msg: Message,
    repo: String,
    branch: String,
) -> Result<(), CommandError> {
    todo!()
}

async fn branch_check(
    bot: Bot,
    msg: Message,
    repo: String,
    branch: String,
) -> Result<(), CommandError> {
    todo!()
}

async fn branch_subscribe(
    bot: Bot,
    msg: Message,
    repo: String,
    branch: String,
    unsubscribe: bool,
) -> Result<(), CommandError> {
    todo!()
}

async fn condition_add(
    bot: Bot,
    msg: Message,
    repo: String,
    identifier: String,
    kind: condition::Kind,
    expr: String,
) -> Result<(), CommandError> {
    todo!()
}

async fn condition_remove(
    bot: Bot,
    msg: Message,
    repo: String,
    identifier: String,
) -> Result<(), CommandError> {
    todo!()
}

async fn condition_trigger(
    bot: Bot,
    msg: Message,
    repo: String,
    identifier: String,
) -> Result<(), CommandError> {
    todo!()
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
    hash: &str,
) -> JsonRequest<SendMessage> {
    match subscribe_button_markup(kind, &repo, &hash) {
        Ok(m) => send.reply_markup(m),
        Err(e) => {
            log::error!("failed to create markup for ({chat}, {repo}, {hash}): {e}");
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
