use std::{collections::BTreeSet, fmt};

use teloxide::{types::Message, utils::markdown};

use crate::{
    chat::{
        results::{BranchCheckResult, CommitCheckResult},
        settings::{BranchSettings, CommitSettings, PRIssueSettings, Subscriber},
    },
    condition::Action,
    error::Error,
    github::GitHubInfo,
    repo::{pr_issue_url, resources::RepoResources},
    utils::empty_or_start_new_line,
};

pub fn commit_check_message(
    repo: &str,
    commit: &str,
    settings: &CommitSettings,
    result: &CommitCheckResult,
    mention: bool,
) -> String {
    format!(
        "{summary}
{details}",
        summary = commit_check_message_summary(repo, settings, result),
        details = markdown::expandable_blockquote(&commit_check_message_additional(
            commit, settings, result, mention
        )),
    )
}

pub fn commit_check_message_summary(
    repo: &str,
    settings: &CommitSettings,
    result: &CommitCheckResult,
) -> String {
    let escaped_comment = markdown::escape(&settings.notify.comment);
    let comment_link = match &settings.url {
        Some(url) => markdown::link(url.as_ref(), &escaped_comment),
        None => escaped_comment,
    };
    format!(
        "\\[{repo}\\] {comment_link} \\+{new}",
        repo = markdown::escape(repo),
        new = markdown_list_compat(result.new.iter()),
    )
}

pub fn commit_check_message_additional(
    commit: &str,
    settings: &CommitSettings,
    result: &CommitCheckResult,
    mention: bool,
) -> String {
    let remove_conditions: BTreeSet<&String> = result.conditions_of_action(Action::Remove);
    let auto_remove_msg = if remove_conditions.is_empty() {
        "".to_string()
    } else {
        format!(
            "\n*auto removed* by conditions:
{}",
            markdown_list(remove_conditions.iter())
        )
    };
    format!(
        "`{commit}`{notify}

*all* branches containing this commit:
{all}
{auto_remove_msg}
",
        commit = markdown::escape(commit),
        notify = if mention {
            empty_or_start_new_line(&settings.notify.subscribers_markdown())
        } else {
            "".to_string()
        },
        all = markdown_list(result.all.iter())
    )
}

pub async fn pr_issue_id_pretty(resources: &RepoResources, id: u64) -> Result<String, Error> {
    let url = pr_issue_url(resources, id).await?;
    Ok(markdown::link(
        url.as_ref(),
        &format!("{repo}/{id}", repo = resources.name),
    ))
}

pub async fn pr_issue_merged_message(
    resources: &RepoResources,
    id: u64,
    settings: &PRIssueSettings,
    commit: &String,
) -> Result<String, Error> {
    Ok(format!(
        "{pretty_id} merged as `{commit}`{notify}",
        pretty_id = pr_issue_id_pretty(resources, id).await?,
        notify = empty_or_start_new_line(&settings.notify.subscribers_markdown()),
    ))
}

pub async fn pr_issue_closed_message(
    resources: &RepoResources,
    id: u64,
    settings: &PRIssueSettings,
) -> Result<String, Error> {
    Ok(format!(
        "{pretty_id} has been closed{notify}",
        pretty_id = pr_issue_id_pretty(resources, id).await?,
        notify = empty_or_start_new_line(&settings.notify.subscribers_markdown()),
    ))
}

pub fn branch_check_message(
    repo: &str,
    branch: &str,
    settings: &BranchSettings,
    result: &BranchCheckResult,
    github_info: Option<&GitHubInfo>,
) -> String {
    let status = if result.old == result.new {
        format!(
            "{}
\\(not changed\\)",
            markdown_optional_commit(result.new.as_deref(), github_info)
        )
    } else if let (Some(info), Some(old), Some(new)) = (github_info, &result.old, &result.new) {
        github_commit_diff(info, old, new)
    } else {
        format!(
            "{old} \u{2192}
{new}",
            old = markdown_optional_commit(result.old.as_deref(), github_info),
            new = markdown_optional_commit(result.new.as_deref(), github_info),
        )
    };
    format!(
        "{repo}/`{branch}`
{status}{notify}
",
        repo = markdown::escape(repo),
        branch = markdown::escape(branch),
        notify = empty_or_start_new_line(&settings.notify.subscribers_markdown()),
    )
}

const SHORT_COMMIT_LENGTH: usize = 11;

pub fn short_commit(commit: &str) -> &str {
    &commit[..SHORT_COMMIT_LENGTH.min(commit.len())]
}

pub fn github_commit_diff(github_info: &GitHubInfo, old: &str, new: &str) -> String {
    let GitHubInfo { owner, repo, .. } = github_info;
    let old_short = short_commit(old);
    let new_short = short_commit(new);
    let url = format!("https://github.com/{owner}/{repo}/compare/{old}...{new}");
    let text = format!("{old_short}...{new_short}");
    markdown::link(&url, &markdown::escape(&text))
}

pub fn markdown_optional_commit(commit: Option<&str>, github_info: Option<&GitHubInfo>) -> String {
    match &commit {
        None => "\\(nothing\\)".to_owned(),
        Some(commit) => match github_info {
            Some(info) => {
                let GitHubInfo { owner, repo, .. } = info;
                let short = short_commit(commit);
                let url = format!("https://github.com/{owner}/{repo}/commit/{short}");
                markdown::link(&url, &markdown::escape(short))
            }
            None => markdown::code_inline(&markdown::escape(commit)),
        },
    }
}

pub fn markdown_list<Iter, T>(items: Iter) -> String
where
    Iter: Iterator<Item = T>,
    T: fmt::Display,
{
    let mut result = String::new();
    for item in items {
        result.push_str(&format!("\\- `{}`\n", markdown::escape(&item.to_string())));
    }
    if result.is_empty() {
        "\u{2205}".to_owned() // the empty set symbol
    } else {
        assert_eq!(result.pop(), Some('\n'));
        result
    }
}

pub fn markdown_list_compat<Iter, T>(items: Iter) -> String
where
    Iter: Iterator<Item = T>,
    T: fmt::Display,
{
    let mut result = String::new();
    for item in items {
        result.push_str(&format!("`{}` ", markdown::escape(&item.to_string())));
    }
    if result.is_empty() {
        "\u{2205}".to_owned() // the empty set symbol
    } else {
        assert_eq!(result.pop(), Some(' '));
        result
    }
}

pub fn subscriber_from_msg(msg: &Message) -> Option<Subscriber> {
    msg.from.as_ref().map(Subscriber::from_tg_user)
}
