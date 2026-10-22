use std::sync::LazyLock;

use regex::Regex;

use crate::error::Error;

static NAME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("^[a-zA-Z0-9_-]+$").expect("repository name regex"));

/// A repository name is used verbatim as a single path component below the
/// working directory (`repositories/<name>` and `chats/<chat>/<name>`), so
/// anything the path layer would reinterpret — separators, dots, colons — must
/// be rejected before it reaches `Path::join`.
pub fn validate(name: &str) -> Result<(), Error> {
    if NAME_RE.is_match(name) {
        Ok(())
    } else {
        Err(Error::Name(name.to_owned()))
    }
}

/// `clap` value parser for arguments that name an existing repository.
pub fn parse(name: &str) -> Result<String, Error> {
    validate(name)?;
    Ok(name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_names() {
        for name in ["nixpkgs", "home-manager", "poetry2nix", "a_b-1"] {
            assert!(validate(name).is_ok(), "'{name}' should be accepted");
        }
    }

    #[test]
    fn rejects_path_like_names() {
        for name in [
            "",
            "https:",
            "https://github.com/NixOS/nixpkgs",
            "a/b",
            "..",
            ".",
            "a b",
        ] {
            assert!(validate(name).is_err(), "'{name}' should be rejected");
        }
    }
}
