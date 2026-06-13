//! An in-memory [`RepoRepository`] used as a fake in tests (preferred over a
//! mock). It also serves as a second backend, validating the port abstraction.

use std::cell::RefCell;

use crate::domain::error::Result;
use crate::domain::repo::Repo;
use crate::storage::repository::RepoRepository;

/// Holds entries in memory; `save_all` replaces the whole list.
pub struct InMemoryRepoRepository {
    repos: RefCell<Vec<Repo>>,
}

impl InMemoryRepoRepository {
    /// Creates a repository seeded with `initial`.
    pub fn new(initial: Vec<Repo>) -> Self {
        InMemoryRepoRepository {
            repos: RefCell::new(initial),
        }
    }
}

impl RepoRepository for InMemoryRepoRepository {
    fn find_all(&self) -> Result<Vec<Repo>> {
        Ok(self.repos.borrow().clone())
    }

    fn save_all(&self, repos: &[Repo]) -> Result<()> {
        *self.repos.borrow_mut() = repos.to_vec();
        Ok(())
    }
}
