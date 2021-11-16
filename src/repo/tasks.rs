use std::collections::BTreeSet;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter};
use std::path::Path;
use std::sync::{Arc, Mutex};

use git2::Repository;
use rusqlite::Connection;
use serde::de::DeserializeOwned;
use serde::Serialize;

use super::paths;
use super::results::Results;
use super::settings::Settings;
use crate::cache;
use crate::error::Error;

static TASKS: once_cell::sync::Lazy<Mutex<BTreeSet<Task>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(BTreeSet::new()));

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug)]
pub struct Task {
    pub chat: i64,
    pub repo: String,
}

impl Task {
    pub fn lock_bare(self) -> Option<TaskGuardBare> {
        let mut running = TASKS.lock().unwrap();
        if running.contains(&self) {
            None
        } else {
            log::debug!("bare task locked: {:?}", self);
            running.insert(self.clone());
            Some(Arc::new(TaskGuardBareInner { task: self }))
        }
    }

    pub fn lock(self) -> Result<Option<TaskGuard>, Error> {
        let mut running = TASKS.lock().unwrap();
        if running.contains(&self) {
            Ok(None)
        } else {
            log::debug!("task locked: {:?}", self);

            let paths = paths::get(self.chat, &self.repo)?;

            if !paths.outer.is_dir() {
                return Err(Error::UnknownRepository(self.repo));
            }

            // load repo
            let repo = Repository::open(&paths.repo)?;
            // load cache
            let cache_exists = paths.cache.is_file();
            let cache = Connection::open(&paths.cache)?;
            if !cache_exists {
                cache::initialize(&cache)?;
            }
            // load settings
            let settings = read_json(&paths.settings)?;
            // load results
            let results = read_json(&paths.results)?;

            let resources = Resources {
                repo,
                cache,
                settings,
                results,
            };

            let guard_inner = TaskGuardInner {
                task: self.clone(),
                resources: Mutex::new(resources),
            };
            let result = Ok(Some(Arc::new(guard_inner)));

            // finally, perform the real lock action
            running.insert(self);
            result
        }
    }
}

pub struct Resources {
    pub repo: Repository,
    pub cache: Connection,
    pub settings: Settings,
    pub results: Results,
}

pub struct TaskGuardInner {
    pub task: Task,
    pub resources: Mutex<Resources>,
}

pub struct TaskGuardBareInner {
    pub task: Task,
}

pub type TaskGuard = Arc<TaskGuardInner>;
pub type TaskGuardBare = Arc<TaskGuardBareInner>;

pub trait TaskRef {
    fn task(&self) -> &Task;
    fn chat(&self) -> i64 {
        self.task().chat
    }

    fn repo_name(&self) -> &str {
        &self.task().repo
    }

    fn paths(&self) -> Result<paths::Paths, Error> {
        let t = self.task();
        paths::get(t.chat, &t.repo)
    }
}

impl TaskRef for TaskGuardInner {
    fn task(&self) -> &Task {
        &self.task
    }
}

impl TaskRef for TaskGuardBareInner {
    fn task(&self) -> &Task {
        &self.task
    }
}

impl TaskGuardInner {
    pub fn save_resources(&self) -> Result<(), Error> {
        let r = self.resources.try_lock().map_err(|_| Error::TryLock)?;
        let paths = self.paths()?;
        write_json(&paths.settings, &r.settings)?;
        write_json(&paths.results, &r.results)?;
        Ok(())
    }
}

impl Drop for TaskGuardInner {
    fn drop(&mut self) {
        let mut running = TASKS.lock().unwrap();
        let removed = running.remove(&self.task);
        assert!(removed);
        log::debug!("task unlocked: {:?}", self.task);
    }
}

impl Drop for TaskGuardBareInner {
    fn drop(&mut self) {
        let mut running = TASKS.lock().unwrap();
        let removed = running.remove(&self.task);
        assert!(removed);
        log::debug!("bare task unlocked: {:?}", self.task);
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
    let writer = BufWriter::new(file);
    Ok(serde_json::to_writer_pretty(writer, rs)?)
}
