use std::{collections::BTreeSet, fmt};

use teloxide::{types::Message, utils::markdown};

use crate::{
    chat::{
        results::{BranchCheckResult, CommitCheckResult, ConditionCheckResult},
        settings::{BranchSettings, CommitSettings, PullRequestSettings, Subscriber},
    },
    error::Error,
    utils::push_empty_line,
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
    format!(
        "\\[{repo}\\] {comment} \\+{new}",
        repo = markdown::escape(repo),
        comment = markdown::escape(&settings.notify.comment),
        new = markdown_list_compat(result.new.iter()),
    )
}

pub fn commit_check_message_detail(
    repo: &str,
    commit: &str,
    settings: &CommitSettings,
    result: &CommitCheckResult,
) -> String {
    let auto_remove_msg = match &result.removed_by_condition {
        None => String::new(),
        Some(condition) => format!(
            "\n*auto removed* by condition: `{}`",
            markdown::escape(condition)
        ),
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
        notify = push_empty_line(&settings.notify.notify_markdown()),
        new = markdown_list(result.new.iter()),
        all = markdown_list(result.all.iter())
    )
}

pub fn pr_merged_message(
    repo: &str,
    pr: u64,
    settings: &PullRequestSettings,
    commit: &String,
) -> String {
    format!(
        "{repo}/{pr}
        merged as `{commit}`{notify}
",
        notify = push_empty_line(&settings.notify.notify_markdown()),
    )
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
        notify = push_empty_line(&settings.notify.notify_markdown()),
    )
}

pub fn condition_check_message(
    repo: &str,
    identifier: &str,
    result: &ConditionCheckResult,
) -> String {
    format!(
        "{repo}/`{identifier}`

branches removed by this condition:
{removed}
",
        repo = markdown::escape(repo),
        identifier = markdown::escape(identifier),
        removed = markdown_list(result.removed.iter()),
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
    match &msg.from {
        None => None,
        Some(u) => u.username.as_ref().map(|name| Subscriber::Telegram {
            username: name.to_string(),
        }),
    }
}

pub fn modify_subscriber_set(
    set: &mut BTreeSet<Subscriber>,
    subscriber: Subscriber,
    unsubscribe: bool,
) -> Result<(), Error> {
    if unsubscribe {
        if !set.contains(&subscriber) {
            return Err(Error::NotSubscribed);
        }
        set.remove(&subscriber);
    } else {
        if set.contains(&subscriber) {
            return Err(Error::AlreadySubscribed);
        }
        set.insert(subscriber);
    }
    Ok(())
}
