{
  pkgs,
  config,
  lib,
  ...
}: let
  cfg = config.services.commit-notifier;
in {
  options.services.commit-notifier = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Whether to enable commit-notifier service.
      '';
    };
    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.commit-notifier;
      defaultText = "pkgs.commit-notifier";
      description = ''
        commit-notifier derivation to use.
      '';
    };
    cron = lib.mkOption {
      type = lib.types.str;
      description = ''
        Update cron expression.
      '';
    };
    adminChatId = lib.mkOption {
      type = lib.types.str;
      description = ''
        Chat id of the admin chat.
      '';
    };
    tokenFiles = {
      telegramBot = lib.mkOption {
        type = lib.types.str;
        description = ''
          Telegram bot token file.
        '';
      };
      github = lib.mkOption {
        type = lib.types.str;
        description = ''
          GitHub token file.
        '';
      };
    };
    rustLog = lib.mkOption {
      type = lib.types.str;
      default = "info";
      description = ''
        RUST_LOG environment variable;
      '';
    };
    maintenance = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = ''
          Whether to consolidate the git packfiles of the tracked repositories.
          Every fetch of new objects writes another packfile, and nothing else
          consolidates them, so the number of packfiles would otherwise grow
          without bound and slow every fetch down.
        '';
      };
      calendar = lib.mkOption {
        type = lib.types.str;
        default = "daily";
        description = ''
          When to consolidate the git packfiles of the tracked repositories.
        '';
      };
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.services.commit-notifier = {
      description = "Git commit notifier";

      script = ''
        export TELOXIDE_TOKEN=$(cat "$CREDENTIALS_DIRECTORY/telegram-bot")
        export GITHUB_TOKEN=$(cat "$CREDENTIALS_DIRECTORY/github")

        "${cfg.package}/bin/commit-notifier" \
          --working-dir /var/lib/commit-notifier \
          --cron "${cfg.cron}" \
          --admin-chat-id="${cfg.adminChatId}"
      '';

      path = [
        pkgs.git
      ];

      serviceConfig = {
        User = "commit-notifier";
        Group = "commit-notifier";
        StateDirectory = "commit-notifier";
        LoadCredential = [
          "telegram-bot:${cfg.tokenFiles.telegramBot}"
          "github:${cfg.tokenFiles.github}"
        ];
      };

      environment."RUST_LOG" = cfg.rustLog;

      wantedBy = ["multi-user.target"];
      after = ["network-online.target"];
      requires = ["network-online.target"];
    };

    users.users.commit-notifier = {
      isSystemUser = true;
      group = "commit-notifier";
    };
    users.groups.commit-notifier = {};

    systemd.services.commit-notifier-maintenance = lib.mkIf cfg.maintenance.enable {
      description = "Consolidate git packfiles of commit-notifier repositories";

      script = ''
        shopt -s nullglob
        for repo in /var/lib/commit-notifier/repositories/*/repo; do
          echo "repacking $repo"
          git -C "$repo" repack --geometric=2 -d --write-midx -q
        done
      '';

      path = [
        pkgs.git
      ];

      serviceConfig = {
        User = "commit-notifier";
        Group = "commit-notifier";
        StateDirectory = "commit-notifier";
        Type = "oneshot";
        Nice = 10;
        IOSchedulingClass = "idle";
      };
    };

    systemd.timers.commit-notifier-maintenance = lib.mkIf cfg.maintenance.enable {
      wantedBy = ["timers.target"];
      timerConfig = {
        OnCalendar = cfg.maintenance.calendar;
        Persistent = true;
        RandomizedDelaySec = "1h";
      };
    };
  };
}
