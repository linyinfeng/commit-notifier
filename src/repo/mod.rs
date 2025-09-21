use std::{
    collections::{BTreeSet, VecDeque},
    process::{Command, Output},
    sync::{Arc, LazyLock},
};

use git2::{Commit, Oid, Repository};
use regex::Regex;
use tokio::{
    fs::{create_dir_all, read_dir, remove_dir_all},
    sync::Mutex,
    task,
};

use crate::{
    error::Error,
    repo::{
        cache::batch_store_cache,
        paths::RepoPaths,
        resources::{RESOURCES_MAP, RepoResources},
    },
};

pub mod cache;
pub mod paths;
pub mod resources;
pub mod settings;

pub async fn create(name: &str, url: &str) -> Result<Output, Error> {
    let paths = RepoPaths::new(name);
    log::info!("try clone '{url}' into {:?}", paths.repo);
    if paths.outer.exists() {
        return Err(Error::RepoExists(name.to_string()));
    }
    create_dir_all(paths.outer).await?;
    let output = {
        let url = url.to_owned();
        let path = paths.repo.clone();
        task::spawn_blocking(move || {
            Command::new("git")
                .arg("clone")
                .arg(url)
                .arg(path)
                // blobless clone
                .arg("--filter=tree:0")
                .output()
        })
        .await
        .unwrap()?
    };
    if !output.status.success() {
        return Err(Error::GitClone {
            url: url.to_owned(),
            name: name.to_owned(),
            output,
        });
    }
    // try open the repository
    let _repo = Repository::open(&paths.repo)?;
    log::info!("cloned git repository {:?}", paths.repo);

    Ok(output)
}

pub async fn remove(name: &str) -> Result<(), Error> {
    RESOURCES_MAP
        .remove(&name.to_string(), async |r: Arc<RepoResources>| {
            log::info!("try remove repository outer directory: {:?}", r.paths.outer);
            remove_dir_all(&r.paths.outer).await?;
            log::info!("repository outer directory removed: {:?}", &r.paths.outer);
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn list() -> Result<BTreeSet<String>, Error> {
    let mut dir = read_dir(&*paths::GLOBAL_REPO_OUTER).await?;
    let mut result = BTreeSet::new();
    while let Some(entry) = dir.next_entry().await? {
        let filename = entry.file_name();
        result.insert(
            filename
                .to_str()
                .ok_or_else(|| Error::InvalidOsString(filename.clone()))?
                .to_owned(),
        );
    }
    Ok(result)
}

pub async fn fetch_and_update_cache(resources: &RepoResources) -> Result<(), Error> {
    fetch(resources).await?;
    update_cache(resources).await?;
    Ok(())
}

pub async fn fetch(resources: &RepoResources) -> Result<Output, Error> {
    let paths = &resources.paths;
    let repo_path = &paths.repo;
    log::info!("fetch {repo_path:?}");

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
            name: resources.name.to_owned(),
            output,
        });
    }

    Ok(output)
}

pub async fn update_cache(resources: &RepoResources) -> Result<(), Error> {
    let branches: BTreeSet<String> = watching_branches(resources).await?;
    log::debug!(
        "update cache for branches of {repo}: {branches:?}",
        repo = resources.name
    );
    let cache = resources.cache().await?;
    let old_branches = cache
        .interact(move |c| cache::branches(c))
        .await
        .map_err(|e| Error::DBInteract(Mutex::new(e)))??;
    let mut new_branches: BTreeSet<String> = branches.difference(&old_branches).cloned().collect();
    let update_branches = branches.intersection(&old_branches);
    let mut remove_branches: BTreeSet<String> =
        old_branches.difference(&branches).cloned().collect();
    let repo = resources.repo.lock().await;
    for b in update_branches {
        let commit = branch_commit(&repo, b)?;
        let b_cloned = b.clone();
        let old_commit_str = cache
            .interact(move |conn| cache::query_branch(conn, &b_cloned))
            .await
            .map_err(|e| Error::DBInteract(Mutex::new(e)))??;
        let old_commit = repo.find_commit(Oid::from_str(&old_commit_str)?)?;
        if old_commit.id() == commit.id() {
            log::debug!(
                "branch ({repo}, {b}) does not change, skip...",
                repo = resources.name
            );
        } else if is_parent(old_commit.clone(), commit.clone()) {
            log::debug!("updating branch ({repo}, {b})...", repo = resources.name);
            let mut queue = VecDeque::new();
            let mut new_commits = BTreeSet::new();
            queue.push_back(commit.clone());
            while let Some(c) = queue.pop_front() {
                let id = c.id().to_string();
                let exist = {
                    let b = b.clone();
                    let id = id.clone();
                    cache
                        .interact(move |conn| cache::query_cache(conn, &b, &id))
                        .await
                        .map_err(|e| Error::DBInteract(Mutex::new(e)))??
                };
                if !exist && !new_commits.contains(&id) {
                    new_commits.insert(id);
                    if new_commits.len() % 100000 == 0 {
                        log::debug!(
                            "gathering new commits, current count: {count}, current queue size: {size}",
                            count = new_commits.len(),
                            size = queue.len()
                        );
                    }
                    for p in c.parents() {
                        queue.push_back(p);
                    }
                }
            }
            log::debug!("find {} new commits", new_commits.len());
            {
                let b = b.clone();
                cache
                    .interact(move |conn| batch_store_cache(conn, &b, new_commits))
                    .await
                    .map_err(|e| Error::DBInteract(Mutex::new(e)))??;
            }
            {
                let commit_str = commit.id().to_string();
                let b = b.clone();
                cache
                    .interact(move |conn| cache::update_branch(conn, &b, &commit_str))
                    .await
                    .map_err(|e| Error::DBInteract(Mutex::new(e)))??;
            }
        } else {
            remove_branches.insert(b.to_owned());
            new_branches.insert(b.to_owned());
        }
    }
    for b in remove_branches {
        log::debug!("removing branch ({repo}, {b})...", repo = resources.name);
        let b = b.clone();
        cache
            .interact(move |conn| cache::remove_branch(conn, &b))
            .await
            .map_err(|e| Error::DBInteract(Mutex::new(e)))??;
    }
    for b in new_branches {
        log::debug!("adding branch ({repo}, {b})...", repo = resources.name);
        let commit = branch_commit(&repo, &b)?;
        let commits = gather_commits(commit.clone());
        {
            let b = b.clone();
            cache
                .interact(move |conn| batch_store_cache(conn, &b, commits))
                .await
                .map_err(|e| Error::DBInteract(Mutex::new(e)))??;
        }
        let commit_str = commit.id().to_string();
        cache
            .interact(move |conn| cache::store_branch(conn, &b, &commit_str))
            .await
            .map_err(|e| Error::DBInteract(Mutex::new(e)))??;
    }
    Ok(())
}

fn branch_commit<'repo>(repo: &'repo Repository, branch: &str) -> Result<Commit<'repo>, Error> {
    let full_name = format!("origin/{branch}");
    let branch = repo.find_branch(&full_name, git2::BranchType::Remote)?;
    let commit = branch.into_reference().peel_to_commit()?;
    Ok(commit)
}

fn gather_commits<'repo>(commit: Commit<'repo>) -> BTreeSet<String> {
    let mut commits = BTreeSet::new();
    let mut queue = VecDeque::new();
    queue.push_back(commit);
    while let Some(c) = queue.pop_front() {
        if commits.insert(c.id().to_string()) {
            if commits.len() % 100000 == 0 {
                log::debug!(
                    "gathering commits, current count: {count}, current queue size: {size}",
                    count = commits.len(),
                    size = queue.len()
                );
            }
            for p in c.parents() {
                queue.push_back(p);
            }
        }
    }
    commits
}

fn is_parent<'repo>(parent: Commit<'repo>, child: Commit<'repo>) -> bool {
    let mut queue = VecDeque::new();
    let mut visited = BTreeSet::new();
    queue.push_back(child);
    while let Some(c) = queue.pop_front() {
        if c.id() == parent.id() {
            return true;
        }
        if visited.insert(c.id()) {
            // not visited
            if visited.len() % 100000 == 0 {
                log::debug!(
                    "testing parent commit, current count: {count}, current queue size: {size}",
                    count = visited.len(),
                    size = queue.len()
                );
            }
            for p in c.parents() {
                queue.push_back(p);
            }
        }
    }
    false
}

pub async fn watching_branches(resources: &RepoResources) -> Result<BTreeSet<String>, Error> {
    let repo = resources.repo.lock().await;
    let remote_branches = repo.branches(Some(git2::BranchType::Remote))?;
    let branch_regex = {
        let settings = resources.settings.read().await;
        Regex::new(&format!("^{}$", settings.branch_regex))?
    };
    let mut matched_branches = BTreeSet::new();
    for branch_iter_res in remote_branches {
        let (branch, _) = branch_iter_res?;
        // clean up name
        let branch_name = match branch.name()?.and_then(branch_name_map_filter) {
            None => continue,
            Some(n) => n,
        };
        // skip if not match
        if branch_regex.is_match(branch_name) {
            matched_branches.insert(branch_name.to_string());
        }
    }
    Ok(matched_branches)
}

static ORIGIN_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new("^origin/(.*)$").unwrap());

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
