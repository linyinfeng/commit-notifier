{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils-plus.url = "github:gytis-ivaskevicius/flake-utils-plus";
    naersk.url = "github:nmattia/naersk";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };
  outputs =
    inputs@{ self, nixpkgs, flake-utils-plus, naersk, rust-overlay }:
    let
      name = "commit-notifier";
      utils = flake-utils-plus.lib;
    in
    utils.mkFlake {
      inherit self inputs;

      channels.nixpkgs = {
        overlaysBuilder = _channels: [
          rust-overlay.overlay
        ];
      };

      outputsBuilder = channels:
        let
          pkgs = channels.nixpkgs;
          rust = pkgs.rust-bin.nightly.latest.default.override {
            extensions = [ "rust-src" "rust-analyzer-preview" ];
          };
          naersk-lib = naersk.lib.${pkgs.system}.override {
            cargo = rust;
            rustc = rust;
          };
        in
        rec {
          packages.${name} = naersk-lib.buildPackage {
            pname = name;
            root = ./.;

            buildInputs = with pkgs; [
              openssl
              sqlite
              libgit2
            ];
            nativeBuildInputs = with pkgs; [ pkg-config ];
          };
          defaultPackage = packages.${name};
          apps.${name} = utils.lib.mkApp { drv = packages.${name}; };
          defaultApp = apps.${name};

          devShell = pkgs.mkShell {
            inputsFrom = [ packages.${name} ];
            packages = with pkgs; [
              fup-repl
            ];
          };

          checks = packages;
        };
    };
}
