//! The process-wide `Ctrl+C` flag.
//!
//! The handler only records the interrupt: the ZIP backup builds into a
//! `.part` file and renames it atomically, and the terminal guard restores the
//! screen on drop, so unwinding normally is what actually cleans up. Killing
//! the process from inside the handler would skip both. Long CLI loops ask
//! [`is_interrupted`] between steps, and `main` turns the flag into exit 130.

use std::sync::atomic::{AtomicBool, Ordering};

/// Set once `Ctrl+C` (or SIGTERM) arrived.
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// Installs the handler that records an interrupt. A second installation
/// (or a platform without signals) is ignored: the run then simply cannot be
/// stopped early, which is no reason to fail it.
pub fn install() {
    let _ = ctrlc::set_handler(|| {
        INTERRUPTED.store(true, Ordering::SeqCst);
    });
}

/// Whether an interrupt has arrived since the process started.
pub fn is_interrupted() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}
