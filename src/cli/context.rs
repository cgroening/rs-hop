//! What the command handlers need to know about the terminal they run in.
//!
//! The TTY gate is resolved once here rather than re-queried at each call site,
//! so every command agrees on whether it may prompt and whether it may style
//! its output.

use sparcli::terminal;

/// The terminal facts a command handler decides on.
#[derive(Debug, Clone, Copy)]
pub struct CliContext {
    /// Whether a prompt may be shown: both stdin and stdout are terminals.
    ///
    /// Off a terminal a command must fail early with a clear message rather
    /// than block - a program waiting for input inside a pipeline cannot be
    /// told apart from one that has hung.
    pub is_interactive: bool,
    /// Whether stdout is a terminal, so styled tables and borders make sense.
    pub is_output_tty: bool,
}

impl CliContext {
    /// Reads the terminal state of the current process.
    pub fn detect() -> Self {
        CliContext {
            is_interactive: terminal::is_input_tty(),
            is_output_tty: terminal::is_output_tty(),
        }
    }
}
