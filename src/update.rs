use std::collections::BTreeSet;

use teloxide::{
    ApiError, Bot, RequestError,
    payloads::SendMessageSetters,
    prelude::Requester,
    sugar::request::RequestLinkPreviewExt,
    types::{Banned, ChatId, ChatMemberKind, ParseMode, UntilDate, UserId},
};

use crate::{
    CommandError,
    chat::{
        self,
        resources::ChatRepoResources,
        results::PRIssueCheckResult,
        settings::{BranchSettings, CommitSettings, PRIssueSettings, Subscriber},
    },
    condition::Action,
    message::{
        branch_check_message, commit_check_message, pr_issue_closed_message,
        pr_issue_merged_message, pr_issue_opened_message,
    },
    options,
    repo::{self, resources::RepoResources},
    try_attach_subscribe_button_markup,
};

pub async fn update_and_report_error(bot: Bot) -> Result<(), teloxide::RequestError> {
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
    let repos = repo::list().await?;
    for repo in repos {
        log::info!("updating repository {repo}...");
        let resources = repo::resources(&repo).await?;
        log::info!("updating {repo}...");
        if let Err(e) = repo::fetch_and_update_cache(resources).await {
            log::error!("update error for repository {repo}: {e}");
        }
    }
    log::info!("updating chats...");
    let chats = chat::chats().await?;
    for chat in chats {
        if let Err(e) = update_chat(bot.clone(), chat).await {
            log::error!("update error for chat {chat}: {e}");
        }
    }
    Ok(())
}

async fn update_chat(bot: Bot, chat: ChatId) -> Result<(), CommandError> {
    let repos = chat::repos(chat).await?;
    for repo in repos {
        log::info!("updating repository of chat ({chat}, {repo})...");
        if let Err(e) = update_chat_repo(bot.clone(), chat, &repo).await {
            log::error!("update error for repository of chat ({chat}, {repo}): {e}");
        }
    }
    Ok(())
}

async fn update_chat_repo(bot: Bot, chat: ChatId, repo: &str) -> Result<(), CommandError> {
    log::info!("updating ({chat}, {repo})...");
    let resources = chat::resources_chat_repo(chat, repo.to_string()).await?;
    sweep_subscribers(&bot, &resources, chat).await?;
    let repo_resources = repo::resources(repo).await?;

    // check pull requests/issues before checking commits
    let pr_issues = {
        let settings = resources.settings.read().await;
        settings.pr_issues.clone()
    };
    for (id, settings) in pr_issues {
        if let Err(e) = update_chat_repo_pr_issue(
            bot.clone(),
            &resources,
            &repo_resources,
            chat,
            repo,
            id,
            &settings,
        )
        .await
        {
            log::error!("update error for PR ({chat}, {repo}, {id}): {e}");
        }
    }

    // check branches of the repo
    let branches = {
        let settings = resources.settings.read().await;
        settings.branches.clone()
    };
    for (branch, settings) in branches {
        let _guard = resources.branch_lock(branch.clone()).await;
        if let Err(e) = update_chat_repo_branch(
            bot.clone(),
            &resources,
            &repo_resources,
            chat,
            repo,
            &branch,
            &settings,
        )
        .await
        {
            log::error!("update error for branch ({chat}, {repo}, {branch}): {e}");
        }
    }

    // check commits of the repo
    let commits = {
        let settings = resources.settings.read().await;
        settings.commits.clone()
    };
    for (commit, settings) in commits {
        let _guard = resources.commit_lock(commit.clone()).await;
        if let Err(e) = update_chat_repo_commit(
            bot.clone(),
            &resources,
            &repo_resources,
            chat,
            repo,
            &commit,
            &settings,
        )
        .await
        {
            log::error!("update error for commit ({chat}, {repo}, {commit}): {e}");
        }
    }
    Ok(())
}

async fn update_chat_repo_pr_issue(
    bot: Bot,
    resources: &ChatRepoResources,
    repo_resources: &RepoResources,
    chat: ChatId,
    repo: &str,
    id: u64,
    settings: &PRIssueSettings,
) -> Result<(), CommandError> {
    let result = chat::pr_issue_check(resources, repo_resources, id).await?;
    log::info!("finished PR/issue check ({chat}, {repo}, {id})");
    match result {
        PRIssueCheckResult::Merged(commit) => {
            let message = pr_issue_merged_message(repo_resources, id, settings, &commit).await?;
            bot.send_message(chat, message)
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
            Ok(())
        }
        PRIssueCheckResult::Opened => {
            let message = pr_issue_opened_message(repo_resources, id, settings).await?;
            bot.send_message(chat, message)
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
            Ok(())
        }
        PRIssueCheckResult::Closed => {
            let message = pr_issue_closed_message(repo_resources, id, settings).await?;
            bot.send_message(chat, message)
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
            Ok(())
        }
        PRIssueCheckResult::Waiting => Ok(()),
        PRIssueCheckResult::Expiring => Ok(()),
        PRIssueCheckResult::Expired => {
            log::info!("stopped tracking to pr/issue {id}");
            Ok(())
        }
        PRIssueCheckResult::Unknown => {
            log::warn!("issue {id} state unknown");
            Ok(())
        }
    }
}

async fn update_chat_repo_commit(
    bot: Bot,
    resources: &ChatRepoResources,
    repo_resources: &RepoResources,
    chat: ChatId,
    repo: &str,
    commit: &str,
    settings: &CommitSettings,
) -> Result<(), CommandError> {
    let result = chat::commit_check(resources, repo_resources, commit).await?;
    log::info!("finished commit check ({chat}, {repo}, {commit})");
    if !result.new.is_empty() {
        let suppress_notification_conditions: BTreeSet<&String> =
            result.conditions_of_action(Action::SuppressNotification);
        if !suppress_notification_conditions.is_empty() {
            log::info!("suppress notification for check result of ({chat}, {repo}): {result:?}",);
        } else {
            // mention in update
            let message = commit_check_message(repo, commit, settings, &result, true);
            let mut send = bot
                .send_message(chat, message)
                .parse_mode(ParseMode::MarkdownV2)
                .disable_link_preview(true);
            let remove_conditions: BTreeSet<&String> = result.conditions_of_action(Action::Remove);
            if remove_conditions.is_empty() {
                send = try_attach_subscribe_button_markup(chat, send, "c", repo, commit);
            }
            send.await?;
        }
    }
    Ok(())
}

async fn update_chat_repo_branch(
    bot: Bot,
    resources: &ChatRepoResources,
    repo_resources: &RepoResources,
    chat: ChatId,
    repo: &str,
    branch: &str,
    settings: &BranchSettings,
) -> Result<(), CommandError> {
    let result = chat::branch_check(resources, repo_resources, branch).await?;
    log::info!("finished branch check ({chat}, {repo}, {branch})");
    if result.new != result.old {
        let message = {
            let repo_settings = repo_resources.settings.read().await;
            branch_check_message(
                repo,
                branch,
                settings,
                &result,
                repo_settings.github_info.as_ref(),
            )
        };
        let mut send = bot
            .send_message(chat, message)
            .parse_mode(ParseMode::MarkdownV2)
            .disable_link_preview(true);
        send = try_attach_subscribe_button_markup(chat, send, "b", repo, branch);
        send.await?;
    }
    Ok(())
}

/// Remove subscribers that are not members of the chat (left, banned,
/// removed, or account deleted on Telegram). Also refresh stale mentions
/// from the current user data.
async fn sweep_subscribers(
    bot: &Bot,
    resources: &ChatRepoResources,
    chat: ChatId,
) -> Result<(), CommandError> {
    let mut subscribers = BTreeSet::new();
    {
        let settings = resources.settings.read().await;
        for s in settings.pr_issues.values() {
            subscribers.extend(s.notify.subscribers.iter().cloned());
        }
        for s in settings.branches.values() {
            subscribers.extend(s.notify.subscribers.iter().cloned());
        }
        for s in settings.commits.values() {
            subscribers.extend(s.notify.subscribers.iter().cloned());
        }
    }
    let mut invalid = BTreeSet::new();
    let mut renamed = Vec::new(); // (old subscriber, fresh subscriber with updated mention/user id)
    for subscriber in subscribers {
        let Subscriber::Telegram { user_id, .. } = &subscriber;
        let member = match bot.get_chat_member(chat, UserId(*user_id)).await {
            Ok(member) => member,
            Err(e) if subscriber_unresolvable(&e) => {
                log::info!("remove subscriber {subscriber:?} from chat {chat}: {e}");
                invalid.insert(subscriber);
                continue;
            }
            Err(e) => {
                log::warn!("can not check subscriber {subscriber:?} in chat {chat}: {e}");
                continue; // keep on other api errors
            }
        };
        if !is_member(&member.kind) {
            log::info!("remove subscriber {subscriber:?} from chat {chat}");
            invalid.insert(subscriber);
            continue;
        }
        // update stale mention (user renamed) and persist the change
        let fresh = Subscriber::from_tg_user(&member.user);
        if fresh != subscriber {
            log::info!("update subscriber {subscriber:?} to {fresh:?} in chat {chat}");
            renamed.push((subscriber, fresh));
        }
    }
    if invalid.is_empty() && renamed.is_empty() {
        return Ok(());
    }
    {
        let mut settings = resources.settings.write().await;
        for s in settings.pr_issues.values_mut() {
            update_subscribers(&mut s.notify.subscribers, &invalid, &renamed);
        }
        for s in settings.branches.values_mut() {
            update_subscribers(&mut s.notify.subscribers, &invalid, &renamed);
        }
        for s in settings.commits.values_mut() {
            update_subscribers(&mut s.notify.subscribers, &invalid, &renamed);
        }
    }
    resources.save_settings().await?;
    Ok(())
}

/// Whether `get_chat_member` cannot resolve the subscriber as a participant of
/// the chat at all:
///
/// - `user not found` for an id Telegram no longer knows, and
/// - `PARTICIPANT_ID_INVALID`, documented as "the specified participant ID is
///   invalid" for `channels.getParticipant`, the method behind `getChatMember`.
///   teloxide has no variant for it, so it arrives as `Unknown`, and it does not
///   prove the account is gone: it is also reported for ids the bot cannot
///   resolve at all (tdlib/telegram-bot-api#802).
///
/// Either way the id cannot be mentioned in the chat, so keeping the
/// subscription changes nothing; drop it instead of retrying on every update.
fn subscriber_unresolvable(e: &RequestError) -> bool {
    match e {
        RequestError::Api(ApiError::UserNotFound) => true,
        RequestError::Api(ApiError::Unknown(text)) => text.contains("PARTICIPANT_ID_INVALID"),
        _ => false,
    }
}

/// Remove invalid subscribers and replace renamed ones (mention changed).
fn update_subscribers(
    subscribers: &mut BTreeSet<Subscriber>,
    invalid: &BTreeSet<Subscriber>,
    renamed: &[(Subscriber, Subscriber)],
) {
    subscribers.retain(|s| !invalid.contains(s));
    for (old, new) in renamed {
        if subscribers.remove(old) {
            subscribers.insert(new.clone());
        }
    }
}

/// Whether the member can receive messages in the chat: owner, admin,
/// regular member, or a restricted member still in the chat. Being muted
/// (restricted, can't send) does not prevent receiving mentions. A
/// temporary ban lifts automatically, so it does not remove the subscriber
/// either; only a permanent ban (or left/removed) does.
fn is_member(kind: &ChatMemberKind) -> bool {
    match kind {
        ChatMemberKind::Owner(_) | ChatMemberKind::Administrator(_) | ChatMemberKind::Member(_) => {
            true
        }
        ChatMemberKind::Restricted(r) => r.is_member,
        ChatMemberKind::Left => false,
        ChatMemberKind::Banned(Banned {
            until_date: UntilDate::Date(_),
        }) => true,
        ChatMemberKind::Banned(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use crate::utils::tg_user_link_id;

    use super::{Subscriber, is_member, update_subscribers};
    use teloxide::types::{Banned, ChatMemberKind, Member, UntilDate};

    fn restricted(is_member: bool, can_send_messages: bool) -> ChatMemberKind {
        serde_json::from_str(&format!(
            r#"{{"status": "restricted", "until_date": 0, "is_member": {is_member}, "can_send_messages": {can_send_messages}, "can_send_audios": true, "can_send_documents": true, "can_send_photos": true, "can_send_videos": true, "can_send_video_notes": true, "can_send_voice_notes": true, "can_send_other_messages": true, "can_add_web_page_previews": true, "can_change_info": true, "can_invite_users": true, "can_pin_messages": true, "can_manage_topics": true, "can_send_polls": true}}"#
        ))
        .unwrap()
    }

    #[test]
    fn is_member_check() {
        assert!(is_member(&ChatMemberKind::Member(Member {
            until_date: None
        })));
        assert!(!is_member(&ChatMemberKind::Left));
        assert!(!is_member(&ChatMemberKind::Banned(Banned {
            until_date: UntilDate::Forever
        })));
        // temporary ban lifts automatically: keep
        assert!(is_member(&ChatMemberKind::Banned(Banned {
            until_date: UntilDate::Date(chrono::Utc::now())
        })));
        // muted but still a member: can receive mentions, keep
        assert!(is_member(&restricted(true, false)));
        assert!(is_member(&restricted(true, true)));
        assert!(!is_member(&restricted(false, true)));
    }

    fn subscriber(user_id: u64, mention: &str) -> Subscriber {
        Subscriber::Telegram {
            user_id,
            markdown_mention: mention.to_string(),
        }
    }

    #[test]
    fn update_subscribers_removes_invalid_and_renames() {
        let mut set = BTreeSet::from([
            subscriber(1, "@alice"),
            subscriber(2, "@bob"),
            subscriber(3, "@carol"),
        ]);
        let invalid = BTreeSet::from([subscriber(2, "@bob")]);
        let renamed = vec![(subscriber(3, "@carol"), subscriber(3, "@carol_new"))];
        update_subscribers(&mut set, &invalid, &renamed);
        assert_eq!(
            set,
            BTreeSet::from([subscriber(1, "@alice"), subscriber(3, "@carol_new")])
        );
    }

    #[test]
    fn tg_user_link_id_extracts() {
        assert_eq!(tg_user_link_id(r"[Foo](tg://user/?id=123)"), Some(123));
        assert_eq!(tg_user_link_id("@alice"), None);
        assert_eq!(tg_user_link_id(r"[Foo](tg://user/?id=abc)"), None);
    }
}
