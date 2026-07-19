//! Binary entry point: parse the command line, install file logging and the
//! interrupt handler, then dispatch. All logic lives in the `hop` library
//! crate.

use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};

use clap::Parser;
use hop::cli::output::EXIT_INTERRUPTED;
use hop::cli::{self, Cli};
use hop::util::{logging, paths};

/// Set once `Ctrl+C` (or SIGTERM) arrived, so the run can end with 130.
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

fn main() -> ExitCode {
    let cli = Cli::parse();
    if cli.is_color_disabled() {
        disable_color();
    }
    // A TUI owns the terminal, so diagnostics go to a file, never stderr.
    let _ = logging::init(cli.log_level(), Some(&paths::log_file()));
    install_interrupt_handler();

    let code = cli::run(cli);
    if INTERRUPTED.load(Ordering::SeqCst) {
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

/// Records an interrupt so the run can end with the conventional 130.
///
/// The handler only sets a flag: the ZIP backup builds into a `.part` file and
/// renames it atomically, and the terminal guard restores the screen on drop,
/// so unwinding normally is what actually cleans up. Killing the process from
/// inside the handler would skip both.
fn install_interrupt_handler() {
    let _ = ctrlc::set_handler(|| {
        INTERRUPTED.store(true, Ordering::SeqCst);
    });
}
