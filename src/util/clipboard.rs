//! Best-effort clipboard copy via the system tool (`pbcopy` on macOS).
//!
//! Kept dependency-free: the text is piped to `pbcopy` rather than pulling in a
//! clipboard crate. On systems without `pbcopy` the spawn fails and the caller
//! surfaces it as a transient status message.

use std::io::Write;
use std::process::{Command, Stdio};

use crate::domain::error::{Error, Result};

/// Copies `text` to the system clipboard.
///
/// # Errors
/// Returns an I/O error if the clipboard tool cannot be spawned or written to.
pub fn copy(text: &str) -> Result<()> {
    let mut child = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| Error::io("spawn pbcopy", e))?;
    child
        .stdin
        .take()
        .ok_or_else(|| Error::invalid("pbcopy stdin unavailable"))?
        .write_all(text.as_bytes())
        .map_err(|e| Error::io("write to pbcopy", e))?;
    child.wait().map_err(|e| Error::io("wait for pbcopy", e))?;
    Ok(())
}
