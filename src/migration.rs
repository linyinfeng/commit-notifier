use std::{collections::BTreeSet, mem};

use teloxide::utils::markdown;

use crate::{
    chat::{
        self,
        settings::{NotifySettings, Subscriber},
    },
    error::Error,
};

pub async fn from_0_2_1() -> Result<(), Error> {
    fn migrate_notify_settings(settings: &mut NotifySettings) {
        let mut subscribers = BTreeSet::new();
        mem::swap(&mut subscribers, &mut settings.subscribers);
        for subscriber in subscribers {
            match subscriber {
                Subscriber::Telegram { markdown_mention } => {
                    if markdown_mention.starts_with("@") {
                        let new_mention = markdown::escape(&markdown_mention);
                        if new_mention != markdown_mention {
                            log::info!("escape username '{markdown_mention}' to '{new_mention}'");
                        }
                        settings.subscribers.insert(Subscriber::Telegram {
                            markdown_mention: new_mention,
                        });
                    } else {
                        settings
                            .subscribers
                            .insert(Subscriber::Telegram { markdown_mention });
                    }
                }
            }
        }
    }

    log::info!("migration from version 0.2.1");
    let chats = chat::chats().await?;
    for chat in chats.iter().cloned() {
        log::info!("migrating chat {chat}...");
        let repos = chat::repos(chat).await?;
        for repo in repos {
            log::info!("migrating repo {chat}/{repo}...");
            let resources = chat::resources_chat_repo(chat, repo.to_string()).await?;
            let mut settings = resources.settings.write().await;
            for (_, settings) in settings.branches.iter_mut() {
                migrate_notify_settings(&mut settings.notify)
            }
            for (_, settings) in settings.commits.iter_mut() {
                migrate_notify_settings(&mut settings.notify)
            }
            for (_, settings) in settings.pr_issues.iter_mut() {
                migrate_notify_settings(&mut settings.notify)
            }
        }
    }

    // no errors, save all settings at once
    for chat in chats {
        let repos = chat::repos(chat).await?;
        for repo in repos {
            let resources = chat::resources_chat_repo(chat, repo.to_string()).await?;
            resources.save_settings().await?;
        }
    }
    Ok(())
}
