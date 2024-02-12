pub mod paths;
pub mod results;
pub mod settings;
pub mod tasks;

use git2::{BranchType, Commit, Oid, Repository};
use regex::Regex;
use rusqlite::Connection;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::Arc;
use teloxide::types::ChatId;
use tokio::sync::Mutex;
use tokio::task;

use crate::condition::Condition;
use crate::error::Error;
use crate::github::GitHubInfo;
use crate::repo::results::CommitResults;
use crate::repo::tasks::ResourcesMap;
use crate::utils::push_empty_line;
use crate::{cache, github};

use self::results::BranchResults;
use self::settings::{
    BranchSettings, CommitSettings, ConditionSettings, NotifySettings, PullRequestSettings,
};
use self::tasks::Resources;

static ORIGIN_RE: once_cell::sync::Lazy<Regex> =
    once_cell::sync::Lazy::new(|| Regex::new("^origin/(.*)$").unwrap());

#[derive(Debug)]
pub struct CommitCheckResult {
    pub all: BTreeSet<String>,
    pub new: BTreeSet<String>,
    pub removed_by_condition: Option<String>,
}

#[derive(Debug)]
pub struct BranchCheckResult {
    pub old: Option<String>,
    pub new: Option<String>,
}

#[derive(Debug)]
pub struct ConditionCheckResult {
    pub removed: Vec<String>,
}

pub async fn create(name: &str, path: PathBuf, url: &str) -> Result<Output, Error> {
    log::info!("try clone '{}' into {:?}", url, path);

    let output = {
        let url = url.to_owned();
        let path = path.clone();
        task::spawn_blocking(move || {
            Command::new("git")
                .arg("clone")
                .arg(url)
                .arg(path)
                // blobless clone
                .arg("--filter=tree:0")
                .output()
        })
        .await??
    };
    if !output.status.success() {
        return Err(Error::GitClone {
            url: url.to_owned(),
            name: name.to_owned(),
            output,
        });
    }

    let _repo = Repository::open(&path)?;
    log::info!("cloned git repository {:?}", path);

    Ok(output)
}

pub async fn fetch(resources: Arc<Resources>) -> Result<Output, Error> {
    let paths = &resources.paths;
    let repo_path = &paths.repo;
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
            name: resources.task.repo.to_owned(),
            output,
        });
    }

    Ok(output)
}

pub fn exists(chat: ChatId, name: &str) -> Result<bool, Error> {
    let path = paths::get(chat, name)?.repo;
    Ok(path.is_dir())
}

pub async fn remove(resources: Arc<Resources>) -> Result<(), Error> {
    let paths = resources.paths.clone();
    let task = resources.task.clone();

    drop(resources); // drop resources in hand
    ResourcesMap::remove(&task, || {
        log::info!("try remove repository outer directory: {:?}", paths.outer);
        fs::remove_dir_all(&paths.outer)?;
        log::info!("repository outer directory removed: {:?}", &paths.outer);
        Ok(())
    })
    .await?;

    Ok(())
}

pub async fn commit_add(
    resources: Arc<Resources>,
    commit: &str,
    settings: CommitSettings,
) -> Result<(), Error> {
    let _guard = resources.commit_lock(commit.to_string()).await;
    {
        let mut locked = resources.settings.write().await;
        if locked.commits.contains_key(commit) {
            return Err(Error::CommitExists(commit.to_owned()));
        }
        locked.commits.insert(commit.to_owned(), settings);
    }
    resources.save_settings().await
}

pub async fn commit_remove(resources: Arc<Resources>, commit: &str) -> Result<(), Error> {
    let _guard = resources.commit_lock(commit.to_string()).await;
    {
        let mut settings = resources.settings.write().await;
        if !settings.commits.contains_key(commit) {
            return Err(Error::UnknownCommit(commit.to_owned()));
        }
        settings.commits.remove(commit);
    }
    resources.save_settings().await?;

    {
        let mut results = resources.results.write().await;
        results.commits.remove(commit);
    }
    resources.save_results().await?;

    {
        let cache = resources.cache().await?;
        let commit = commit.to_owned();
        cache
            .interact(move |conn| cache::remove(conn, &commit))
            .await
            .map_err(|e| Error::DBInteract(Mutex::new(e)))??;
    }
    Ok(())
}

pub async fn commit_check(
    resources: Arc<Resources>,
    target_commit: &str,
) -> Result<CommitCheckResult, Error> {
    let result = {
        let _guard = resources.commit_lock(target_commit.to_string()).await;

        /* 2 phase */

        /* phase 1: commit check */

        // get settings
        let settings = {
            let s = resources.settings.read().await;
            s.clone()
        };

        // get old results
        let mut old_results = {
            let r = resources.results.read().await;
            r.clone()
        };
        let old_commit_results = match old_results.commits.remove(target_commit) {
            Some(bs) => bs,
            None => Default::default(), // create default result
        };
        let branches_hit_old = old_commit_results.branches;

        let branches_hit = {
            let cache = resources.cache().await?;
            let target_commit = target_commit.to_owned();
            let resources = resources.clone();
            cache
                .interact(move |conn| -> Result<_, Error> {
                    let repo = resources.repo.blocking_lock();
                    let branches = repo.branches(Some(git2::BranchType::Remote))?;
                    let branch_regex = Regex::new(&settings.branch_regex)?;

                    let mut branches_hit = BTreeSet::new();
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
                        update_from_root(conn, &target_commit, &repo, root)?;

                        // query result from cache
                        let hit_in_branch = cache::query(conn, &target_commit, &str_root)?
                            .expect("update_from_root should build cache");
                        if hit_in_branch {
                            branches_hit.insert(branch_name.to_owned());
                        }
                    }
                    Ok(branches_hit)
                })
                .await
                .map_err(|e| Error::DBInteract(Mutex::new(e)))??
        };

        let commit_results = CommitResults {
            branches: branches_hit.clone(),
        };
        log::info!(
            "finished updating for commit {} on {}",
            target_commit,
            resources.task.repo
        );

        // insert and save new results
        {
            let mut results = resources.results.write().await;
            results
                .commits
                .insert(target_commit.to_owned(), commit_results.clone());
        }
        resources.save_results().await?;

        // construct final check results
        let branches_hit_diff = branches_hit
            .difference(&branches_hit_old)
            .cloned()
            .collect();

        /* phase 2: condition check */
        let mut removed_by_condition = None;
        for (identifier, c) in settings.conditions.iter() {
            if c.condition.meet(&commit_results) && removed_by_condition.is_none() {
                removed_by_condition = Some(identifier.clone());
            }
        }

        CommitCheckResult {
            all: branches_hit,
            new: branches_hit_diff,
            removed_by_condition,
        }
    }; // release the commit lock

    if result.removed_by_condition.is_some() {
        commit_remove(resources.clone(), target_commit).await?;
    }
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

fn update_from_root<'r>(
    cache: &Connection,
    target: &str,
    repo: &'r Repository,
    root: Commit<'r>,
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
        let str_commit = format!("{oid}");

        let mut hit = false;
        if str_commit == target {
            hit = true;
        } else {
            for parent in commit.parents() {
                let pid = parent.id();
                hit |= if in_memory_cache.contains_key(&pid) {
                    in_memory_cache[&pid]
                } else {
                    let str_parent = format!("{pid}");
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
        let str_commit = format!("{oid}");
        // wrap store operations in transaction to improve performance
        cache::store(&tx, target, &str_commit, hit)?;
    }
    tx.commit()?;

    Ok(())
}

pub async fn branch_add(
    resources: Arc<Resources>,
    branch: &str,
    settings: BranchSettings,
) -> Result<(), Error> {
    let _guard = resources.branch_lock(branch.to_string()).await;
    {
        let mut locked = resources.settings.write().await;
        if locked.branches.contains_key(branch) {
            return Err(Error::BranchExists(branch.to_owned()));
        }
        locked.branches.insert(branch.to_owned(), settings);
    }
    resources.save_settings().await
}

pub async fn branch_remove(resources: Arc<Resources>, branch: &str) -> Result<(), Error> {
    let _guard = resources.branch_lock(branch.to_string()).await;
    {
        let mut locked = resources.settings.write().await;
        if !locked.branches.contains_key(branch) {
            return Err(Error::UnknownBranch(branch.to_owned()));
        }
        locked.branches.remove(branch);
    }
    resources.save_settings().await?;

    {
        let mut locked = resources.results.write().await;
        locked.branches.remove(branch);
    }
    resources.save_results().await
}

pub async fn branch_check(
    resources: Arc<Resources>,
    branch_name: &str,
) -> Result<BranchCheckResult, Error> {
    let _guard = resources.branch_lock(branch_name.to_string()).await;
    let result = {
        let old_result = {
            let results = resources.results.read().await;
            match results.branches.get(branch_name) {
                Some(r) => r.clone(),
                None => Default::default(),
            }
        };

        // get the new commit (optional)
        let commit = {
            let repo = resources.repo.lock().await;
            let remote_branch_name = format!("origin/{branch_name}");
            let c = match repo.find_branch(&remote_branch_name, BranchType::Remote) {
                Ok(branch) => {
                    let commit: String = branch.into_reference().peel_to_commit()?.id().to_string();
                    Some(commit)
                }
                Err(_error) => {
                    log::warn!(
                        "branch {} not found in ({}, {})",
                        branch_name,
                        resources.task.chat,
                        resources.task.repo,
                    );
                    None
                }
            };
            c
        };

        {
            let mut results = resources.results.write().await;
            results.branches.insert(
                branch_name.to_owned(),
                BranchResults {
                    commit: commit.clone(),
                },
            );
        }
        resources.save_results().await?;

        BranchCheckResult {
            old: old_result.commit,
            new: commit,
        }
    };
    Ok(result)
}

pub async fn condition_add(
    resources: Arc<Resources>,
    identifier: &str,
    settings: ConditionSettings,
) -> Result<(), Error> {
    {
        let mut locked = resources.settings.write().await;
        if locked.conditions.contains_key(identifier) {
            return Err(Error::ConditionExists(identifier.to_owned()));
        }
        locked.conditions.insert(identifier.to_owned(), settings);
    }
    resources.save_settings().await
}

pub async fn condition_remove(resources: Arc<Resources>, identifier: &str) -> Result<(), Error> {
    {
        let mut locked = resources.settings.write().await;
        if !locked.conditions.contains_key(identifier) {
            return Err(Error::UnknownCondition(identifier.to_owned()));
        }
        locked.conditions.remove(identifier);
    }
    resources.save_settings().await
}

pub async fn condition_trigger(
    resources: Arc<Resources>,
    identifier: &str,
) -> Result<ConditionCheckResult, Error> {
    let mut remove_list = Vec::new();
    {
        let cond = {
            let settings = resources.settings.read().await;
            match settings.conditions.get(identifier) {
                Some(s) => s.condition.clone(),
                None => return Err(Error::UnknownCondition(identifier.to_owned())),
            }
        };
        let commits = {
            let results = resources.results.read().await;
            results.commits.clone()
        };
        for (commit, result) in commits.iter() {
            if cond.meet(result) {
                remove_list.push(commit.clone());
            }
        }
        for r in remove_list.iter() {
            commit_remove(resources.clone(), r).await?;
        }
    }
    Ok(ConditionCheckResult {
        removed: remove_list,
    })
}

pub async fn pr_add(
    resources: Arc<Resources>,
    id: u64,
    settings: PullRequestSettings,
) -> Result<(), Error> {
    {
        let mut locked = resources.settings.write().await;
        if locked.pull_requests.contains_key(&id) {
            return Err(Error::PullRequestExists(id));
        }
        locked.pull_requests.insert(id, settings);
    }
    resources.save_settings().await
}

pub async fn pr_remove(resources: Arc<Resources>, id: u64) -> Result<(), Error> {
    {
        let mut locked = resources.settings.write().await;
        if !locked.pull_requests.contains_key(&id) {
            return Err(Error::UnknownPullRequest(id));
        }
        locked.pull_requests.remove(&id);
    }
    resources.save_settings().await
}

pub async fn pr_check(resources: Arc<Resources>, id: u64) -> Result<Option<String>, Error> {
    let github_info = {
        let locked = resources.settings.read().await;
        locked
            .github_info
            .clone()
            .ok_or(Error::NoGitHubInfo(resources.task.repo.clone()))?
    };
    log::info!("checking pr {github_info}#{id}");
    if github::is_merged(&github_info, id).await? {
        let settings = {
            let mut locked = resources.settings.write().await;
            locked
                .pull_requests
                .remove(&id)
                .ok_or(Error::UnknownPullRequest(id))?
        };
        resources.save_settings().await?;
        let commit = merged_pr_to_commit(resources, github_info, id, settings).await?;
        Ok(Some(commit))
    } else {
        Ok(None)
    }
}

pub async fn merged_pr_to_commit(
    resources: Arc<Resources>,
    github_info: GitHubInfo,
    pr_id: u64,
    settings: PullRequestSettings,
) -> Result<String, Error> {
    let pr = github::get_pr(&github_info, pr_id).await?;
    let commit = pr
        .merge_commit_sha
        .ok_or(Error::NoMergeCommit { github_info, pr_id })?;
    let comment = format!(
        "{title}{comment}",
        title = pr.title.as_deref().unwrap_or("untitled"),
        comment = push_empty_line(&settings.notify.comment),
    );
    let commit_settings = CommitSettings {
        url: Some(settings.url),
        notify: NotifySettings {
            comment,
            subscribers: settings.notify.subscribers,
        },
    };

    commit_add(resources, &commit, commit_settings)
        .await
        .map(|()| commit)
}
