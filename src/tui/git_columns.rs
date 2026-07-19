//! Shared text helpers for the git Standard columns (branch, status, GitHub and
//! ZIP Backup), used by both the flat table and the sectioned view so the two
//! renderers agree without duplicating the cell text.

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Local};

use crate::domain::repo::{GitInfo, Repo, RepoKind};
use crate::tui::presentation::{IconSet, status_text};

/// The git info to display for `repo`: example info in example mode, otherwise
/// the live info (which may still be loading).
pub fn effective_info(repo: &Repo, example_mode: bool) -> Option<&GitInfo> {
    if example_mode {
        repo.example_git_info.as_ref()
    } else {
        repo.git_info.as_ref()
    }
}

/// The branch text, or a loading marker / dash.
pub fn branch_text(info: Option<&GitInfo>) -> String {
    match info {
        None => "\u{2026}".to_string(),
        Some(info) => info
            .current_branch_name
            .clone()
            .unwrap_or_else(|| "-".to_string()),
    }
}

/// The GitHub name text, or a dash.
pub fn github_text(info: Option<&GitInfo>) -> String {
    info.and_then(|info| info.github_repo_name.clone())
        .unwrap_or_else(|| "-".to_string())
}

/// The plain status text used to size the status column (no colour): a loading
/// marker while unknown, a dash for a missing path, else the formatted status.
pub fn status_display(info: Option<&GitInfo>, icons: &IconSet) -> String {
    match info {
        None => "\u{2026}".to_string(),
        Some(info) if info.is_path_missing() => "-".to_string(),
        Some(info) => status_text(info, icons),
    }
}

/// The ZIP Backup cell text for `repo`: the excluded marker when the entry opts
/// out of the "backup all" run, else the last-backup date (`YYYY-MM-DD`) or a
/// dash when never backed up. Read from the precomputed map (no filesystem I/O).
pub fn zip_date_text(
    repo: &Repo,
    icons: &IconSet,
    zip_backups: &HashMap<PathBuf, DateTime<Local>>,
) -> String {
    if !repo.include_in_backup {
        return icons.excluded.to_string();
    }
    zip_backups
        .get(&repo.path)
        .map_or_else(|| "-".to_string(), |dt| dt.format("%Y-%m-%d").to_string())
}

/// Whether `repo` should show the error marker: a git entry flagged by its
/// gathered info (missing or invalid repository, or the example error in example
/// mode), else `false` (a path entry's marker is driven by the existence check).
pub fn git_marker_errored(repo: &Repo, example_mode: bool) -> bool {
    match repo.kind {
        RepoKind::Git if example_mode => repo.example_error().is_some(),
        RepoKind::Git => repo.entry_error().is_some(),
        RepoKind::Path => false,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::theme::GlyphVariant;

    fn icon_set() -> IconSet {
        IconSet::new(GlyphVariant::Unicode)
    }

    fn git_repo() -> Repo {
        Repo::new(PathBuf::from("/code/hop"))
    }

    fn info_on(branch: Option<&str>) -> GitInfo {
        GitInfo {
            valid: true,
            current_branch_name: branch.map(str::to_string),
            ..GitInfo::default()
        }
    }

    #[test]
    fn example_mode_reads_the_example_info_instead_of_the_live_one() {
        let mut repo = git_repo();
        repo.git_info = Some(info_on(Some("live")));
        repo.example_git_info = Some(info_on(Some("demo")));

        assert_eq!(
            branch_text(effective_info(&repo, false)),
            "live",
            "normally the gathered info wins"
        );
        assert_eq!(
            branch_text(effective_info(&repo, true)),
            "demo",
            "example mode never shows live git data"
        );
    }

    #[test]
    fn an_unknown_status_shows_the_loading_marker_not_a_dash() {
        // A dash would claim the repo has no branch; the ellipsis says the
        // background refresh has simply not reported yet.
        assert_eq!(branch_text(None), "\u{2026}");
        assert_eq!(status_display(None, &icon_set()), "\u{2026}");
    }

    #[test]
    fn a_repo_without_a_branch_or_remote_reads_as_a_dash() {
        let info = info_on(None);
        assert_eq!(branch_text(Some(&info)), "-");
        assert_eq!(github_text(Some(&info)), "-");
        assert_eq!(github_text(None), "-");
    }

    #[test]
    fn an_entry_opted_out_of_backup_shows_the_excluded_glyph() {
        let icons = icon_set();
        let mut repo = git_repo();
        let backups = HashMap::new();

        assert_eq!(zip_date_text(&repo, &icons, &backups), "-");

        repo.include_in_backup = false;
        assert_eq!(
            zip_date_text(&repo, &icons, &backups),
            icons.excluded.to_string(),
            "an excluded entry must not read as 'never backed up'"
        );
    }

    #[test]
    fn only_git_entries_are_flagged_by_their_gathered_info() {
        // A path entry's marker comes from the on-demand existence check, so it
        // must never be flagged here regardless of its (absent) git info.
        let mut path_entry = git_repo();
        path_entry.kind = RepoKind::Path;
        assert!(!git_marker_errored(&path_entry, false));

        // A git entry is flagged only once the background refresh reported it
        // as invalid - there is deliberately no per-frame filesystem stat.
        let mut unreported = git_repo();
        assert!(!git_marker_errored(&unreported, false));

        unreported.git_info = Some(GitInfo {
            valid: false,
            ..GitInfo::default()
        });
        assert!(git_marker_errored(&unreported, false));
    }
}
