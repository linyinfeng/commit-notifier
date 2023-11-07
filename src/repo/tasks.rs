use std::collections::BTreeMap;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use deadpool_sqlite::Pool;
use fs4::FileExt;
use git2::Repository;
use lockable::LockPool;
use once_cell::sync::Lazy;
use serde::de::DeserializeOwned;
use serde::Serialize;
use teloxide::types::ChatId;
use tokio::sync::{Mutex, RwLock};
use tokio::time::sleep;

use super::paths::{self, Paths};
use super::results::Results;
use super::settings::Settings;
use crate::cache;
use crate::error::Error;

#[derive(Default)]
pub struct ResourcesMap {
    pub map: Lazy<Mutex<BTreeMap<Task, Arc<Resources>>>>,
}

pub static RESOURCES_MAP: ResourcesMap = ResourcesMap {
    map: Lazy::new(|| Mutex::new(Default::default())),
};

impl ResourcesMap {
    pub async fn get(task: &Task) -> Result<Arc<Resources>, Error> {
        let mut map = RESOURCES_MAP.map.lock().await;
        match map.get(task) {
            Some(resources) => Ok(resources.clone()),
            None => {
                let resources = Arc::new(Resources::open(task).await?);
                map.insert(task.clone(), resources.clone());
                Ok(resources)
            }
        }
    }

    pub async fn remove<F>(task: &Task, cleanup: F) -> Result<(), Error>
    where
        F: FnOnce() -> Result<(), Error>,
    {
        let mut map = RESOURCES_MAP.map.lock().await;
        if let Some(arc) = map.remove(task) {
            wait_for_resources_drop(task, arc).await;
            cleanup()?; // run before the map unlock
            Ok(())
        } else {
            Err(Error::UnknownRepository(task.repo.clone()))
        }
    }

    pub async fn clear() -> Result<(), Error> {
        let mut map = RESOURCES_MAP.map.lock().await;
        while let Some((task, resources)) = map.pop_first() {
            wait_for_resources_drop(&task, resources).await;
        }
        Ok(())
    }
}

pub async fn wait_for_resources_drop(task: &Task, mut arc: Arc<Resources>) {
    loop {
        match Arc::try_unwrap(arc) {
            Ok(_resource) => {
                // do nothing
                // just drop
                break;
            }
            Err(a) => {
                arc = a;
                log::info!(
                    "removing {}/{}, waiting for existing jobs",
                    task.chat,
                    task.repo
                );
                sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug)]
pub struct Task {
    pub chat: ChatId,
    pub repo: String,
}

impl Task {
    pub fn paths(&self) -> Result<Paths, Error> {
        paths::get(self.chat, &self.repo)
    }
}

pub struct Resources {
    pub task: Task,
    pub paths: Paths,
    pub repo: Mutex<Repository>,
    pub cache: Pool,
    pub settings: RwLock<Settings>,
    pub results: RwLock<Results>,

    pub commit_locks: LockPool<String>,
    pub branch_locks: LockPool<String>,
}

impl Resources {
    pub async fn open(task: &Task) -> Result<Self, Error> {
        let paths = task.paths()?;

        if !paths.outer.is_dir() {
            return Err(Error::UnknownRepository(task.repo.clone()));
        }

        // load repo
        let repo = Mutex::new(Repository::open(&paths.repo)?);
        // load cache
        let cache_exists = paths.cache.is_file();
        let cache_cfg = deadpool_sqlite::Config::new(&paths.cache);
        let cache = cache_cfg.create_pool(deadpool_sqlite::Runtime::Tokio1)?;
        if !cache_exists {
            let conn = cache.get().await?;
            conn.interact(|c| cache::initialize(c))
                .await
                .map_err(|e| Error::DBInteract(Mutex::new(e)))??;
        }
        // load settings
        let settings = RwLock::new(read_json(&paths.settings)?);
        // load results
        let results = RwLock::new(read_json(&paths.results)?);

        Ok(Resources {
            task: task.clone(),
            paths,
            repo,
            cache,
            settings,
            results,
            commit_locks: LockPool::new(),
            branch_locks: LockPool::new(),
        })
    }

    pub async fn save_settings(&self) -> Result<(), Error> {
        let paths = self.task.paths()?;
        let in_mem = self.settings.read().await;
        write_json(&paths.settings, &*in_mem)
    }

    pub async fn save_results(&self) -> Result<(), Error> {
        let paths = self.task.paths()?;
        let in_mem = self.results.read().await;
        write_json(&paths.results, &*in_mem)
    }

    pub async fn cache(&self) -> Result<deadpool_sqlite::Object, Error> {
        Ok(self.cache.get().await?)
    }

    pub async fn commit_lock(&self, key: String) -> impl Drop + '_ {
        self.commit_locks.async_lock(key).await
    }

    pub async fn branch_lock(&self, key: String) -> impl Drop + '_ {
        self.branch_locks.async_lock(key).await
    }
}

fn read_json<P, T>(path: P) -> Result<T, Error>
where
    P: AsRef<Path> + fmt::Debug,
    T: Serialize + DeserializeOwned + Default,
{
    if !path.as_ref().is_file() {
        log::info!("auto create file: {:?}", path);
        write_json::<_, T>(&path, &Default::default())?;
    }
    log::debug!("read from file: {:?}", path);
    let file = File::open(path)?;
    file.lock_shared()?; // close of file automatically release the lock
    let reader = BufReader::new(file);
    Ok(serde_json::from_reader(reader)?)
}

fn write_json<P, T>(path: P, rs: &T) -> Result<(), Error>
where
    P: AsRef<Path> + fmt::Debug,
    T: Serialize,
{
    log::debug!("write to file: {:?}", path);
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    file.lock_exclusive()?;
    let writer = BufWriter::new(file);
    Ok(serde_json::to_writer_pretty(writer, rs)?)
}
