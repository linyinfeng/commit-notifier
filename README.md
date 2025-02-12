# commit-notifier

A simple telegram bot monitoring commit status.

## Self-hosting (non-NixOS)

1. Build the project with `cargo build`.

2. Specify Telegram bot token and GitHub token through environment variables

   GitHub token are used to check PR merge state and get merge commit, so no permission are required.

   ```console
   $ export TELOXIDE_TOKEN="{YOUR_TELEGRAM_BOT_TOKEN}"
   $ export GITHUB_TOKEN="{YOUR_GITHUB_TOKEN}"
   ```

3. Start `commit-notifier`.

   ```console
   $ commit-notifier" \
     --working-dir /var/lib/commit-notifier \
     --cron "0 */5 * * * *"
   ```

   Automatic check will be triggered based on the cron expression. In the example, `0 */5 * * * *` means "at every 5th minute". cron documentation: <https://docs.rs/cron/latest/cron>.

## Self-hosting (NixOS)

This repository is a Nix flake.

### Outputs

1. Main package: `packages.${system}.commit-notifier`.
2. Overlay: `overlays.default` (contains `commit-notifier`).
3. NixOS module: `nixosModules.commit-notifier`.

### NixOS module example

My instance: <https://github.com/linyinfeng/dotfiles/blob/main/nixos/profiles/services/commit-notifier/default.nix>

```nix
{
    services.commit-notifier = {
        enable = true;
        cron = "0 */5 * * * *";
        tokenFiles = {
            telegramBot = /path/to/telegram/bot/token;
            github = /path/to/github/token;
        };
    };
}
```

## Usage

The telegram bot has only one command `/notifier`. But this command provides a full CLI interface. Simply send `/notifier` to the bot without any arguments, the bot will send back the help information.

## Allow List

Since the bot can clone any git repository into its working directory, only manually allowed users/groups can access the bot.

The bot in a new chat returns this kind of error:

* Direct chat

  ```text
  chat id 888888888 is not in allow list
  ```

* Group chat

  ```text
  chat id -1008888888888 is not in allow list
  ```

Currently, the bot does not have an admin interface in telegram. So adding chats to the "allow list" requires manual operation: making a new directory.

* For direct chat:

  ```console
  $ cd /var/lib/commit-notifier
  $ mkdir 888888888 # chat id
  ```

* For group chat

  ```console
  $ cd /var/lib/commit-notifier
  $ mkdir _1008888888888 # chat id (replace "-" with "_")
  ```

**Make sure the new directory is writable by `commit-notifier`.** All data (repositories, settings, check results) related to the chat will be saved in the directory.
