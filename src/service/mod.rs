//! Service layer: business rules over the storage ports.
//!
//! [`repo_service::RepoService`] owns the working entry list and enforces the
//! rules (validation, slug uniqueness, archive, undo); [`status_service`]
//! gathers git status synchronously or in the background; [`preview_service`]
//! gathers the preview `git log` in the background.
//!
//! The services also front the state files their work produces - the git-status
//! cache ([`status_service`]), the statistics cache ([`stats_service`]) and the
//! TUI's view state ([`ui_state_service`]) - so the front ends never reach into
//! a storage adapter themselves.

pub mod preview_service;
pub mod repo_service;
pub mod stats_service;
pub mod status_service;
pub mod ui_state_service;
pub mod zip_service;
