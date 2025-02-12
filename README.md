# commit-notifier

A simple telegram bot monitoring commit status.

## Usage (non-NixOS)

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

## Usage (NixOS)

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
