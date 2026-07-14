//! The repository port decoupling the service from concrete entry storage.
//!
//! The service depends on this trait; the TOML backend and the in-memory fake
//! implement it (DIP).

use crate::domain::error::Result;
use crate::domain::repo::{Repo, RepoKind};

/// Reads and writes the managed entries (the `[[repos]]` array of config.toml).
pub trait RepoRepository {
    /// All entries in file order, with paths expanded and `kind` resolved.
    fn find_all(&self) -> Result<Vec<Repo>>;
    /// Persists the full entry list, preserving the settings block.
    fn save_all(&self, repos: &[Repo]) -> Result<()>;
    /// The ordered list of user section names for `kind` (each kind has its own
    /// independent namespace).
    fn find_sections(&self, kind: RepoKind) -> Result<Vec<String>>;
    /// Persists the ordered section names for `kind`, preserving everything else.
    fn save_sections(&self, kind: RepoKind, sections: &[String]) -> Result<()>;
}
