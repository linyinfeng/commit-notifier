use clap::Parser;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(author, version, about, long_about = None)]
pub struct Options {
    #[arg(short, long)]
    pub working_dir: PathBuf,
    #[arg(short, long)]
    pub cron: String,
}

pub static OPTIONS: once_cell::sync::OnceCell<Options> = once_cell::sync::OnceCell::new();

pub fn initialize() {
    once_cell::sync::OnceCell::set(&OPTIONS, Options::parse()).unwrap();
}

pub fn get() -> &'static Options {
    OPTIONS.get().expect("options not initialized")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_cli() {
        use clap::CommandFactory;
        Options::command().debug_assert()
    }
}
