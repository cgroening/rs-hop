//! An in-memory [`RepoRepository`] used as a fake in tests (preferred over a
//! mock). It also serves as a second backend, validating the port abstraction.

use std::cell::RefCell;

use crate::domain::error::Result;
use crate::domain::repo::{Repo, RepoKind};
use crate::storage::repository::RepoRepository;

/// Holds entries in memory; `save_all` replaces the whole list.
pub struct InMemoryRepoRepository {
    repos: RefCell<Vec<Repo>>,
    git_sections: RefCell<Vec<String>>,
    path_sections: RefCell<Vec<String>>,
}

impl InMemoryRepoRepository {
    /// Creates a repository seeded with `initial` entries and no sections.
    pub fn new(initial: Vec<Repo>) -> Self {
        InMemoryRepoRepository {
            repos: RefCell::new(initial),
            git_sections: RefCell::new(Vec::new()),
            path_sections: RefCell::new(Vec::new()),
        }
    }

    /// The section list for `kind`.
    fn sections_for(&self, kind: RepoKind) -> &RefCell<Vec<String>> {
        match kind {
            RepoKind::Git => &self.git_sections,
            RepoKind::Path => &self.path_sections,
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

    fn find_sections(&self, kind: RepoKind) -> Result<Vec<String>> {
        Ok(self.sections_for(kind).borrow().clone())
    }

    fn save_sections(&self, kind: RepoKind, sections: &[String]) -> Result<()> {
        *self.sections_for(kind).borrow_mut() = sections.to_vec();
        Ok(())
    }
}
