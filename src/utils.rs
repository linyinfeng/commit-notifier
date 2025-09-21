use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter};
use std::path::Path;

use fs4::fs_std::FileExt;
use serde::Serialize;
use serde::de::DeserializeOwned;
use teloxide::types::ReplyParameters;
use teloxide::{payloads::SendMessage, prelude::*, requests::JsonRequest};

use crate::error::Error;

pub fn reply_to_msg<T>(bot: &Bot, msg: &Message, text: T) -> JsonRequest<SendMessage>
where
    T: Into<String>,
{
    bot.send_message(msg.chat.id, text)
        .reply_parameters(ReplyParameters::new(msg.id))
}

pub fn push_empty_line(s: &str) -> String {
    let trimmed = s.trim().to_string();
    if trimmed.is_empty() {
        trimmed
    } else {
        let mut result = "\n\n".to_string();
        result.push_str(&trimmed);
        result
    }
}

pub fn read_json<P, T>(path: P) -> Result<T, Error>
where
    P: AsRef<Path> + fmt::Debug,
    T: Serialize + DeserializeOwned + Default,
{
    if !path.as_ref().is_file() {
        log::info!("auto create file: {path:?}");
        write_json::<_, T>(&path, &Default::default())?;
    }
    log::debug!("read from file: {path:?}");
    let file = File::open(path)?;
    // TODO lock_shared maybe added to the std lib in the future
    FileExt::lock_shared(&file)?; // close of file automatically release the lock
    let reader = BufReader::new(file);
    Ok(serde_json::from_reader(reader)?)
}

pub fn write_json<P, T>(path: P, rs: &T) -> Result<(), Error>
where
    P: AsRef<Path> + fmt::Debug,
    T: Serialize,
{
    log::debug!("write to file: {path:?}");
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    file.lock_exclusive()?;
    let writer = BufWriter::new(file);
    Ok(serde_json::to_writer_pretty(writer, rs)?)
}
