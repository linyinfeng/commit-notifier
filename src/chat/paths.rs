use std::{path::PathBuf, sync::LazyLock};

use teloxide::types::ChatId;

use crate::{chat::Task, options};

#[derive(Debug, Clone)]
pub struct ChatRepoPaths {
    pub chat: PathBuf,
    pub repo: PathBuf,
    pub settings: PathBuf,
    pub results: PathBuf,
}

pub static GLOBAL_CHATS_OUTER: LazyLock<PathBuf> =
    LazyLock::new(|| options::get().working_dir.join("chats"));

impl ChatRepoPaths {
    pub fn new(task: &Task) -> ChatRepoPaths {
        let chat_path = GLOBAL_CHATS_OUTER.join(Self::outer_dir_name(task.chat));
        let repo = chat_path.join(&task.repo);
        Self {
            chat: chat_path,
            settings: repo.join("settings.json"),
            results: repo.join("results.json"),
            repo,
        }
    }

    fn outer_dir_name(chat: ChatId) -> PathBuf {
        let ChatId(num) = chat;
        let chat_dir_name = if num < 0 {
            format!("_{}", num.unsigned_abs())
        } else {
            format!("{chat}")
        };
        chat_dir_name.into()
    }
}
