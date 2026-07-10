//! Clipboard copy, over the toolkit's platform tools.
//!
//! [`ratada::clipboard`] pipes the text to whichever tool the platform has
//! (`pbcopy`, `clip`, `wl-copy`/`xclip`/`xsel`). It reports only success or
//! failure, so this wrapper turns a failure into hop's domain error, which the
//! TUI shows as a status message.

use crate::domain::error::{Error, Result};

/// Copies `text` to the system clipboard.
///
/// # Errors
/// Returns an error when no clipboard tool is available or none accepted the
/// text.
pub fn copy(text: &str) -> Result<()> {
    if ratada::clipboard::copy(text) {
        return Ok(());
    }
    Err(Error::invalid("no clipboard tool available"))
}
