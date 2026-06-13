//! Storage layer: the repository and git-client ports and their backends.
//!
//! The [`repository::RepoRepository`] and [`git_client::GitClient`] traits keep
//! the service free of concrete storage. The TOML backend persists entries, the
//! subprocess backend gathers git status, and the cache and usage-state modules
//! hold derived runtime data outside the user's config.

pub mod cache;
pub mod git_client;
pub mod in_memory_repository;
pub mod repository;
pub mod subprocess_git_client;
pub mod toml_repo_repository;
pub mod ui_state;
pub mod usage_state;
