//! Binary entry point: parse the command line, install file logging and
//! dispatch. All logic lives in the `hop` library crate.

use std::process::ExitCode;

use clap::Parser;
use hop::cli::{self, Cli};
use hop::util::{logging, paths};
use log::LevelFilter;

fn main() -> ExitCode {
    let cli = Cli::parse();
    // A TUI owns the terminal, so diagnostics go to a file, never stderr.
    let _ = logging::init(LevelFilter::Info, Some(&paths::log_file()));
    cli::run(cli)
}
