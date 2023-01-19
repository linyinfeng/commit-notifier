pub mod paths;
pub mod results;
pub mod settings;
pub mod tasks;

use git2::{BranchType, Commit, Oid, Repository};
use regex::Regex;
use rusqlite::Connection;
use teloxide::types::ChatId;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::ops::DerefMut;
use std::process::{Command, Output};
use tokio::task::{self, spawn_blocking};

use crate::cache;
use crate::error::Error;
use crate::repo::results::CommitResults;
use crate::repo::tasks::TaskRef;

use self::results::BranchResults;
use self::settings::{BranchSettings, CommitSettings};
use self::tasks::{TaskGuard, TaskGuardBare};

static ORIGIN_RE: once_cell::sync::Lazy<Regex> =
    once_cell::sync::Lazy::new(|| Regex::new("^origin/(.*)$").unwrap());

#[derive(Debug)]
pub struct CommitCheckResult {
    pub all: BTreeSet<String>,
    pub new: BTreeSet<String>,
}

#[derive(Debug)]
pub struct BranchCheckResult {
    pub old: Option<String>,
    pub new: Option<String>,
}

pub async fn create(lock: TaskGuardBare, url: &str) -> Result<Output, Error> {
    let paths = lock.paths()?;

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
            name: lock.repo_name().to_owned(),
            output,
        });
    }

    let _repo = Repository::open(&repo_path)?;
    log::info!("cloned git repository {:?}", repo_path);

    Ok(output)
}

pub async fn fetch(task: TaskGuard) -> Result<Output, Error> {
    let paths = task.paths()?;
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
            name: task.repo_name().to_owned(),
            output,
        });
    }

    Ok(output)
}

pub fn exists(chat: ChatId, name: &str) -> Result<bool, Error> {
    let path = paths::get(chat, name)?.repo;
    Ok(path.is_dir())
}

pub async fn remove(lock: TaskGuardBare) -> Result<(), Error> {
    let paths = lock.paths()?;

    log::info!("try remove repository outer directory: {:?}", paths.outer);
    fs::remove_dir_all(&paths.outer)?;
    log::info!("repository outer directory removed: {:?}", &paths.outer);

    Ok(())
}

pub async fn commit_add(
    lock: TaskGuard,
    commit: &str,
    settings: CommitSettings,
) -> Result<(), Error> {
    {
        let mut resources = lock.resources.lock().unwrap();
        if resources.settings.commits.contains_key(commit) {
            return Err(Error::CommitExists(commit.to_owned()));
        }
        resources
            .settings
            .commits
            .insert(commit.to_owned(), settings);
    }
    lock.save_resources()?;
    Ok(())
}

pub async fn commit_remove(lock: TaskGuard, commit: &str) -> Result<(), Error> {
    {
        let mut resources = lock.resources.lock().unwrap();

        if !resources.settings.commits.contains_key(commit) {
            return Err(Error::UnknownCommit(commit.to_owned()));
        }

        resources.settings.commits.remove(commit);
        resources.results.commits.remove(commit);
        cache::remove(&resources.cache, commit)?;
    }
    lock.save_resources()?;
    Ok(())
}

pub async fn commit_check(
    task: TaskGuard,
    target_commit: &str,
) -> Result<CommitCheckResult, Error> {
    let target_commit = target_commit.to_owned();
    spawn_blocking(move || {
        let result = {
            let mut guard = task.resources.lock().unwrap();
            let resources = guard.deref_mut();

            let mut branches_hit = BTreeSet::new();

            let results = &mut resources.results;

            // save old results
            let old_commit_results = match results.commits.remove(&target_commit) {
                Some(bs) => bs,
                None => Default::default(), // create default result
            };
            let branches_hit_old = old_commit_results.branches;

            let repo = &resources.repo;
            let branches = repo.branches(Some(git2::BranchType::Remote))?;
            let branch_regex = Regex::new(&resources.settings.branch_regex)?;
            for branch_iter_res in branches {
                let (branch, _) = branch_iter_res?;
                // clean up name
                let branch_name = match branch.name()?.and_then(branch_name_map_filter) {
                    None => continue,
                    Some(n) => n,
                };
                // skip if not match
                if !branch_regex.is_match(branch_name) {
                    continue;
                }
                let root = branch.get().peel_to_commit()?;
                let str_root = format!("{}", root.id());

                // build the cache
                update_from_root(&resources.cache, &target_commit, repo, root)?;

                // query result from cache
                let hit_in_branch = cache::query(&resources.cache, &target_commit, &str_root)?
                    .expect("update_from_root should build cache");
                if hit_in_branch {
                    branches_hit.insert(branch_name.to_owned());
                }
            }

            // insert new results
            results.commits.insert(
                target_commit.clone(),
                CommitResults {
                    branches: branches_hit.clone(),
                },
            );
            log::info!(
                "finished updating for commit {} on {}",
                target_commit,
                task.repo_name()
            );

            // construct final check results
            let branches_hit_diff = branches_hit
                .difference(&branches_hit_old)
                .cloned()
                .collect();
            CommitCheckResult {
                all: branches_hit,
                new: branches_hit_diff,
            }
        };
        // release the lock and save result
        task.save_resources()?;
        Ok(result)
    })
    .await?
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
    root: Commit,
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
                log::info!("checking phase 1, visited {} commits", visited.len());
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
        if !sorted.is_empty() && sorted.len() % 1000 == 0 {
            log::info!("checking phase 4, remaining {} commits", sorted.len());
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
    }

    // unchecked: no nested transaction
    let tx = cache.unchecked_transaction()?;
    for (oid, hit) in in_memory_cache {
        let str_commit = format!("{}", oid);
        // wrap store operations in transaction to improve performance
        cache::store(&tx, target, &str_commit, hit)?;
    }
    tx.commit()?;

    Ok(())
}

pub async fn branch_add(
    lock: TaskGuard,
    branch: &str,
    settings: BranchSettings,
) -> Result<(), Error> {
    {
        let mut resources = lock.resources.lock().unwrap();
        if resources.settings.branches.contains_key(branch) {
            return Err(Error::BranchExists(branch.to_owned()));
        }
        resources
            .settings
            .branches
            .insert(branch.to_owned(), settings);
    }
    lock.save_resources()?;
    Ok(())
}

pub async fn branch_remove(lock: TaskGuard, branch: &str) -> Result<(), Error> {
    {
        let mut resources = lock.resources.lock().unwrap();

        if !resources.settings.branches.contains_key(branch) {
            return Err(Error::UnknownBranch(branch.to_owned()));
        }

        resources.settings.branches.remove(branch);
        resources.results.branches.remove(branch);
    }
    lock.save_resources()?;
    Ok(())
}

pub async fn branch_check(lock: TaskGuard, branch_name: &str) -> Result<BranchCheckResult, Error> {
    let result = {
        let mut resources = lock.resources.lock().unwrap();
        let old_result = match resources.results.branches.remove(branch_name) {
            Some(r) => r,
            None => Default::default(),
        };

        // get the new commit (optional)
        let remote_branch_name = format!("origin/{}", branch_name);
        let commit = match resources
            .repo
            .find_branch(&remote_branch_name, BranchType::Remote)
        {
            Ok(branch) => Some(branch.into_reference().peel_to_commit()?.id().to_string()),
            Err(_error) => {
                log::warn!(
                    "branch {} not found in ({}, {})",
                    branch_name,
                    lock.chat(),
                    lock.repo_name()
                );
                None
            }
        };

        resources.results.branches.insert(
            branch_name.to_owned(),
            BranchResults {
                commit: commit.clone(),
            },
        );

        BranchCheckResult {
            old: old_result.commit,
            new: commit,
        }
    };
    lock.save_resources()?;
    Ok(result)
}
