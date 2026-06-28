//! Pure naming of repo ZIP backups, guaranteeing a unique file per repo.
//!
//! The file name is the repo's display name run through [`slug::slugify`], so it
//! is filesystem-safe (`[a-z0-9-]`) and human-readable. Two repos that would
//! otherwise share a name (e.g. `(rs) mdtask` and `(C++) mdtask`, both folder
//! `mdtask`) each get a short, stable path-hash suffix, so a backup never
//! silently overwrites another. The base-name collisions are also surfaced by
//! `hop doctor`.

use std::path::Path;

use crate::domain::repo::Repo;
use crate::domain::slug;

/// Fallback base name when a repo's display name slugifies to nothing.
const FALLBACK_BASE: &str = "backup";

/// The backup file name for `repo` (e.g. `rs-mdtask.zip`), unique within
/// `repos`: the slugified display name, plus a path-hash suffix when another git
/// repo shares that base name.
pub fn backup_filename(repo: &Repo, repos: &[Repo]) -> String {
    let base = base_name(repo);
    if shares_base_name(repo, repos, &base) {
        return format!("{base}-{}.zip", short_hash(&repo.path));
    }
    format!("{base}.zip")
}

/// The base file name (no extension, no disambiguator) for `repo`: its display
/// name slugified, or [`FALLBACK_BASE`] when that is empty.
pub(crate) fn base_name(repo: &Repo) -> String {
    let base = slug::slugify(&repo.display_name());
    if base.is_empty() {
        return FALLBACK_BASE.to_string();
    }
    base
}

/// Whether another entry in `repos` produces the same `base` name (so `repo`
/// needs a disambiguating suffix). `repo` is matched by path, not skipped by
/// position, so duplicate-path configs still count as a collision. Both git
/// repos and file/folder entries are considered, since both produce archives.
fn shares_base_name(repo: &Repo, repos: &[Repo], base: &str) -> bool {
    repos
        .iter()
        .filter(|other| other.path != repo.path)
        .any(|other| base_name(other) == base)
}

/// A short, stable hex digest of `path` (FNV-1a 64-bit, 8 hex chars). Used only
/// to disambiguate colliding base names; chosen over `DefaultHasher` because the
/// latter is not stable across Rust releases (the file name must stay constant).
fn short_hash(path: &Path) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for byte in path.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{:08x}", hash & 0xffff_ffff)
}

/// Entries that share a base backup name (before disambiguation), as
/// `(base, display names)` pairs sorted by base name. Backs the `hop doctor`
/// duplicate-backup-name check; both git repos and file/folder entries count.
pub fn duplicate_base_names(repos: &[Repo]) -> Vec<(String, Vec<String>)> {
    use std::collections::BTreeMap;
    let mut by_base: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for repo in repos.iter() {
        by_base
            .entry(base_name(repo))
            .or_default()
            .push(repo.display_name());
    }
    by_base
        .into_iter()
        .filter(|(_, names)| names.len() > 1)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn git(name: &str, path: &str) -> Repo {
        let mut repo = Repo::new(PathBuf::from(path));
        repo.name = Some(name.to_string());
        repo
    }

    #[test]
    fn distinct_names_keep_clean_filenames() {
        let repos = vec![
            git("(rs) hop", "/code/rust/hop"),
            git("(rs) mdtask", "/code/rust/mdtask"),
        ];
        assert_eq!(backup_filename(&repos[0], &repos), "rs-hop.zip");
        assert_eq!(backup_filename(&repos[1], &repos), "rs-mdtask.zip");
    }

    #[test]
    fn same_base_name_gets_distinct_hash_suffixes() {
        let repos = vec![
            git("mdtask", "/code/rust/mdtask"),
            git("mdtask", "/code/cpp/mdtask"),
        ];
        let a = backup_filename(&repos[0], &repos);
        let b = backup_filename(&repos[1], &repos);
        assert_ne!(a, b);
        assert!(a.starts_with("mdtask-") && a.ends_with(".zip"));
        assert!(b.starts_with("mdtask-") && b.ends_with(".zip"));
    }

    #[test]
    fn git_and_folder_sharing_a_base_name_both_get_suffixes() {
        let mut folder = git("mdtask", "/notes/mdtask");
        folder.kind = crate::domain::repo::RepoKind::Path;
        let repos = vec![git("mdtask", "/code/rust/mdtask"), folder];
        let a = backup_filename(&repos[0], &repos);
        let b = backup_filename(&repos[1], &repos);
        assert_ne!(a, b);
        assert!(a.starts_with("mdtask-") && b.starts_with("mdtask-"));
    }

    #[test]
    fn empty_slug_name_falls_back() {
        let repos = vec![git("+++", "/code/weird")];
        // "+++" slugifies to empty; display name falls back, base to "backup".
        assert_eq!(base_name(&repos[0]), "backup");
        assert_eq!(backup_filename(&repos[0], &repos), "backup.zip");
    }

    #[test]
    fn hash_is_deterministic() {
        let path = Path::new("/code/rust/mdtask");
        assert_eq!(short_hash(path), short_hash(path));
        assert_eq!(short_hash(path).len(), 8);
    }

    #[test]
    fn duplicate_base_names_lists_collisions() {
        let repos = vec![
            git("mdtask", "/code/rust/mdtask"),
            git("mdtask", "/code/cpp/mdtask"),
            git("(rs) hop", "/code/rust/hop"),
        ];
        let dups = duplicate_base_names(&repos);
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0].0, "mdtask");
        assert_eq!(dups[0].1.len(), 2);
    }
}
