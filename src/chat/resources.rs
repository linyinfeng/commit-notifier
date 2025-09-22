use std::sync::LazyLock;

use lockable::LockPool;
use tokio::{fs::create_dir_all, sync::RwLock};

use crate::{
    chat::{Task, paths::ChatRepoPaths, results::ChatRepoResults, settings::ChatRepoSettings},
    error::Error,
    resources::{Resource, ResourcesMap},
    utils::{read_json, write_json},
};

pub static RESOURCES_MAP: LazyLock<ResourcesMap<Task, ChatRepoResources>> =
    LazyLock::new(ResourcesMap::new);

pub struct ChatRepoResources {
    pub task: Task,
    pub paths: ChatRepoPaths,
    pub settings: RwLock<ChatRepoSettings>,
    pub results: RwLock<ChatRepoResults>,

    pub commit_locks: LockPool<String>,
    pub branch_locks: LockPool<String>,
}

impl Resource<Task> for ChatRepoResources {
    async fn open(task: &Task) -> Result<Self, Error> {
        let paths = ChatRepoPaths::new(task);
        if !paths.repo.is_dir() {
            create_dir_all(&paths.repo).await?;
        }
        let settings = RwLock::new(read_json(&paths.settings)?);
        let results = RwLock::new(read_json(&paths.results)?);
        Ok(Self {
            task: task.clone(),
            paths,
            settings,
            results,
            commit_locks: LockPool::new(),
            branch_locks: LockPool::new(),
        })
    }
}

impl ChatRepoResources {
    pub async fn save_settings(&self) -> Result<(), Error> {
        let in_mem = self.settings.read().await;
        write_json(&self.paths.settings, &*in_mem)
    }
    pub async fn save_results(&self) -> Result<(), Error> {
        let in_mem = self.results.read().await;
        write_json(&self.paths.results, &*in_mem)
    }
    pub async fn commit_lock(&self, key: String) -> impl Drop + '_ {
        self.commit_locks.async_lock(key).await
    }

    pub async fn branch_lock(&self, key: String) -> impl Drop + '_ {
        self.branch_locks.async_lock(key).await
    }
}
