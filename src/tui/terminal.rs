//! Terminal guard, layered on the `ratada` toolkit.
//!
//! The RAII guard itself (raw mode + the alternate screen, restored on drop or
//! panic) is [`ratada::Tui`], re-exported here together with [`ratada::TuiEvent`]
//! so the run loop reads classified events (key press, resize, global quit)
//! instead of driving crossterm directly. hop's logging writes to a file only,
//! so no stderr-mute hooks are needed; the plain [`ratada::Tui::new`] is used.

pub use ratada::{Tui, TuiEvent};
