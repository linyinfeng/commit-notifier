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
     --cron "0 */5 * * * *" \
     --admin-chat-id="{YOUR_ADMIN_CHAT_ID}"
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
        adminChatId = "{YOUR_ADMIN_CHAT_ID}";
        tokenFiles = {
            telegramBot = /path/to/telegram/bot/token;
            github = /path/to/github/token;
        };
    };
}
```

## Self-hosting (docker)

Docker images are published on GitHub package registry (<https://github.com/linyinfeng/commit-notifier/pkgs/container/commit-notifier>).

```console
$ docker run \
  --env "TELOXIDE_TOKEN={YOUR_TELEGRAM_BOT_TOKEN}" \
  --env "GITHUB_TOKEN={YOUR_GITHUB_TOKEN}" \
  --env "COMMIT_NOTIFIER_CRON=0 */5 * * * *" \
  --volume commit-notifier-data:/data \
  ghcr.io/linyinfeng/commit-notifier:latest
```

## Usage

The telegram bot has only one command `/notifier`. But this command provides a full CLI interface. Simply send `/notifier` to the bot without any arguments, the bot will send back the help information.

## Allow List

Currently the bot use `GITHUB_TOKEN` to check status for issues/pull requests, so only manually allowed users/groups can access the bot.

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
  $ mkdir -p {WORKING_DIR}/chats/888888888 # chat id
  ```

* For group chat

  ```console
  $ mkdir -p {WORKING_DIR}/chats/_1008888888888 # chat id (replace "-" with "_")
  ```

**Make sure the new directory is writable by `commit-notifier`.** All data (repositories, settings, check results) related to the chat will be saved in the directory.

## Migration Guide

### From `0.1.x` to `0.2.0`

There are several differences between `0.1.x` and `0.2.x`.

* In `0.1.x`, chats data are saved at `{WORKING_DIR}`; in `0.2.x`, chats data are saved in `{WORKING_DIR}/chats`.
* In `0.1.x`, repositories and their settings are managed by every chat; in `0.2.x`, repositories and their settings are saved in `{WORKING_DIR}/repositories`, and can only managed by the admin chat. Also, in `0.2.x`, repositories are shared between all chats.
* In `0.1.x`, caches are built in a per-commit manner; in `0.2.x`, caches are built in a per-branch manner, including every branch matches `--branch-regex`.

#### How to migrate

1. Backup old `{WORKING_DIR}` to `{BACKUP_DIR}`.
2. Start the bot.
3. Check old configurations in `{BACKUP_DIR}`, find all repositories.
4. In admin chat, manually run `/notifier repo-add ...` for each repositories.
5. Properly configure each repositories.

   * Use `/notifier repo-edit ...` to set branch regex. Use `/notifier condition-add ...` to set conditions.

   * Or just edit `repositories/{REPO_NAME}/settings.json` manually.

     <details>
     <summary>An example configuration for nixpkgs</summary>

     ```json
     {
       "branch_regex": "^(master|nixos-unstable|nixpkgs-unstable|staging|release-\\d\\d\\.\\d\\d|nixos-\\d\\d\\.\\d\\d)$",
       "github_info": {
         "owner": "nixos",
         "repo": "nixpkgs"
       },
       "conditions": {
         "in-nixos-release": {
            "condition": {
             "InBranch": {
               "branch_regex": "^nixos-\\d\\d\\.\\d\\d$"
             }
           }
         },
         "in-nixos-unstable": {
           "condition": {
             "InBranch": {
               "branch_regex": "^nixos-unstable$"
             }
           }
         },
         "master-to-staging": {
           "condition": {
             "SuppressFromTo": {
               "from_regex": "main",
               "to_regex": "staging(-next)?"
             }
           }
         }
       }
     }
     ```

     </details>

6. Wait for the first update (first-time cache building can be slow). Restart the bot to trigger update immediately.
7. Restore chat configurations. `rsync --recursive {BACKUP_DIR}/ {WORKING_DIR}/chats/ --exclude cache.sqlite --exclude lock --exclude repo --verbose` (trailing `/` is important.)
