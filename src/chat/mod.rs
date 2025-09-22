use std::sync::Arc;

use teloxide::types::{ChatId, Message};

use crate::{
    chat::{resources::ChatRepoResources, results::CommitCheckResult, settings::CommitSettings},
    error::Error,
    repo::resources::RepoResources,
};

pub mod paths;
pub mod resources;
pub mod results;
pub mod settings;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Task {
    chat: ChatId,
    repo: String,
}

pub async fn resources(task: &Task) -> Result<Arc<ChatRepoResources>, Error> {
    Ok(resources::RESOURCES_MAP.get(task).await?)
}

pub async fn resources_chat_repo(
    chat: ChatId,
    repo: String,
) -> Result<Arc<ChatRepoResources>, Error> {
    let task = Task { chat, repo };
    resources(&task).await
}

pub async fn resources_msg_repo(
    msg: &Message,
    repo: String,
) -> Result<Arc<ChatRepoResources>, Error> {
    let chat = msg.chat.id;
    resources_chat_repo(chat, repo).await
}

pub async fn commit_add(
    resources: &ChatRepoResources,
    hash: &str,
    settings: CommitSettings,
) -> Result<(), Error> {
    let _guard = resources.commit_lock(hash.to_string()).await;
    {
        let mut locked = resources.settings.write().await;
        if locked.commits.contains_key(hash) {
            return Err(Error::CommitExists(hash.to_owned()));
        }
        locked.commits.insert(hash.to_owned(), settings);
    }
    resources.save_settings().await;
    Ok(())
}

pub async fn commit_remove(resources: &ChatRepoResources, hash: &str) -> Result<(), Error> {
    let _guard = resources.commit_lock(hash.to_string()).await;
    {
        let mut settings = resources.settings.write().await;
        if !settings.commits.contains_key(hash) {
            return Err(Error::UnknownCommit(hash.to_owned()));
        }
        settings.commits.remove(hash);
    }
    {
        let mut results = resources.results.write().await;
        results.commits.remove(hash);
    }
    resources.save_settings().await;
    resources.save_results().await;
    Ok(())
}

pub(crate) async fn commit_check(
    resources: Arc<ChatRepoResources>,
    repo_resources: &RepoResources,
    hash: &str,
) -> Result<CommitCheckResult, Error> {
    todo!()
}
