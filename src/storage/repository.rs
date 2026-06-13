//! The repository port decoupling the service from concrete entry storage.
//!
//! The service depends on this trait; the TOML backend and the in-memory fake
//! implement it (DIP).

use crate::domain::error::Result;
use crate::domain::repo::Repo;

/// Reads and writes the managed entries (the `[[repos]]` array of config.toml).
pub trait RepoRepository {
    /// All entries in file order, with paths expanded and `kind` resolved.
    fn find_all(&self) -> Result<Vec<Repo>>;
    /// Persists the full entry list, preserving the settings block.
    fn save_all(&self, repos: &[Repo]) -> Result<()>;
}
