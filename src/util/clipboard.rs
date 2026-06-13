//! Best-effort clipboard copy via the system tool, dependency-free.
//!
//! The text is piped to a platform tool (`pbcopy` on macOS, `clip` on Windows,
//! `wl-copy`/`xclip`/`xsel` on Linux) rather than pulling in a clipboard crate.
//! When no tool is available the caller surfaces it as a status message.

use std::io::Write;
use std::process::{Command, Stdio};

use crate::domain::error::{Error, Result};

/// Copies `text` to the system clipboard, trying each platform tool in turn.
///
/// # Errors
/// Returns an error when no clipboard tool can be spawned or written to.
pub fn copy(text: &str) -> Result<()> {
    let mut last: Option<std::io::Error> = None;
    for (program, args) in candidates() {
        match pipe_to(program, args, text) {
            Ok(()) => return Ok(()),
            Err(error) => last = Some(error),
        }
    }
    Err(match last {
        Some(error) => Error::io("spawn clipboard tool", error),
        None => Error::invalid("no clipboard tool configured"),
    })
}

/// The clipboard tools to try for the current platform, in order.
fn candidates() -> &'static [(&'static str, &'static [&'static str])] {
    #[cfg(target_os = "macos")]
    {
        &[("pbcopy", &[])]
    }
    #[cfg(target_os = "windows")]
    {
        &[("clip", &[])]
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        &[
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
        ]
    }
}

/// Spawns `program args` and writes `text` to its stdin.
fn pipe_to(program: &str, args: &[&str], text: &str) -> std::io::Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or_else(|| std::io::Error::other("clipboard stdin unavailable"))?
        .write_all(text.as_bytes())?;
    child.wait()?;
    Ok(())
}
