//! Terminal RAII guard.
//!
//! [`Tui`] enables raw mode and the alternate screen on creation and restores
//! the terminal on drop, so the user's shell is left clean even on error or
//! panic. Event reading and the global quit chord are handled by the run loop.

use std::io::{self, Stdout, stdout};

use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
    enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

/// The concrete terminal backend used throughout the TUI.
pub type Backend = CrosstermBackend<Stdout>;

/// Owns the terminal in raw/alternate-screen mode for the session.
pub struct Tui {
    /// The ratatui terminal; draw through this.
    pub terminal: Terminal<Backend>,
}

impl Tui {
    /// Enters raw mode and the alternate screen.
    ///
    /// # Errors
    /// Returns an I/O error if the terminal cannot be configured.
    pub fn new() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut out = stdout();
        execute!(out, EnterAlternateScreen)?;
        let terminal = Terminal::new(CrosstermBackend::new(out))?;
        Ok(Tui { terminal })
    }
}

/// Leaves raw mode and the alternate screen and shows the cursor again.
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
}

impl Drop for Tui {
    fn drop(&mut self) {
        restore_terminal();
    }
}
