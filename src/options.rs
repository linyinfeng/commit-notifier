use std::lazy::SyncOnceCell;
use std::path::PathBuf;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
pub struct Options {
    #[structopt(short, long, parse(from_os_str))]
    pub working_dir: PathBuf,
    #[structopt(short, long)]
    pub cron: String,
}

pub static OPTIONS: SyncOnceCell<Options> = SyncOnceCell::new();

pub fn initialize() {
    SyncOnceCell::set(&OPTIONS, Options::from_args()).unwrap();
}

pub fn get() -> &'static Options {
    OPTIONS.get().expect("options not initialized")
}
