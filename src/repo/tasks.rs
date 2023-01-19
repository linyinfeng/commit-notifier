use std::fmt;
use std::fs::{create_dir, File, OpenOptions};
use std::io::{BufReader, BufWriter};
use std::path::Path;
use std::sync::{Arc, Mutex};

use fs4::FileExt;
use git2::Repository;
use rusqlite::Connection;
use serde::de::DeserializeOwned;
use serde::Serialize;
use teloxide::types::ChatId;

use super::paths;
use super::results::Results;
use super::settings::Settings;
use crate::cache;
use crate::error::Error;

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug)]
pub struct Task {
    pub chat: ChatId,
    pub repo: String,
}

impl Task {
    // TODO: add lock and lock_bare

    // try lock and acquire all resources related to the task
    pub fn lock(self) -> Result<Option<TaskGuard>, Error> {
        self.try_lock_inner().map(|o| o.map(Arc::new))
    }

    // try lock only
    pub fn try_lock_bare(self) -> Result<Option<TaskGuardBare>, Error> {
        self.try_lock_bare_inner().map(|o| o.map(Arc::new))
    }

    pub fn ensure_lock_file(&self) -> Result<File, Error> {
        let paths = paths::get(self.chat, &self.repo)?;
        if !paths.outer.is_dir() {
            create_dir(paths.outer)?;
        }
        if !paths.lock.is_file() {
            File::create(&paths.lock)?;
        }
        Ok(File::open(&paths.lock)?)
    }

    pub fn try_lock_bare_inner(self) -> Result<Option<TaskGuardBareInner>, Error> {
        let f = self.ensure_lock_file()?;
        match f.try_lock_exclusive() {
            Ok(()) => {
                log::debug!("task locked: {:?}", self);
                Ok(Some(TaskGuardBareInner {
                    task: self,
                    lock_file: f,
                }))
            }
            Err(e) => {
                log::debug!("failed to lock task: {e}");
                Ok(None)
            }
        }
    }

    pub fn try_lock_inner(self) -> Result<Option<TaskGuardInner>, Error> {
        match self.try_lock_bare_inner() {
            Ok(None) => Ok(None),
            Ok(Some(bare)) => {
                let paths = paths::get(bare.chat(), bare.repo_name())?;

                if !paths.outer.is_dir() {
                    return Err(Error::UnknownRepository(bare.repo_name().to_owned()));
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

                Ok(Some(TaskGuardInner {
                    bare,
                    resources: Mutex::new(resources),
                }))
            }
            Err(e) => Err(e),
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
    pub bare: TaskGuardBareInner,
    pub resources: Mutex<Resources>,
}

pub struct TaskGuardBareInner {
    pub task: Task,
    pub lock_file: File,
}

pub type TaskGuard = Arc<TaskGuardInner>;
pub type TaskGuardBare = Arc<TaskGuardBareInner>;

pub trait TaskRef {
    fn task(&self) -> &Task;
    fn chat(&self) -> ChatId {
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

impl TaskRef for Task {
    fn task(&self) -> &Task {
        self
    }
}

impl TaskRef for TaskGuardInner {
    fn task(&self) -> &Task {
        &self.bare.task
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

impl Drop for TaskGuardBareInner {
    fn drop(&mut self) {
        let result = self.lock_file.unlock();
        if let Err(e) = result {
            log::error!("failed to unlock task {:?}: {:?}", self.task(), e);
        };
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
