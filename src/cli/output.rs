//! CLI output helpers: error reporting at the binary edge, via sparcli.

use std::io;
use std::process::ExitCode;

use sparcli::{Alert, Renderable};

use crate::domain::error::Error;

/// Prints `error` as a sparcli error alert to stderr and returns a failure code.
pub fn report_error(error: &Error) -> ExitCode {
    let _ = Alert::error(error.to_string()).print_to(&mut io::stderr().lock());
    ExitCode::FAILURE
}

/// Prints a sparcli error alert to stderr and returns a failure exit code.
pub fn fail(message: &str) -> ExitCode {
    let _ =
        Alert::error(message.to_string()).print_to(&mut io::stderr().lock());
    ExitCode::FAILURE
}
