use regex::Regex;
use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::error::Error;
use crate::options;
use std::fs;

pub struct Paths {
    pub outer: PathBuf,
    pub repo: PathBuf,
    pub cache: PathBuf,
    pub settings: PathBuf,
    pub results: PathBuf,
}

#[derive(Debug)]
pub struct CheckResult {
    pub all: BTreeSet<String>,
    pub new: BTreeSet<String>,
}

static NAME_RE: once_cell::sync::Lazy<Regex> =
    once_cell::sync::Lazy::new(|| Regex::new("^[a-zA-Z0-9_\\-]*$").unwrap());

pub fn get(chat: i64, repo: &str) -> Result<Paths, Error> {
    if !NAME_RE.is_match(repo) {
        return Err(Error::Name(repo.to_owned()));
    }

    let chat_working_dir = chat_dir(chat);
    if !chat_working_dir.is_dir() {
        Err(Error::NotInAllowList(chat))
    } else {
        let outer_dir = chat_working_dir.join(repo);
        Ok(Paths {
            outer: outer_dir.clone(),
            repo: outer_dir.join("repo"),
            cache: outer_dir.join("cache.sqlite"),
            settings: outer_dir.join("settings.json"),
            results: outer_dir.join("results.json"),
        })
    }
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

pub fn chats() -> Result<BTreeSet<i64>, Error> {
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

pub fn repos(chat: i64) -> Result<BTreeSet<String>, Error> {
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
