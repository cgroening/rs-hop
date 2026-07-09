//! hop: a fuzzy-finder TUI to jump between git repositories and folders.
//!
//! It is a Rust port of the `git-repo-jumper` tool: pick an entry, the path is
//! written to a file the shell wrapper reads to `cd`, and (for git repos) the
//! configured git tool is launched. On top of the original it adds in-app
//! management, three tabs (git repos / files and folders / archive), slugs for
//! a `hop <slug>` fast jump, sort modes and a marker plus picker for paths that
//! no longer exist.
//!
//! The crate is organised in layers with dependencies pointing inward:
//! [`domain`] (entities and pure logic) is wrapped by [`storage`] (the
//! repository and git-client ports with their backends), then [`service`]
//! (business rules), and finally the [`cli`] and [`tui`] front ends. [`config`]
//! and [`util`] are leaf utilities. The binary (`main.rs`) is a thin
//! composition root over this library, which also lets the integration tests
//! exercise the public API.

pub mod cli;
pub mod config;
pub mod domain;
pub mod keymap;
pub mod service;
pub mod storage;
pub mod tui;
pub mod util;

/// Framework-agnostic theming (colors, palette, glyphs, themes), re-exported
/// from the `ratada` toolkit so the rest of the crate can refer to
/// `crate::theme` without naming the dependency at every call site.
pub use ratada::theme;
