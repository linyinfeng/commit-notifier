use std::path::PathBuf;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
pub struct Options {
    #[structopt(short, long, parse(from_os_str))]
    pub working_dir: PathBuf,
    #[structopt(short, long)]
    pub cron: String,
}

pub static OPTIONS: once_cell::sync::OnceCell<Options> = once_cell::sync::OnceCell::new();

pub fn initialize() {
    once_cell::sync::OnceCell::set(&OPTIONS, Options::from_args()).unwrap();
}

pub fn get() -> &'static Options {
    OPTIONS.get().expect("options not initialized")
}
