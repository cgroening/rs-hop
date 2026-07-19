//! The repository port decoupling the service from concrete entry storage.
//!
//! The service depends on this trait; the TOML backend and the in-memory fake
//! implement it (DIP).

use crate::domain::error::Result;
use crate::domain::repo::{Repo, RepoKind};

/// Reads and writes the managed entries (the `[[repos]]` array of config.toml).
pub trait RepoRepository {
    /// All entries in file order, with paths expanded and `kind` resolved.
    ///
    /// A backing store that does not exist yet reads as an empty list rather
    /// than an error, so a first run needs no setup.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`](crate::domain::error::Error::Config) if the
    /// store cannot be read or does not parse.
    fn find_all(&self) -> Result<Vec<Repo>>;

    /// Persists the full entry list, preserving the settings block.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`](crate::domain::error::Error::Config) if the
    /// store cannot be read, does not parse, or cannot be written.
    fn save_all(&self, repos: &[Repo]) -> Result<()>;

    /// The ordered list of user section names for `kind` (each kind has its own
    /// independent namespace).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`](crate::domain::error::Error::Config) if the
    /// store cannot be read or does not parse.
    fn find_sections(&self, kind: RepoKind) -> Result<Vec<String>>;

    /// Persists the ordered section names for `kind`, preserving everything
    /// else.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`](crate::domain::error::Error::Config) if the
    /// store cannot be read, does not parse, or cannot be written.
    fn save_sections(&self, kind: RepoKind, sections: &[String]) -> Result<()>;
}
