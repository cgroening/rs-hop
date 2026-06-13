//! Launches external programs (the git tool, an editor) for a repo or file.
//!
//! Each launcher runs the child to completion with the terminal handed over, so
//! callers must restore the terminal (drop the `Tui` guard) before calling. The
//! command is built with explicit `.arg()` values (never a shell string) so
//! paths with spaces are safe.

use std::io;
use std::path::Path;
use std::process::{Command, ExitStatus};

/// Runs the git tool `program` with its working directory set to `dir`.
///
/// # Errors
/// Returns an I/O error if the program cannot be spawned (e.g. not installed).
pub fn launch_git_tool(program: &str, dir: &Path) -> io::Result<ExitStatus> {
    Command::new(program).current_dir(dir).status()
}

/// Opens `file` in `editor`, with the working directory set to its parent.
///
/// # Errors
/// Returns an I/O error if the editor cannot be spawned.
pub fn open_in_editor(editor: &str, file: &Path) -> io::Result<ExitStatus> {
    let mut command = Command::new(editor);
    command.arg(file);
    if let Some(parent) = file.parent() {
        command.current_dir(parent);
    }
    command.status()
}

/// Resolves the editor to use: an explicit `configured` value, then `$VISUAL`,
/// then `$EDITOR`, falling back to `vi`.
pub fn resolve_editor(configured: Option<&str>) -> String {
    if let Some(editor) = configured
        && !editor.trim().is_empty()
    {
        return editor.to_string();
    }
    for var in ["VISUAL", "EDITOR"] {
        if let Ok(value) = std::env::var(var)
            && !value.trim().is_empty()
        {
            return value;
        }
    }
    "vi".to_string()
}
