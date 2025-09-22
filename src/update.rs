use std::collections::BTreeSet;

use teloxide::{
    Bot,
    payloads::SendMessageSetters,
    prelude::Requester,
    sugar::request::RequestLinkPreviewExt,
    types::{ChatId, ParseMode},
};

use crate::{
    CommandError,
    chat::{
        self,
        resources::ChatRepoResources,
        settings::{BranchSettings, CommitSettings, PullRequestSettings},
    },
    condition::Action,
    message::{branch_check_message, commit_check_message, pr_merged_message},
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
    let repo_resources = repo::resources(repo).await?;

    // check pull requests before checking commits
    let pull_requests = {
        let settings = resources.settings.read().await;
        settings.pull_requests.clone()
    };
    for (pr, settings) in pull_requests {
        if let Err(e) = update_chat_repo_pr(
            bot.clone(),
            &resources,
            &repo_resources,
            chat,
            repo,
            pr,
            &settings,
        )
        .await
        {
            log::error!("update error for PR ({chat}, {repo}, {pr}): {e}");
        }
    }

    // check branches of the repo
    let branches = {
        let settings = resources.settings.read().await;
        settings.branches.clone()
    };
    for (branch, settings) in branches {
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

async fn update_chat_repo_pr(
    bot: Bot,
    resources: &ChatRepoResources,
    repo_resources: &RepoResources,
    chat: ChatId,
    repo: &str,
    pr: u64,
    settings: &PullRequestSettings,
) -> Result<(), CommandError> {
    let result = chat::pr_check(resources, repo_resources, pr).await?;
    log::info!("finished pr check ({chat}, {repo}, {pr})");
    if let Some(commit) = result {
        let message = pr_merged_message(repo, pr, settings, &commit);
        bot.send_message(chat, message)
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
    }
    Ok(())
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
    // check again commit existence after acquiring the lock
    {
        let _guard = resources.commit_lock(commit.to_string()).await;
        let settings = resources.settings.read().await;
        if !settings.commits.contains_key(commit) {
            return Ok(())
        }
    }

    let result = chat::commit_check(resources, repo_resources, commit).await?;
    log::info!("finished commit check ({chat}, {repo}, {commit})");
    if !result.new.is_empty() {
        let suppress_notification_conditions: BTreeSet<&String> =
            result.conditions_of_action(Action::SuppressNotification);
        if !suppress_notification_conditions.is_empty() {
            log::info!("suppress notification for check result of ({chat}, {repo}): {result:?}",);
        } else {
            let message = commit_check_message(repo, commit, settings, &result);
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
    // check again commit existence after acquiring the lock
    {
        let _guard = resources.branch_lock(branch.to_string()).await;
        let settings = resources.settings.read().await;
        if !settings.branches.contains_key(branch) {
            return Ok(())
        }
    }

    let result = chat::branch_check(resources, repo_resources, branch).await?;
    log::info!("finished branch check ({chat}, {repo}, {branch})");
    if result.new != result.old {
        let message = branch_check_message(repo, branch, settings, &result);
        let mut send = bot
            .send_message(chat, message)
            .parse_mode(ParseMode::MarkdownV2);
        send = try_attach_subscribe_button_markup(chat, send, "b", repo, branch);
        send.await?;
    }
    Ok(())
}
