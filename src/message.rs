use std::{collections::BTreeSet, fmt};

use teloxide::{types::Message, utils::markdown};

use crate::{
    chat::{
        results::{BranchCheckResult, CommitCheckResult},
        settings::{BranchSettings, CommitSettings, PRIssueSettings, Subscriber},
    },
    condition::Action,
    error::Error,
    repo::{pr_issue_url, resources::RepoResources},
    utils::empty_or_start_new_line,
};

pub fn commit_check_message(
    repo: &str,
    commit: &str,
    settings: &CommitSettings,
    result: &CommitCheckResult,
) -> String {
    format!(
        "{summary}
{details}",
        summary = commit_check_message_summary(repo, settings, result),
        details = markdown::expandable_blockquote(&commit_check_message_detail(
            repo, commit, settings, result
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

pub fn commit_check_message_detail(
    repo: &str,
    commit: &str,
    settings: &CommitSettings,
    result: &CommitCheckResult,
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
        "{repo}/`{commit}`{url}{notify}

*new* branches containing this commit:
{new}

*all* branches containing this commit:
{all}
{auto_remove_msg}
",
        repo = markdown::escape(repo),
        commit = markdown::escape(commit),
        url = settings
            .url
            .as_ref()
            .map(|u| format!("\n{}", markdown::escape(u.as_str())))
            .unwrap_or_default(),
        notify = empty_or_start_new_line(&settings.notify.notify_markdown()),
        new = markdown_list(result.new.iter()),
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
        notify = empty_or_start_new_line(&settings.notify.notify_markdown()),
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
        notify = empty_or_start_new_line(&settings.notify.notify_markdown()),
    ))
}

pub fn branch_check_message(
    repo: &str,
    branch: &str,
    settings: &BranchSettings,
    result: &BranchCheckResult,
) -> String {
    let status = if result.old == result.new {
        format!(
            "{}
\\(not changed\\)",
            markdown_optional_commit(result.new.as_deref())
        )
    } else {
        format!(
            "{old} \u{2192}
{new}",
            old = markdown_optional_commit(result.old.as_deref()),
            new = markdown_optional_commit(result.new.as_deref()),
        )
    };
    format!(
        "{repo}/`{branch}`
{status}{notify}
",
        repo = markdown::escape(repo),
        branch = markdown::escape(branch),
        notify = empty_or_start_new_line(&settings.notify.notify_markdown()),
    )
}

pub fn markdown_optional_commit(commit: Option<&str>) -> String {
    match &commit {
        None => "\\(nothing\\)".to_owned(),
        Some(c) => markdown::code_inline(&markdown::escape(c)),
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
