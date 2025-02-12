{
  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs";

    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

    flake-utils.url = "github:numtide/flake-utils";
    flake-utils.inputs.systems.follows = "systems";

    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";

    crane.url = "github:ipetkov/crane";

    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";

    systems.url = "github:nix-systems/default";
  };

  outputs = inputs @ {flake-parts, ...}:
    flake-parts.lib.mkFlake {inherit inputs;}
    ({
      config,
      self,
      inputs,
      lib,
      getSystem,
      ...
    }: {
      systems = import inputs.systems;
      imports = [
        inputs.flake-parts.flakeModules.easyOverlay
        inputs.treefmt-nix.flakeModule
      ];
      flake = {
        nixosModules.commit-notifier = ./nixos/commit-notifier.nix;
      };
      perSystem = {
        config,
        self',
        pkgs,
        system,
        ...
      }: let
        craneLib = inputs.crane.mkLib pkgs;
        src = craneLib.cleanCargoSource (craneLib.path ./.);
        bareCommonArgs = {
          inherit src;
          nativeBuildInputs = with pkgs; [
            pkg-config
          ];
          buildInputs = with pkgs; [
            sqlite
            libgit2
            openssl
          ];
        };
        cargoArtifacts = craneLib.buildDepsOnly bareCommonArgs;
        commonArgs = bareCommonArgs // {inherit cargoArtifacts;};
      in {
        packages = {
          default = config.packages.commit-notifier;
          commit-notifier = craneLib.buildPackage commonArgs;
          dockerImage = pkgs.dockerTools.buildImage {
            name = "commit-notifier";
            tag = self.sourceInfo.rev or null;
            copyToRoot = pkgs.buildEnv {
              name = "commit-notifier-env";
              paths =
                (with pkgs; [
                  git
                  coreutils # for manual operations
                ])
                ++ (with pkgs.dockerTools; [
                  usrBinEnv
                  binSh
                  caCertificates
                ]);
            };
            config = {
              Entrypoint = ["${pkgs.tini}/bin/tini" "--"];
              Cmd = let
                start = pkgs.writeShellScript "start-commit-notifier" ''
                  exec ${config.packages.commit-notifier}/bin/commit-notifier \
                    --working-dir "/data" \
                    --cron "$COMMIT_NOTIFIER_CRON" \
                    $EXTRA_ARGS "$@"
                '';
              in ["${start}"];
              Env = [
                "TELOXIDE_TOKEN="
                "GITHUB_TOKEN="
                "RUST_LOG=commit_notifier=info"
                "COMMIT_NOTIFIER_CRON=0 */5 * * * *"
                "EXTRA_ARGS="
              ];
              WorkingDirectory = "/data";
              Volumes = {"/data" = {};};
              Labels =
                {
                  "org.opencontainers.image.title" = "commit-notifier";
                  "org.opencontainers.image.description" = "A simple telegram bot monitoring commit status";
                  "org.opencontainers.image.url" = "https://github.com/linyinfeng/commit-notifier";
                  "org.opencontainers.image.source" = "https://github.com/linyinfeng/commit-notifier";
                  "org.opencontainers.image.licenses" = "MIT";
                }
                // lib.optionalAttrs (self.sourceInfo ? rev) {
                  "org.opencontainers.image.revision" = self.sourceInfo.rev;
                };
            };
          };
        };
        overlayAttrs = {
          inherit (config.packages) commit-notifier;
        };
        checks = {
          inherit (self'.packages) commit-notifier dockerImage;
          doc = craneLib.cargoDoc commonArgs;
          fmt = craneLib.cargoFmt {inherit src;};
          nextest = craneLib.cargoNextest commonArgs;
          clippy = craneLib.cargoClippy (commonArgs
            // {
              cargoClippyExtraArgs = "--all-targets -- --deny warnings";
            });
        };
        treefmt = {
          projectRootFile = "flake.nix";
          programs = {
            alejandra.enable = true;
            rustfmt.enable = true;
            shfmt.enable = true;
          };
        };
        devShells.default = pkgs.mkShell {
          inputsFrom = lib.attrValues self'.checks;
          packages = with pkgs; [
            rustup
            rust-analyzer
          ];
        };
      };
    });
}
