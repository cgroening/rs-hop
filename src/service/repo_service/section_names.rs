//! Managing the per-kind section names: creating, renaming, deleting and
//! reordering them, and keeping the entries that reference them in step.
//!
//! The git and path lists are independent namespaces, so a rename or a
//! delete only ever rewrites entries of the same kind.

use super::RepoService;
use crate::domain::error::{Error, Result};
use crate::domain::repo::RepoKind;
use crate::domain::sections;

impl RepoService {
    /// The ordered list of section names for `kind` (each kind has its own
    /// independent namespace).
    pub fn sections(&self, kind: RepoKind) -> &[String] {
        self.sections_of(kind)
    }

    /// Appends a new section to `kind`'s namespace, validating its name.
    ///
    /// # Errors
    ///
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
    ///
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
    ///
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
    ///
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
    ///
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
}
