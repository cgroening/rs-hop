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
use crate::domain::repo::{GitInfo, Repo, RepoKind};
use crate::domain::sections;
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
    git_sections: Vec<String>,
    path_sections: Vec<String>,
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
            git_sections: repository.find_sections(RepoKind::Git)?,
            path_sections: repository.find_sections(RepoKind::Path)?,
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

    /// Adds several entries as one undo-able action (used by `hop scan`).
    /// A no-op for an empty list.
    ///
    /// # Errors
    /// Returns [`Error::Slug`] for an invalid or duplicate slug, or a write
    /// error if persistence fails.
    pub fn add_many(&mut self, repos: Vec<Repo>) -> Result<()> {
        if repos.is_empty() {
            return Ok(());
        }
        for repo in &repos {
            if let Some(slug) = &repo.slug {
                self.check_slug(slug, None)?;
            }
        }
        self.mutate("add entries", |existing| {
            existing.extend(repos);
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

    /// Applies `edit` to every entry in `indices` as one action (a bulk edit of
    /// fields with no per-entry validation, e.g. section/kind/favourite/backup).
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] for a bad index, or a write error.
    pub fn update_many(
        &mut self,
        indices: &[usize],
        edit: impl Fn(&mut Repo),
    ) -> Result<()> {
        for &index in indices {
            self.ensure_index(index)?;
        }
        let indices = indices.to_vec();
        self.mutate("edit entries", |repos| {
            for &index in &indices {
                edit(&mut repos[index]);
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

    /// The ordered list of section names for `kind` (each kind has its own
    /// independent namespace).
    pub fn sections(&self, kind: RepoKind) -> &[String] {
        self.sections_of(kind)
    }

    /// Appends a new section to `kind`'s namespace, validating its name.
    ///
    /// # Errors
    /// Returns [`Error::Invalid`] for an empty, reserved or duplicate name, or a
    /// write error.
    pub fn add_section(&mut self, kind: RepoKind, name: &str) -> Result<()> {
        let name = name.trim();
        self.validate_section_name(kind, name, None)?;
        let mut next = self.sections_of(kind).to_vec();
        next.push(name.to_string());
        self.persist_sections(kind, next)
    }

    /// Registers `name` in `kind`'s namespace if it is non-empty and not already
    /// known (case-insensitive), used when an entry is saved with a new section.
    ///
    /// # Errors
    /// Returns a write error if persistence fails.
    pub fn ensure_section(&mut self, kind: RepoKind, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() || self.section_index(kind, name).is_some() {
            return Ok(());
        }
        let mut next = self.sections_of(kind).to_vec();
        next.push(name.to_string());
        self.persist_sections(kind, next)
    }

    /// Renames the section `old` to `new` in `kind`'s namespace, updating every
    /// entry of that kind which referenced it (one undo frame for the entries).
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] when `old` is unknown, [`Error::Invalid`] for
    /// a bad `new` name, or a write error.
    pub fn rename_section(
        &mut self,
        kind: RepoKind,
        old: &str,
        new: &str,
    ) -> Result<()> {
        let new = new.trim();
        let pos = self
            .section_index(kind, old)
            .ok_or_else(|| Error::NotFound(format!("section '{old}'")))?;
        self.validate_section_name(kind, new, Some(pos))?;
        let old_name = self.sections_of(kind)[pos].clone();
        let new_name = new.to_string();
        self.mutate("rename section", |repos| {
            for repo in repos.iter_mut() {
                if repo.kind == kind
                    && repo.section.as_deref() == Some(old_name.as_str())
                {
                    repo.section = Some(new_name.clone());
                }
            }
            Ok(())
        })?;
        let mut next = self.sections_of(kind).to_vec();
        next[pos] = new.to_string();
        self.persist_sections(kind, next)
    }

    /// Deletes the section `name` from `kind`'s namespace, moving that kind's
    /// entries in it to Ungrouped (one undo frame for the entries).
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] when `name` is unknown, or a write error.
    pub fn delete_section(&mut self, kind: RepoKind, name: &str) -> Result<()> {
        let pos = self
            .section_index(kind, name)
            .ok_or_else(|| Error::NotFound(format!("section '{name}'")))?;
        let removed = self.sections_of(kind)[pos].clone();
        self.mutate("delete section", |repos| {
            for repo in repos.iter_mut() {
                if repo.kind == kind
                    && repo.section.as_deref() == Some(removed.as_str())
                {
                    repo.section = None;
                }
            }
            Ok(())
        })?;
        let mut next = self.sections_of(kind).to_vec();
        next.remove(pos);
        self.persist_sections(kind, next)
    }

    /// Moves the section at `from` to position `to` in `kind`'s order.
    ///
    /// # Errors
    /// Returns [`Error::NotFound`] for an out-of-range index, or a write error.
    pub fn move_section(
        &mut self,
        kind: RepoKind,
        from: usize,
        to: usize,
    ) -> Result<()> {
        let len = self.sections_of(kind).len();
        if from >= len || to >= len {
            return Err(Error::NotFound(format!("section index {from}/{to}")));
        }
        let mut next = self.sections_of(kind).to_vec();
        let name = next.remove(from);
        next.insert(to, name);
        self.persist_sections(kind, next)
    }

    /// The section list for `kind`.
    fn sections_of(&self, kind: RepoKind) -> &Vec<String> {
        match kind {
            RepoKind::Git => &self.git_sections,
            RepoKind::Path => &self.path_sections,
        }
    }

    /// The mutable section list for `kind`.
    fn sections_slot(&mut self, kind: RepoKind) -> &mut Vec<String> {
        match kind {
            RepoKind::Git => &mut self.git_sections,
            RepoKind::Path => &mut self.path_sections,
        }
    }

    /// The index of the section named `name` in `kind`'s namespace
    /// (case-insensitive), if present.
    fn section_index(&self, kind: RepoKind, name: &str) -> Option<usize> {
        let name = name.trim();
        self.sections_of(kind)
            .iter()
            .position(|section| section.eq_ignore_ascii_case(name))
    }

    /// Validates a section name in `kind`'s namespace: non-empty, not the
    /// reserved Ungrouped label, and not a duplicate (case-insensitive).
    fn validate_section_name(
        &self,
        kind: RepoKind,
        name: &str,
        except: Option<usize>,
    ) -> Result<()> {
        if name.is_empty() {
            return Err(Error::invalid("section name must not be empty"));
        }
        if name.eq_ignore_ascii_case(sections::UNGROUPED) {
            return Err(Error::invalid("'Ungrouped' is a reserved name"));
        }
        let clash =
            self.sections_of(kind)
                .iter()
                .enumerate()
                .any(|(index, sec)| {
                    Some(index) != except && sec.eq_ignore_ascii_case(name)
                });
        if clash {
            return Err(Error::invalid(format!(
                "section '{name}' already exists"
            )));
        }
        Ok(())
    }

    /// Replaces `kind`'s section list and persists it, rolling back on write
    /// failure.
    fn persist_sections(
        &mut self,
        kind: RepoKind,
        next: Vec<String>,
    ) -> Result<()> {
        let previous = std::mem::replace(self.sections_slot(kind), next);
        let write = self.repository.save_sections(kind, self.sections_of(kind));
        if let Err(error) = write {
            *self.sections_slot(kind) = previous;
            return Err(error);
        }
        Ok(())
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

    fn sectioned(name: &str, section: Option<&str>) -> Repo {
        let mut repo = repo(name);
        repo.section = section.map(str::to_string);
        repo
    }

    #[test]
    fn update_many_applies_to_all_in_one_undo_frame() {
        let mut svc = service(vec![repo("a"), repo("b"), repo("c")]);
        svc.update_many(&[0, 2], |repo| {
            repo.section = Some("Work".to_string());
        })
        .unwrap();
        assert_eq!(svc.get(0).unwrap().section.as_deref(), Some("Work"));
        assert_eq!(svc.get(1).unwrap().section, None);
        assert_eq!(svc.get(2).unwrap().section.as_deref(), Some("Work"));
        // One undo frame restores every touched entry.
        svc.undo().unwrap();
        assert_eq!(svc.get(0).unwrap().section, None);
        assert_eq!(svc.get(2).unwrap().section, None);
    }

    const GIT: RepoKind = RepoKind::Git;
    const PATH: RepoKind = RepoKind::Path;

    #[test]
    fn add_section_rejects_empty_reserved_and_duplicate() {
        let mut svc = service(vec![]);
        svc.add_section(GIT, "Work").unwrap();
        assert_eq!(svc.sections(GIT), ["Work"]);
        assert!(matches!(svc.add_section(GIT, "  "), Err(Error::Invalid(_))));
        assert!(matches!(
            svc.add_section(GIT, "Ungrouped"),
            Err(Error::Invalid(_))
        ));
        // Duplicate is case-insensitive.
        assert!(matches!(
            svc.add_section(GIT, "work"),
            Err(Error::Invalid(_))
        ));
    }

    #[test]
    fn ensure_section_is_idempotent() {
        let mut svc = service(vec![]);
        svc.ensure_section(GIT, "Work").unwrap();
        svc.ensure_section(GIT, "work").unwrap();
        svc.ensure_section(GIT, "  ").unwrap();
        assert_eq!(svc.sections(GIT), ["Work"]);
    }

    #[test]
    fn rename_section_updates_entries() {
        let mut svc =
            service(vec![sectioned("a", Some("Work")), sectioned("b", None)]);
        svc.add_section(GIT, "Work").unwrap();
        svc.rename_section(GIT, "Work", "Job").unwrap();
        assert_eq!(svc.sections(GIT), ["Job"]);
        assert_eq!(svc.get(0).unwrap().section.as_deref(), Some("Job"));
        assert_eq!(svc.get(1).unwrap().section, None);
    }

    #[test]
    fn delete_section_ungroups_entries() {
        let mut svc = service(vec![sectioned("a", Some("Work"))]);
        svc.add_section(GIT, "Work").unwrap();
        svc.delete_section(GIT, "Work").unwrap();
        assert!(svc.sections(GIT).is_empty());
        assert_eq!(svc.get(0).unwrap().section, None);
    }

    #[test]
    fn move_section_reorders() {
        let mut svc = service(vec![]);
        svc.add_section(GIT, "Work").unwrap();
        svc.add_section(GIT, "Personal").unwrap();
        svc.add_section(GIT, "Misc").unwrap();
        svc.move_section(GIT, 2, 0).unwrap();
        assert_eq!(svc.sections(GIT), ["Misc", "Work", "Personal"]);
    }

    #[test]
    fn git_and_path_namespaces_are_independent() {
        // Same section name in both namespaces is allowed and separate.
        let mut svc = service(vec![]);
        svc.add_section(GIT, "Work").unwrap();
        svc.add_section(PATH, "Work").unwrap();
        assert_eq!(svc.sections(GIT), ["Work"]);
        assert_eq!(svc.sections(PATH), ["Work"]);
    }

    #[test]
    fn rename_only_touches_the_matching_kind() {
        // A git entry and a (path) folder both sit in a "Work" section.
        let git_entry = sectioned("g", Some("Work"));
        let mut folder = sectioned("f", Some("Work"));
        folder.kind = PATH;
        let mut svc = service(vec![git_entry, folder]);
        svc.add_section(GIT, "Work").unwrap();
        svc.rename_section(GIT, "Work", "Job").unwrap();
        // Only the git entry is rewritten; the folder keeps its own "Work".
        assert_eq!(svc.get(0).unwrap().section.as_deref(), Some("Job"));
        assert_eq!(svc.get(1).unwrap().section.as_deref(), Some("Work"));
    }

    #[test]
    fn delete_removes_entry() {
        let mut svc = service(vec![repo("a"), repo("b")]);
        svc.delete(0).unwrap();
        assert_eq!(svc.repos().len(), 1);
        assert_eq!(svc.get(0).unwrap().display_name(), "b");
    }

    #[test]
    fn add_many_appends_all_in_one_action() {
        let mut svc = service(vec![repo("a")]);
        svc.add_many(vec![repo("b"), repo("c")]).unwrap();
        let names: Vec<_> =
            svc.repos().iter().map(Repo::display_name).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
        // One undo frame reverts the whole batch.
        svc.undo().unwrap();
        assert_eq!(svc.repos().len(), 1);
        // An empty batch is a no-op.
        svc.add_many(vec![]).unwrap();
        assert_eq!(svc.repos().len(), 1);
    }

    #[test]
    fn update_changes_kind() {
        let mut svc = service(vec![repo("a")]);
        let mut edited = repo("a");
        edited.kind = RepoKind::Path;
        svc.update(0, edited).unwrap();
        assert_eq!(svc.get(0).unwrap().kind, RepoKind::Path);
    }
}
