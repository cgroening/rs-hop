//! Binary entry point: parse the command line, install file logging and the
//! interrupt handler, then dispatch. All logic lives in the `hop` library
//! crate.

use std::process::ExitCode;

use clap::Parser;
use hop::cli::output::EXIT_INTERRUPTED;
use hop::cli::{self, Cli};
use hop::util::{interrupt, logging, paths};

fn main() -> ExitCode {
    let cli = Cli::parse();
    if cli.is_color_disabled() {
        disable_color();
    }
    // A TUI owns the terminal, so diagnostics go to a file, never stderr.
    let _ = logging::init(cli.log_level(), Some(&paths::log_file()));
    interrupt::install();

    let code = cli::run(cli);
    if interrupt::is_interrupted() {
        return ExitCode::from(EXIT_INTERRUPTED);
    }
    code
}

/// Suppresses colored output for this process and anything it launches.
///
/// `NO_COLOR` is the variable sparcli already consults, so setting it keeps a
/// single place deciding on colour rather than adding a second, private flag.
fn disable_color() {
    // SAFETY: called once at the very top of `main`, before the logger, the
    // interrupt handler or any worker thread exists, so no other thread can
    // observe the environment while it changes.
    unsafe { std::env::set_var("NO_COLOR", "1") };
}
