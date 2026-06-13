//! Business rules over the managed entries: CRUD, favourite/archive toggles,
//! slug assignment, path repair, usage tracking and one-level undo.
//!
//! The service owns the working entry list (with runtime git info and usage
//! hydrated onto it) and persists stored fields through the injected
//! [`RepoRepository`]. Every config-mutating method records the pre-change list
//! as a single undo frame and rolls back if the write fails.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::domain::error::{Error, Result};
use crate::domain::repo::{GitInfo, Repo};
use crate::domain::slug;
use crate::storage::repository::RepoRepository;
use crate::storage::usage_state;

/// A captured entry list plus a human label, for one level of undo.
struct UndoSnapshot {
    repos: Vec<Repo>,
    label: String,
}

/// Coordinates reads and writes of the managed entries.
pub struct RepoService {
    repository: Box<dyn RepoRepository>,
    repos: Vec<Repo>,
    usage_path: PathBuf,
    selected_repo_path: PathBuf,
    undo: Option<UndoSnapshot>,
}

impl RepoService {
    /// Loads entries through `repository` and hydrates usage from `usage_path`.
    ///
    /// # Errors
    /// Returns an error if the entries cannot be read.
    pub fn new(
        repository: Box<dyn RepoRepository>,
        usage_path: PathBuf,
        selected_repo_path: PathBuf,
    ) -> Result<Self> {
        let mut service = RepoService {
            repos: repository.find_all()?,
            repository,
            usage_path,
            selected_repo_path,
            undo: None,
        };
        service.hydrate_usage();
        Ok(service)
    }

    /// All entries in stored order (the view applies its own sort and filter).
    pub fn repos(&self) -> &[Repo] {
        &self.repos
    }

    /// The entry at `index`, if any.
    pub fn get(&self, index: usize) -> Option<&Repo> {
        self.repos.get(index)
    }

    /// The index of the entry with the given slug, if any.
    pub fn index_by_slug(&self, slug: &str) -> Option<usize> {
        self.repos
            .iter()
            .position(|repo| repo.slug.as_deref() == Some(slug))
    }

    /// Adds `repo`, validating its slug when present.
    ///
    /// # Errors
    /// Returns [`Error::Slug`] for an invalid or duplicate slug, or a write
    /// error if persistence fails.
    pub fn add(&mut self, repo: Repo) -> Result<()> {
        if let Some(slug) = repo.slug.clone() {
            self.check_slug(&slug, None)?;
        }
        self.mutate("add entry", |repos| {
            repos.push(repo);
            Ok(())
        })
    }

    /// Replaces the entry at `index` with `repo`, validating its slug.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] for a bad index, [`Error::Slug`] for an
    /// invalid or duplicate slug, or a write error.
    pub fn update(&mut self, index: usize, repo: Repo) -> Result<()> {
        self.ensure_index(index)?;
        if let Some(slug) = repo.slug.clone() {
            self.check_slug(&slug, Some(index))?;
        }
        self.mutate("edit entry", |repos| {
            repos[index] = repo;
            Ok(())
        })
    }

    /// Removes the entry at `index`.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] for a bad index, or a write error.
    pub fn delete(&mut self, index: usize) -> Result<()> {
        self.ensure_index(index)?;
        self.mutate("delete entry", |repos| {
            repos.remove(index);
            Ok(())
        })
    }

    /// Toggles the favourite flag of the entry at `index`.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] for a bad index, or a write error.
    pub fn toggle_fav(&mut self, index: usize) -> Result<()> {
        self.ensure_index(index)?;
        self.mutate("toggle favourite", |repos| {
            repos[index].fav = !repos[index].fav;
            Ok(())
        })
    }

    /// Sets the archived flag of the entry at `index`.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] for a bad index, or a write error.
    pub fn set_archived(&mut self, index: usize, archived: bool) -> Result<()> {
        self.ensure_index(index)?;
        let label = if archived {
            "archive entry"
        } else {
            "restore entry"
        };
        self.mutate(label, |repos| {
            repos[index].archived = archived;
            Ok(())
        })
    }

    /// Sets or clears the slug of the entry at `index`.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] for a bad index, [`Error::Slug`] for an
    /// invalid or duplicate slug, or a write error.
    pub fn set_slug(
        &mut self,
        index: usize,
        new_slug: Option<String>,
    ) -> Result<()> {
        self.ensure_index(index)?;
        if let Some(slug) = &new_slug {
            self.check_slug(slug, Some(index))?;
        }
        self.mutate("set slug", |repos| {
            repos[index].slug = new_slug;
            Ok(())
        })
    }

    /// Deletes every entry in `indices` as one undo-able action.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] for a bad index, or a write error.
    pub fn delete_many(&mut self, indices: &[usize]) -> Result<()> {
        for &index in indices {
            self.ensure_index(index)?;
        }
        let mut sorted = indices.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        self.mutate("delete entries", |repos| {
            // Remove from the back so earlier indices stay valid.
            for &index in sorted.iter().rev() {
                repos.remove(index);
            }
            Ok(())
        })
    }

    /// Sets the archived flag for every entry in `indices` as one action.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] for a bad index, or a write error.
    pub fn set_archived_many(
        &mut self,
        indices: &[usize],
        archived: bool,
    ) -> Result<()> {
        for &index in indices {
            self.ensure_index(index)?;
        }
        let label = if archived {
            "archive entries"
        } else {
            "restore entries"
        };
        let indices = indices.to_vec();
        self.mutate(label, |repos| {
            for &index in &indices {
                repos[index].archived = archived;
            }
            Ok(())
        })
    }

    /// Sets the favourite flag for every entry in `indices` as one action.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] for a bad index, or a write error.
    pub fn set_fav_many(&mut self, indices: &[usize], fav: bool) -> Result<()> {
        for &index in indices {
            self.ensure_index(index)?;
        }
        let indices = indices.to_vec();
        self.mutate("set favourite", |repos| {
            for &index in &indices {
                repos[index].fav = fav;
            }
            Ok(())
        })
    }

    /// Swaps two entries' positions (the stored custom order), as one action.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] for a bad index, or a write error.
    pub fn swap_entries(&mut self, a: usize, b: usize) -> Result<()> {
        self.ensure_index(a)?;
        self.ensure_index(b)?;
        self.mutate("reorder entry", |repos| {
            repos.swap(a, b);
            Ok(())
        })
    }

    /// Repoints the entry at `index` to `path` (used by the path-repair picker).
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] for a bad index, or a write error.
    pub fn set_path(&mut self, index: usize, path: PathBuf) -> Result<()> {
        self.ensure_index(index)?;
        self.mutate("repair path", |repos| {
            repos[index].path = path;
            Ok(())
        })
    }

    /// Reverts the last config mutation, returning its label.
    ///
    /// # Errors
    /// Returns a write error if persisting the reverted list fails.
    pub fn undo(&mut self) -> Result<Option<String>> {
        let Some(snapshot) = self.undo.take() else {
            return Ok(None);
        };
        self.repository.save_all(&snapshot.repos)?;
        self.repos = snapshot.repos;
        self.hydrate_usage();
        Ok(Some(snapshot.label))
    }

    /// Records an open of the entry at `index`: bumps its usage counters.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] for a bad index, or a write error.
    pub fn mark_used(&mut self, index: usize) -> Result<()> {
        let repo = self
            .repos
            .get(index)
            .ok_or_else(|| Error::NotFound(format!("index {index}")))?;
        let path = repo.path.clone();
        usage_state::record(&self.usage_path, &path)?;
        if let Some(usage) =
            usage_state::load(&self.usage_path).get(&path).copied()
        {
            let repo = &mut self.repos[index];
            repo.last_used = usage.last_used;
            repo.open_count = usage.open_count;
        }
        Ok(())
    }

    /// Writes `repo_path` to the selected-repo handoff file the shell reads.
    ///
    /// # Errors
    /// Returns a write error if the file cannot be written.
    pub fn write_selected(&self, repo_path: &Path) -> Result<()> {
        usage_state::write_selected_repo(&self.selected_repo_path, repo_path)
    }

    /// Applies gathered git info onto the matching entries (by path).
    pub fn apply_git_infos(&mut self, infos: &HashMap<PathBuf, GitInfo>) {
        for repo in &mut self.repos {
            if let Some(info) = infos.get(&repo.path) {
                repo.git_info = Some(info.clone());
            }
        }
    }

    /// Sets the live git info for the entry whose path matches `path`.
    pub fn set_git_info(&mut self, path: &Path, info: GitInfo) {
        for repo in &mut self.repos {
            if repo.path == path {
                repo.git_info = Some(info.clone());
            }
        }
    }

    /// Loads usage counters and copies them onto the in-memory entries.
    fn hydrate_usage(&mut self) {
        let usage = usage_state::load(&self.usage_path);
        for repo in &mut self.repos {
            if let Some(entry) = usage.get(&repo.path) {
                repo.last_used = entry.last_used;
                repo.open_count = entry.open_count;
            }
        }
    }

    /// Validates a slug's format and that no other entry already uses it.
    fn check_slug(&self, slug: &str, except: Option<usize>) -> Result<()> {
        slug::validate_format(slug)?;
        let clash = self.repos.iter().enumerate().any(|(index, repo)| {
            Some(index) != except && repo.slug.as_deref() == Some(slug)
        });
        if clash {
            return Err(Error::Slug(format!(
                "slug '{slug}' is already in use"
            )));
        }
        Ok(())
    }

    /// Confirms `index` is in range.
    fn ensure_index(&self, index: usize) -> Result<()> {
        if index >= self.repos.len() {
            return Err(Error::NotFound(format!("index {index}")));
        }
        Ok(())
    }

    /// Applies `f` to the entry list, persists it, and records one undo frame.
    /// Rolls back the in-memory list if `f` or the write fails.
    fn mutate<F>(&mut self, label: &str, f: F) -> Result<()>
    where
        F: FnOnce(&mut Vec<Repo>) -> Result<()>,
    {
        let snapshot = self.repos.clone();
        if let Err(error) = f(&mut self.repos) {
            self.repos = snapshot;
            return Err(error);
        }
        if let Err(error) = self.repository.save_all(&self.repos) {
            self.repos = snapshot;
            return Err(error);
        }
        self.undo = Some(UndoSnapshot {
            repos: snapshot,
            label: label.to_string(),
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::repo::RepoKind;
    use crate::storage::in_memory_repository::InMemoryRepoRepository;

    fn service(initial: Vec<Repo>) -> RepoService {
        let dir = std::env::temp_dir().join(format!(
            "hop-svc-{}-{}",
            std::process::id(),
            initial.len()
        ));
        RepoService::new(
            Box::new(InMemoryRepoRepository::new(initial)),
            dir.join("usage.toml"),
            dir.join("selected.txt"),
        )
        .unwrap()
    }

    fn repo(name: &str) -> Repo {
        let mut repo = Repo::new(PathBuf::from(format!("/code/{name}")));
        repo.name = Some(name.to_string());
        repo
    }

    #[test]
    fn add_rejects_duplicate_slug() {
        let mut a = repo("a");
        a.slug = Some("x".to_string());
        let mut svc = service(vec![a]);
        let mut b = repo("b");
        b.slug = Some("x".to_string());
        assert!(matches!(svc.add(b), Err(Error::Slug(_))));
        assert_eq!(svc.repos().len(), 1);
    }

    #[test]
    fn delete_many_removes_all_and_undoes_as_one() {
        let mut svc = service(vec![repo("a"), repo("b"), repo("c")]);
        svc.delete_many(&[0, 2]).unwrap();
        let names: Vec<_> =
            svc.repos().iter().map(Repo::display_name).collect();
        assert_eq!(names, vec!["b"]);
        svc.undo().unwrap();
        assert_eq!(svc.repos().len(), 3);
    }

    #[test]
    fn set_archived_and_fav_many() {
        let mut svc = service(vec![repo("a"), repo("b"), repo("c")]);
        svc.set_archived_many(&[0, 1], true).unwrap();
        assert!(svc.get(0).unwrap().archived);
        assert!(svc.get(1).unwrap().archived);
        assert!(!svc.get(2).unwrap().archived);
        svc.set_fav_many(&[1, 2], true).unwrap();
        assert!(svc.get(1).unwrap().fav);
        assert!(svc.get(2).unwrap().fav);
    }

    #[test]
    fn swap_entries_reorders() {
        let mut svc = service(vec![repo("a"), repo("b"), repo("c")]);
        svc.swap_entries(0, 2).unwrap();
        let names: Vec<_> =
            svc.repos().iter().map(Repo::display_name).collect();
        assert_eq!(names, vec!["c", "b", "a"]);
    }

    #[test]
    fn add_rejects_reserved_slug() {
        let mut svc = service(vec![]);
        let mut a = repo("a");
        a.slug = Some("list".to_string());
        assert!(matches!(svc.add(a), Err(Error::Slug(_))));
    }

    #[test]
    fn archive_and_restore_round_trip() {
        let mut svc = service(vec![repo("a")]);
        svc.set_archived(0, true).unwrap();
        assert!(svc.get(0).unwrap().archived);
        svc.set_archived(0, false).unwrap();
        assert!(!svc.get(0).unwrap().archived);
    }

    #[test]
    fn toggle_fav_flips_flag() {
        let mut svc = service(vec![repo("a")]);
        svc.toggle_fav(0).unwrap();
        assert!(svc.get(0).unwrap().fav);
    }

    #[test]
    fn set_slug_then_lookup_by_slug() {
        let mut svc = service(vec![repo("a"), repo("b")]);
        svc.set_slug(1, Some("bee".to_string())).unwrap();
        assert_eq!(svc.index_by_slug("bee"), Some(1));
    }

    #[test]
    fn undo_reverts_last_change() {
        let mut svc = service(vec![repo("a")]);
        svc.add(repo("b")).unwrap();
        assert_eq!(svc.repos().len(), 2);
        let label = svc.undo().unwrap();
        assert_eq!(label.as_deref(), Some("add entry"));
        assert_eq!(svc.repos().len(), 1);
    }

    #[test]
    fn delete_removes_entry() {
        let mut svc = service(vec![repo("a"), repo("b")]);
        svc.delete(0).unwrap();
        assert_eq!(svc.repos().len(), 1);
        assert_eq!(svc.get(0).unwrap().display_name(), "b");
    }

    #[test]
    fn update_changes_kind() {
        let mut svc = service(vec![repo("a")]);
        let mut edited = repo("a");
        edited.kind = RepoKind::Folder;
        svc.update(0, edited).unwrap();
        assert_eq!(svc.get(0).unwrap().kind, RepoKind::Folder);
    }
}
