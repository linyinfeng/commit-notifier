use teloxide::{payloads::SendMessage, prelude::*, requests::JsonRequest};

pub fn reply_to_msg<T>(bot: &Bot, msg: &Message, text: T) -> JsonRequest<SendMessage>
where
    T: Into<String>,
{
    bot.send_message(msg.chat.id, text)
        .reply_to_message_id(msg.id)
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
