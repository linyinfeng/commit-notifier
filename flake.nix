{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils-plus.url = "github:gytis-ivaskevicius/flake-utils-plus";
  };
  outputs =
    inputs@{ self, nixpkgs, flake-utils-plus }:
    let
      name = "commit-notifier";
      utils = flake-utils-plus.lib;
    in
    utils.mkFlake {
      inherit self inputs;

      channels.nixpkgs.input = nixpkgs;

      outputsBuilder = channels:
        let
          pkgs = channels.nixpkgs;
        in
        rec {
          packages.${name} = pkgs.callPackage ./commit-notifier.nix { };
          defaultPackage = packages.${name};
          apps.${name} = utils.lib.mkApp { drv = packages.${name}; };
          defaultApp = apps.${name};

          devShell = pkgs.mkShell {
            packages = with pkgs; [
              fup-repl
              rustup
              pkg-config
              sqlite
              libgit2
              openssl
            ];
          };

          checks = packages;
        };

      overlay = final: prev: {
        commit-notifier = self.defaultPackage.${final.system};
      };
    };
}
