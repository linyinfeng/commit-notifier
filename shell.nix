{ pkgs ? import <nixpkgs> { } }:

pkgs.mkShell {
  packages = with pkgs; [
    rustup

    pkg-config
    sqlite
    libgit2
    openssl
  ];

  RUST_LOG = "info,commit-notifier=debug";
}
