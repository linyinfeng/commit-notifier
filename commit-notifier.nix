{ rustPlatform, lib, pkg-config, openssl, libgit2, sqlite }:

rustPlatform.buildRustPackage
{
  pname = "commit-notifier";
  version = "0.1.0";

  src = ./.;
  cargoLock.lockFile = ./Cargo.lock;

  nativeBuildInputs = [
    pkg-config
  ];
  buildInputs = [
    openssl
    sqlite
    libgit2
  ];

  meta = with lib; {
    license = licenses.mit;
  };
}
