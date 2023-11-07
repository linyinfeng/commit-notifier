use crate::condition;
use crate::error::Error;
use crate::github::GitHubInfo;
use clap::ColorChoice;
use clap::Parser;
use std::{ffi::OsString, iter};

#[derive(Debug, Parser)]
#[command(name = "/notifier",
            author,
            version,
            about,
            color = ColorChoice::Never,
            no_binary_name = true,
)]
pub enum Notifier {
    #[command(about = "add a repository")]
    RepoAdd { name: String, url: String },
    #[command(about = "edit settings of a repository")]
    RepoEdit {
        name: String,
        #[arg(long, short)]
        branch_regex: Option<String>,
        #[arg(long, short, value_parser = GitHubInfo::parse, group = "edit_github_info")]
        github_info: Option<GitHubInfo>,
        #[arg(long, group = "edit_github_info")]
        clear_github_info: bool,
    },
    #[command(about = "remove a repository")]
    RepoRemove { name: String },
    #[command(about = "add a commit")]
    CommitAdd {
        repo: String,
        hash: String,
        #[arg(long, short)]
        comment: String,
    },
    #[command(about = "remove a commit")]
    CommitRemove { repo: String, hash: String },
    #[command(about = "fire a commit check immediately")]
    CommitCheck { repo: String, hash: String },
    #[command(about = "add a pull request")]
    PrAdd {
        repo: String,
        pr: u64,
        #[arg(long, short)]
        comment: Option<String>,
    },
    #[command(about = "remove a pull request")]
    PrRemove { repo: String, pr: u64 },
    #[command(about = "check a pull request")]
    PrCheck { repo: String, pr: u64 },
    #[command(about = "add a branch")]
    BranchAdd { repo: String, branch: String },
    #[command(about = "remove a branch")]
    BranchRemove { repo: String, branch: String },
    #[command(about = "fire a branch check immediately")]
    BranchCheck { repo: String, branch: String },
    #[command(about = "add an auto clean condition")]
    ConditionAdd {
        repo: String,
        identifier: String,
        #[arg(value_enum, short = 't', long = "type")]
        kind: condition::Kind,
        #[arg(short, long = "expr")]
        expression: String,
    },
    #[command(about = "remove an auto clean condition")]
    ConditionRemove { repo: String, identifier: String },
    #[command(about = "manually trigger an auto clean condition check")]
    ConditionTrigger { repo: String, identifier: String },
    #[command(about = "list repositories and commits")]
    List,
}

pub fn parse(raw_input: String) -> Result<Notifier, Error> {
    let input = parse_raw(raw_input)?.into_iter().map(OsString::from);
    Ok(Notifier::try_parse_from(input)?)
}

#[derive(Debug)]
enum PRState {
    Out,
    InWord,
    InSimpleQuote { end_mark: char },
    Escape,
}

pub fn parse_raw(raw_input: String) -> Result<Vec<String>, Error> {
    let mut state = PRState::Out;
    let mut chars = raw_input.chars().chain(iter::once('\0')).peekable();
    let mut current = Vec::new();
    let mut result = Vec::new();

    while let Some(c) = chars.peek().cloned() {
        let mut next = true;
        state = match state {
            PRState::Out => {
                if c == '\0' || c.is_whitespace() {
                    PRState::Out
                } else {
                    next = false;
                    PRState::InWord
                }
            }
            PRState::InWord => match c {
                _ if c == '\0' || c.is_whitespace() => {
                    result.push(current.into_iter().collect());
                    current = Vec::new();
                    PRState::Out
                }
                '\'' | '"' => PRState::InSimpleQuote { end_mark: c },
                '\\' => PRState::Escape,
                '—' => {
                    current.push('-');
                    current.push('-');
                    PRState::InWord
                }
                _ => {
                    current.push(c);
                    PRState::InWord
                }
            },
            PRState::InSimpleQuote { end_mark } => match c {
                _ if c == end_mark => PRState::InWord,
                '\0' => return Err(Error::UnclosedQuote),
                _ => {
                    current.push(c);
                    PRState::InSimpleQuote { end_mark }
                }
            },
            PRState::Escape => match c {
                '\0' => return Err(Error::BadEscape),
                _ => {
                    current.push(c);
                    PRState::InWord
                }
            },
        };
        if next {
            chars.next();
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_raw_simple() {
        assert_eq!(
            parse_raw(
                "some    simple    command —some-option --some-simple=values -b \"a\"b'c'"
                    .to_owned()
            )
            .unwrap(),
            vec![
                "some",
                "simple",
                "command",
                "--some-option",
                "--some-simple=values",
                "-b",
                "abc",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec::<_>>()
        );
    }

    #[test]
    fn parse_raw_escape() {
        assert_eq!(
            parse_raw("\\a\\b\\ aaaaa bbbb".to_owned()).unwrap(),
            vec!["ab aaaaa", "bbbb"]
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec::<_>>()
        );
    }

    #[test]
    fn verify_cli() {
        use clap::CommandFactory;
        Notifier::command().debug_assert()
    }
}
