{ pkgs, config, lib, ... }:

let
  cfg = config.services.commit-notifier;
  pre = pkgs.writeShellScript "commit-notifier-pre" ''
    set -x
    user="$1"
    "${pkgs.coreutils}/bin/cp" "${cfg.tokenFile}" /run/commit-notifier/token --verbose
    "${pkgs.coreutils}/bin/chown" commit-notifier /run/commit-notifier/token --verbose
  '';
in
{
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
    tokenFile = lib.mkOption {
      type = lib.types.str;
      description = ''
        Token file for commit-notifier.
      '';
    };
    rustLog = lib.mkOption {
      type = lib.types.str;
      default = "info";
      description = ''
        RUST_LOG environment variable;
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.services.commit-notifier = {
      description = "Git commit notifier";

      script = ''
        export TELOXIDE_TOKEN=$(cat /run/commit-notifier/token)

        "${cfg.package}/bin/commit-notifier" \
          --working-dir /var/lib/commit-notifier \
          --cron "${cfg.cron}"
      '';

      path = [
        pkgs.git
      ];

      serviceConfig = {
        DynamicUser = true;
        PermissionsStartOnly = true;
        RuntimeDirectory = "commit-notifier";
        StateDirectory = "commit-notifier";
        Restart = "on-failure";
        ExecStartPre = "${pre}";
      };

      environment."RUST_LOG" = cfg.rustLog;

      wantedBy = [ "multi-user.target" ];
    };
  };
}
