use std::{collections::BTreeSet, fmt, sync::Arc};

use git2::BranchType;
use teloxide::types::{ChatId, Message};
use tokio::{fs::read_dir, sync::Mutex};

use crate::{
    chat::{
        paths::ChatRepoPaths,
        resources::ChatRepoResources,
        results::{BranchCheckResult, BranchResults, CommitCheckResult, CommitResults},
        settings::{BranchSettings, CommitSettings, NotifySettings, PullRequestSettings},
    },
    condition::{Action, Condition},
    error::Error,
    github::{self, GitHubInfo},
    repo::{cache::query_cache_commit, resources::RepoResources},
    utils::push_empty_line,
};

pub mod paths;
pub mod resources;
pub mod results;
pub mod settings;

pub async fn chats() -> Result<BTreeSet<ChatId>, Error> {
    let mut chats = BTreeSet::new();
    let dir_path = &paths::GLOBAL_CHATS_OUTER;
    let mut dir = read_dir(dir_path.as_path()).await?;
    while let Some(entry) = dir.next_entry().await? {
        let name_os = entry.file_name();
        let name = name_os.into_string().map_err(Error::InvalidOsString)?;

        let invalid_error = Err(Error::InvalidChatDir(name.clone()));
        if name.is_empty() {
            return invalid_error;
        }

        let name_vec: Vec<_> = name.chars().collect();
        let (sign, num_str) = if name_vec[0] == '_' {
            (-1, &name_vec[1..])
        } else {
            (1, &name_vec[..])
        };
        let n: i64 = match num_str.iter().collect::<String>().parse() {
            Ok(n) => n,
            Err(e) => {
                log::warn!("invalid chat directory '{name}': {e}, ignoring");
                continue;
            }
        };
        chats.insert(ChatId(sign * n));
    }
    Ok(chats)
}

pub async fn repos(chat: ChatId) -> Result<BTreeSet<String>, Error> {
    let directory = ChatRepoPaths::outer_dir(chat);
    if !directory.exists() {
        Ok(BTreeSet::new())
    } else {
        let mut results = BTreeSet::new();
        let mut dir = read_dir(&directory).await?;
        while let Some(entry) = dir.next_entry().await? {
            results.insert(
                entry
                    .file_name()
                    .into_string()
                    .map_err(Error::InvalidOsString)?,
            );
        }
        Ok(results)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Task {
    pub chat: ChatId,
    pub repo: String,
}

impl fmt::Display for Task {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Task({}, {})", self.chat, self.repo)
    }
}

pub async fn resources(task: &Task) -> Result<Arc<ChatRepoResources>, Error> {
    resources::RESOURCES_MAP.get(task).await
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
    {
        let mut locked = resources.settings.write().await;
        if locked.commits.contains_key(hash) {
            return Err(Error::CommitExists(hash.to_owned()));
        }
        locked.commits.insert(hash.to_owned(), settings);
    }
    resources.save_settings().await
}

pub async fn commit_remove(resources: &ChatRepoResources, hash: &str) -> Result<(), Error> {
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
    resources.save_settings().await?;
    resources.save_results().await?;
    Ok(())
}

pub async fn commit_check(
    resources: &ChatRepoResources,
    repo_resources: &RepoResources,
    hash: &str,
) -> Result<CommitCheckResult, Error> {
    log::info!("checking commit ({task}, {hash})", task = resources.task);
    let cache = repo_resources.cache().await?;
    let all_branches = {
        let commit = hash.to_string();
        cache
            .interact(move |conn| query_cache_commit(conn, &commit))
            .await
            .map_err(|e| Error::DBInteract(Mutex::new(e)))??
    };
    let new_results = CommitResults {
        branches: all_branches.clone(),
    };
    let old_results = {
        let mut results = resources.results.write().await;
        results
            .commits
            .insert(hash.to_string(), new_results)
            .unwrap_or_default()
    };
    let new_branches = all_branches
        .difference(&old_results.branches)
        .cloned()
        .collect();
    let mut check_result = CommitCheckResult {
        all: all_branches,
        new: new_branches,
        conditions: Default::default(),
    };
    let mut remove = false;
    {
        let settings = repo_resources.settings.read().await;
        for (condition_name, condition_setting) in &settings.conditions {
            let action = condition_setting.condition.check(&check_result);
            if action.is_none() {
                continue;
            } else {
                check_result
                    .conditions
                    .insert(condition_name.clone(), action);
                if action == Action::Remove {
                    remove = true;
                }
            }
        }
    }
    if remove {
        let mut settings = resources.settings.write().await;
        let mut results = resources.results.write().await;
        settings.commits.remove(hash);
        results.commits.remove(hash);
    }
    resources.save_settings().await?;
    resources.save_results().await?;
    Ok(check_result)
}

pub async fn pr_add(
    resources: &ChatRepoResources,
    pr_id: u64,
    settings: PullRequestSettings,
) -> Result<(), Error> {
    {
        let mut locked = resources.settings.write().await;
        if locked.pull_requests.contains_key(&pr_id) {
            return Err(Error::PullRequestExists(pr_id));
        }
        locked.pull_requests.insert(pr_id, settings);
    }
    resources.save_settings().await
}

pub async fn pr_remove(resources: &ChatRepoResources, id: u64) -> Result<(), Error> {
    {
        let mut locked = resources.settings.write().await;
        if !locked.pull_requests.contains_key(&id) {
            return Err(Error::UnknownPullRequest(id));
        }
        locked.pull_requests.remove(&id);
    }
    resources.save_settings().await
}

pub async fn pr_check(
    resources: &ChatRepoResources,
    repo_resources: &RepoResources,
    id: u64,
) -> Result<Option<String>, Error> {
    log::info!("checking PR ({task}, {id})", task = resources.task);
    let github_info = {
        let locked = repo_resources.settings.read().await;
        locked
            .github_info
            .clone()
            .ok_or(Error::NoGitHubInfo(resources.task.repo.clone()))?
    };
    log::debug!("checking PR {github_info}#{id}");
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
    resources: &ChatRepoResources,
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

pub async fn branch_add(
    resources: &ChatRepoResources,
    branch: &str,
    settings: BranchSettings,
) -> Result<(), Error> {
    {
        let mut locked = resources.settings.write().await;
        if locked.branches.contains_key(branch) {
            return Err(Error::BranchExists(branch.to_owned()));
        }
        locked.branches.insert(branch.to_owned(), settings);
    }
    resources.save_settings().await
}

pub async fn branch_remove(resources: &ChatRepoResources, branch: &str) -> Result<(), Error> {
    {
        let mut locked = resources.settings.write().await;
        if !locked.branches.contains_key(branch) {
            return Err(Error::UnknownBranch(branch.to_owned()));
        }
        locked.branches.remove(branch);
    }
    {
        let mut locked = resources.results.write().await;
        locked.branches.remove(branch);
    }
    resources.save_settings().await?;
    resources.save_results().await
}

pub async fn branch_check(
    resources: &ChatRepoResources,
    repo_resources: &RepoResources,
    branch_name: &str,
) -> Result<BranchCheckResult, Error> {
    log::info!(
        "checking branch ({task}, {branch_name})",
        task = resources.task
    );
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
            let repo = repo_resources.repo.lock().await;
            let remote_branch_name = format!("origin/{branch_name}");

            match repo.find_branch(&remote_branch_name, BranchType::Remote) {
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
            }
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
