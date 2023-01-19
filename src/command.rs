use crate::error::Error;
use std::{ffi::OsString, iter};
use clap::Parser;
use clap::ColorChoice;

#[derive(Debug, Parser)]
#[command(name = "/notifier",
            author,
            version,
            about,
            color = ColorChoice::Never,
            no_binary_name = true,
)]
pub enum Notifier {
    // #[subcommand(about = "add a repository")]
    RepoAdd {
        name: String,
        #[structopt(long, short)]
        url: String,
    },
    // #[structopt(about = "edit settings of a repository")]
    RepoEdit {
        name: String,
        #[arg(long, short)]
        branch_regex: Option<String>,
    },
    // #[structopt(about = "remove a repository")]
    RepoRemove { name: String },
    // #[structopt(about = "add a commit")]
    CommitAdd {
        repo: String,
        hash: String,
        #[structopt(long, short)]
        comment: String,
    },
    // #[structopt(about = "remove a commit")]
    CommitRemove { repo: String, hash: String },
    // #[structopt(about = "fire a commit check immediately")]
    CommitCheck { repo: String, hash: String },
    // #[structopt(about = "add a branch")]
    BranchAdd { repo: String, branch: String },
    // #[structopt(about = "remove a branch")]
    BranchRemove { repo: String, branch: String },
    // #[structopt(about = "fire a branch check immediately")]
    BranchCheck { repo: String, branch: String },
    // #[structopt(about = "list repositories and commits")]
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
