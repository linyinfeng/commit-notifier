use crate::options;
use git2::{Commit, Oid, Repository};
use regex::Regex;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter};
use std::path::Path;
use std::sync::Mutex;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};
use tokio::task;

use crate::cache;
use crate::error::Error;

static NAME_RE: once_cell::sync::Lazy<Regex> = once_cell::sync::Lazy::new(|| Regex::new("^[a-zA-Z0-9_\\-]*$").unwrap());
static ORIGIN_RE: once_cell::sync::Lazy<Regex> = once_cell::sync::Lazy::new(|| Regex::new("^origin/(.*)$").unwrap());
static TASKS: once_cell::sync::Lazy<Mutex<BTreeSet<Task>>> = once_cell::sync::Lazy::new(|| Mutex::new(BTreeSet::new()));

pub struct Paths {
    pub outer: PathBuf,
    pub repo: PathBuf,
    pub cache: PathBuf,
    pub results: PathBuf,
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Debug)]
pub struct Task {
    pub chat: i64,
    pub name: String,
}

pub struct TaskGuard {
    pub task: Task,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitInfo {
    pub comment: String,
}

#[derive(Debug)]
pub struct CheckResult {
    pub all: BTreeSet<String>,
    pub new: BTreeSet<String>,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct Results {
    data: BTreeMap<String, CommitResults>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitResults {
    info: CommitInfo,
    branches: BTreeSet<String>,
}

fn chat_dir(chat: i64) -> PathBuf {
    let working_dir = &options::get().working_dir;
    let chat_dir_name = if chat < 0 {
        format!("_{}", chat.unsigned_abs())
    } else {
        format!("{}", chat)
    };
    working_dir.join(chat_dir_name)
}

fn get_paths(chat: i64, name: &str) -> Result<Paths, Error> {
    if !NAME_RE.is_match(name) {
        return Err(Error::Name(name.to_owned()));
    }

    let chat_working_dir = chat_dir(chat);
    if !chat_working_dir.is_dir() {
        Err(Error::NotInAllowList(chat))
    } else {
        let outer_dir = chat_working_dir.join(name);
        Ok(Paths {
            outer: outer_dir.clone(),
            repo: outer_dir.join("repo"),
            cache: outer_dir.join("cache.sqlite"),
            results: outer_dir.join("results.json"),
        })
    }
}

pub fn get_chats() -> Result<BTreeSet<i64>, Error> {
    let mut chats = BTreeSet::new();
    let working_dir = &options::get().working_dir;
    let dirs = fs::read_dir(working_dir)?;
    for dir_res in dirs {
        let dir = dir_res?;
        let name_os = dir.file_name();
        let name = name_os.into_string().map_err(Error::InvalidOsString)?;

        let invalid_error = Err(Error::InvalidChatDir(name.clone()));
        if name.is_empty() {
            return invalid_error;
        }

        let name_vec: Vec<_> = name.chars().collect();
        if name_vec[0] == '_' {
            let n: i64 = name_vec[1..].iter().collect::<String>().parse()?;
            chats.insert(-n);
        } else {
            chats.insert(name.parse()?);
        }
    }
    Ok(chats)
}

pub fn get_repos(chat: i64) -> Result<BTreeSet<String>, Error> {
    let mut repos = BTreeSet::new();

    let chat_working_dir = chat_dir(chat);
    let dirs = fs::read_dir(chat_working_dir)?;
    for dir_res in dirs {
        let dir = dir_res?;
        let name_os = dir.file_name();
        let name = name_os.into_string().map_err(Error::InvalidOsString)?;
        repos.insert(name);
    }

    Ok(repos)
}

pub fn get_commits(chat: i64, name: &str) -> Result<BTreeMap<String, CommitInfo>, Error> {
    let paths = get_paths(chat, name)?;
    let res = read_results(paths.results)?;
    Ok(res.data.into_iter().map(|(k, v)| (k, v.info)).collect())
}

pub async fn create(lock: &TaskGuard, url: &str) -> Result<Output, Error> {
    let chat = lock.task.chat;
    let name = &lock.task.name;
    let paths = get_paths(chat, name)?;

    let repo_path = paths.repo;
    log::info!("try clone '{}' into {:?}", url, repo_path);

    let output = {
        let url = url.to_owned();
        let path = repo_path.clone();
        task::spawn_blocking(move || Command::new("git").arg("clone").arg(url).arg(path).output())
            .await??
    };
    if !output.status.success() {
        return Err(Error::GitClone {
            url: url.to_owned(),
            name: name.to_owned(),
            output,
        });
    }

    let _repo = Repository::open(&repo_path)?;
    log::info!("cloned git repository {:?}", repo_path);

    Ok(output)
}

pub async fn fetch(lock: &TaskGuard) -> Result<Output, Error> {
    let name = &lock.task.name;
    let paths = get_paths(lock.task.chat, name)?;

    let repo_path = paths.repo;
    log::info!("fetch {:?}", repo_path);

    let output = {
        let path = repo_path.clone();
        task::spawn_blocking(move || {
            Command::new("git")
                .arg("fetch")
                .arg("--all")
                .current_dir(path)
                .output()
        })
        .await??
    };
    if !output.status.success() {
        return Err(Error::GitFetch {
            name: name.to_owned(),
            output,
        });
    }

    Ok(output)
}

pub fn exists(chat: i64, name: &str) -> Result<bool, Error> {
    let path = get_paths(chat, name)?.repo;
    Ok(path.is_dir())
}

pub async fn remove(lock: &TaskGuard) -> Result<(), Error> {
    let paths = get_paths(lock.task.chat, &lock.task.name)?;

    log::info!("try remove result file {:?}", paths.outer);
    fs::remove_dir_all(&paths.outer)?;
    log::info!("cache db {:?} removed", &paths.outer);

    Ok(())
}

pub async fn check(lock: &TaskGuard, target_commit: &str) -> Result<CheckResult, Error> {
    let Task { chat, ref name } = lock.task;
    let paths = get_paths(chat, name)?;

    if !paths.repo.is_dir() {
        return Err(Error::UnknownRepository(name.to_owned()));
    }

    let result = {
        let target_commit = target_commit.to_owned();
        task::spawn_blocking(move || -> Result<CheckResult, Error> {
            let mut branches_hit = BTreeSet::new();

            let mut results = read_results(&paths.results)?;
            let old_commit_results = match results.data.remove(&target_commit) {
                Some(bs) => bs,
                None => return Err(Error::UnknownCommit(target_commit)),
            };
            let branches_hit_old = old_commit_results.branches;

            let cache_exists = paths.cache.is_file();
            let cache = Connection::open(&paths.cache)?;
            if !cache_exists {
                cache::initialize(&cache)?;
            }

            let repo = Repository::open(&paths.repo)?;
            let branches = repo.branches(Some(git2::BranchType::Remote))?;
            for branch_iter_res in branches {
                let (branch, _) = branch_iter_res?;
                let branch_name = match branch.name()?.and_then(branch_name_map_filter) {
                    None => continue,
                    Some(n) => n,
                };
                let root = branch.get().peel_to_commit()?;
                let str_root = format!("{}", root.id());
                update_from_root(&cache, &target_commit, &repo, root)?;
                let hit_in_branch = cache::query(&cache, &target_commit, &str_root)?
                    .expect("update_from_root should build cache");
                if hit_in_branch {
                    branches_hit.insert(branch_name.to_owned());
                }
            }

            let branches_hit_diff = branches_hit
                .difference(&branches_hit_old)
                .cloned()
                .collect();

            results.data.insert(
                target_commit,
                CommitResults {
                    info: old_commit_results.info,
                    branches: branches_hit.clone(),
                },
            );
            write_results(&paths.results, &results)?;

            Ok(CheckResult {
                all: branches_hit,
                new: branches_hit_diff,
            })
        })
        .await??
    };

    Ok(result)
}

fn branch_name_map_filter(name: &str) -> Option<&str> {
    if name == "origin/HEAD" {
        return None;
    }

    let captures = match ORIGIN_RE.captures(name) {
        Some(cap) => cap,
        None => return Some(name),
    };

    Some(captures.get(1).unwrap().as_str())
}

fn update_from_root<'repo>(
    cache: &Connection,
    target: &str,
    repo: &'repo Repository,
    root: Commit<'repo>,
) -> Result<(), Error> {
    // phase 1: find commits with out cache
    let todo = {
        let mut t = BTreeSet::new();
        let mut visited = BTreeSet::new();
        let mut stack = vec![root.clone()];
        while let Some(commit) = stack.pop() {
            if !visited.insert(commit.id()) {
                continue;
            }
            if visited.len() % 1000 == 0 {
                log::info!(
                    "checking '{}' phase 1, visited {} commits",
                    target,
                    visited.len()
                );
            }
            let str_commit = format!("{}", commit.id());
            if cache::query(cache, target, &str_commit)?.is_none() {
                t.insert(commit.id());
                for parent in commit.parents() {
                    stack.push(parent);
                }
            }
        }
        t
    };

    // phase 2: build indegree mapping
    let root_id = root.id();
    let mut indegrees: BTreeMap<Oid, usize> = BTreeMap::new();
    for oid in todo.iter() {
        if !indegrees.contains_key(oid) {
            indegrees.insert(*oid, 0);
        }
        let commit = repo.find_commit(*oid)?;
        for parent in commit.parents() {
            let pid = parent.id();
            if todo.contains(&pid) {
                let n = indegrees.get(&pid).cloned().unwrap_or(0);
                indegrees.insert(pid, n + 1);
            }
        }
    }

    // phase 3: sort commits
    let mut sorted = vec![];

    if !indegrees.is_empty() {
        assert!(indegrees.contains_key(&root_id));
        indegrees.remove(&root_id);
        let mut next = vec![root];
        while let Some(commit) = next.pop() {
            sorted.push(commit.clone());

            for parent in commit.parents() {
                let pid = parent.id();
                if indegrees.contains_key(&pid) {
                    let new_count = indegrees[&pid] - 1;
                    if new_count == 0 {
                        indegrees.remove(&pid);
                        next.push(parent);
                    } else {
                        indegrees.insert(pid, new_count);
                    }
                }
            }
        }
    }
    assert!(indegrees.is_empty());

    // phase 4: build caches
    let mut in_memory_cache: BTreeMap<Oid, bool> = BTreeMap::new();
    while let Some(commit) = sorted.pop() {
        if sorted.len() % 1000 == 0 {
            log::info!(
                "checking '{}' phase 4, remaining {} commits",
                target,
                sorted.len()
            );
        }

        let oid = commit.id();
        let str_commit = format!("{}", oid);

        let mut hit = false;
        if str_commit == target {
            hit = true;
        } else {
            for parent in commit.parents() {
                let pid = parent.id();
                hit |= if in_memory_cache.contains_key(&pid) {
                    in_memory_cache[&pid]
                } else {
                    let str_parent = format!("{}", pid);
                    match cache::query(cache, target, &str_parent)? {
                        None => unreachable!(),
                        Some(b) => b,
                    }
                }
            }
        }

        in_memory_cache.insert(oid, hit);
        cache::store(cache, target, &str_commit, hit)?;
    }

    Ok(())
}

pub async fn commit_add(lock: &TaskGuard, commit: &str, info: CommitInfo) -> Result<(), Error> {
    let Task { chat, ref name } = lock.task;
    let paths = get_paths(chat, name)?;
    if !paths.repo.is_dir() {
        return Err(Error::UnknownRepository(name.to_owned()));
    }
    let mut results = read_results(&paths.results)?;
    if results.data.contains_key(commit) {
        return Err(Error::CommitExists(commit.to_owned()));
    }
    results.data.insert(
        commit.to_owned(),
        CommitResults {
            info,
            branches: BTreeSet::new(),
        },
    );
    write_results(paths.results, &results)?;
    Ok(())
}

pub async fn commit_info(lock: &TaskGuard, commit: &str) -> Result<CommitInfo, Error> {
    let Task { chat, ref name } = lock.task;
    let paths = get_paths(chat, name)?;
    let mut results = read_results(&paths.results)?;
    match results.data.remove(commit) {
        Some(r) => Ok(r.info),
        None => Err(Error::UnknownCommit(commit.to_owned())),
    }
}

pub async fn commit_remove(lock: &TaskGuard, commit: &str) -> Result<(), Error> {
    let Task { chat, ref name } = lock.task;
    let paths = get_paths(chat, name)?;
    if !paths.repo.is_dir() {
        return Err(Error::UnknownRepository(name.to_owned()));
    }

    // remove cache
    let cache_exists = paths.cache.is_file();
    if cache_exists {
        let cache = Connection::open(&paths.cache)?;
        cache::remove(&cache, commit)?;
    }

    let mut results = read_results(&paths.results)?;
    if !results.data.contains_key(commit) {
        return Err(Error::UnknownCommit(commit.to_owned()));
    }
    results.data.remove(commit);
    write_results(paths.results, &results)?;
    Ok(())
}

pub fn lock_task(task: Task) -> Option<TaskGuard> {
    let mut running = TASKS.lock().unwrap();
    if running.contains(&task) {
        None
    } else {
        log::debug!("task locked: {:?}", task);
        running.insert(task.clone());
        Some(TaskGuard { task })
    }
}

impl Drop for TaskGuard {
    fn drop(&mut self) {
        let mut running = TASKS.lock().unwrap();
        let removed = running.remove(&self.task);
        assert!(removed);
        log::debug!("task unlocked: {:?}", self.task);
    }
}

fn read_results<P: AsRef<Path> + fmt::Debug>(path: P) -> Result<Results, Error> {
    if !path.as_ref().is_file() {
        log::info!("create result file: {:?}", path);
        write_results(&path, &Default::default())?;
    }
    log::debug!("read from file: {:?}", path);
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    Ok(serde_json::from_reader(reader)?)
}

fn write_results<P: AsRef<Path> + fmt::Debug>(path: P, rs: &Results) -> Result<(), Error> {
    log::debug!("write to file: {:?}", path);
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    let writer = BufWriter::new(file);
    Ok(serde_json::to_writer_pretty(writer, rs)?)
}