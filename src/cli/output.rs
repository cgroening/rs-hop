//! CLI output helpers: error reporting at the binary edge.

use std::process::ExitCode;

use crate::domain::error::Error;
use crate::util::app_info::APP_NAME;

/// Prints `error` to stderr and returns a failure exit code.
pub fn report_error(error: &Error) -> ExitCode {
    eprintln!("{APP_NAME}: {error}");
    ExitCode::FAILURE
}

/// Prints a plain error line and returns a failure exit code.
pub fn fail(message: &str) -> ExitCode {
    eprintln!("{APP_NAME}: {message}");
    ExitCode::FAILURE
}
