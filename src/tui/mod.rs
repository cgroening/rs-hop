//! TUI front end: the tabbed list view, its key handling and its overlays.
//!
//! The application screen itself lives in [`app`]; this module wires up the
//! views, widgets and helpers it draws with, and re-exports the handful of
//! items the composition root needs ([`App`], [`run`], [`RunOutcome`] and
//! [`StartupStatus`]).

pub mod app;
pub mod appframe;
pub mod bindings;
pub mod columns;
pub mod detail;
pub mod form;
pub mod git_columns;
pub mod help;
pub mod list_layout;
pub mod path_picker;
pub mod presentation;
pub mod preview;
pub mod row_cells;
pub mod scroll;
pub mod section_picker;
pub mod sections_modal;
pub mod sections_view;
pub mod skin;
pub mod table;
pub mod terminal;
pub mod widgets;

pub use app::{App, RunOutcome, StartupStatus, run};
pub use terminal::{Tui, TuiEvent};
