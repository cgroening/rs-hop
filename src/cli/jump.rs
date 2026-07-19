//! `hop <slug>`: the fast jump, captured as clap's external subcommand.

use std::path::{Path, PathBuf};

use crate::cli::output::{CliError, CliResult};
use crate::config::Config;
use crate::domain::repo::{Repo, RepoKind, is_dir_target};
use crate::service::repo_service::RepoService;
use crate::storage::git_client::GitClient;
use crate::storage::subprocess_git_client::SubprocessGitClient;

/// Records the open and writes the handoff `cd` path for the entry `slug`.
///
/// This only `cd`s - it launches neither the git tool nor an editor, mirroring
/// the TUI's `Enter`.
///
/// # Errors
///
/// Returns an error when no slug was given, when no entry carries it, or when
/// the handoff path cannot be written.
pub fn run(
    config: &Config,
    service: &mut RepoService,
    args: &[String],
    is_fetching: bool,
) -> CliResult {
    let Some(slug) = parse_jump_args(args) else {
        return Err(CliError::usage(
            "no slug given (see `hop --help` for the available commands)",
        ));
    };
    let Some(index) = service.index_by_slug(&slug) else {
        return Err(CliError::usage(format!(
            "unknown command or slug '{slug}' (see `hop --help`)"
        )));
    };
    let Some(repo) = service.get(index).cloned() else {
        return Err(CliError::runtime(format!("entry '{slug}' vanished")));
    };
    service.mark_used(index)?;
    // `--fetch`: update the repo's remote refs so the next status is fresh.
    if is_fetching && repo.kind == RepoKind::Git {
        SubprocessGitClient::new(config.github_username.clone())
            .fetch(&repo.path);
    }
    service.write_selected(&jump_target(&repo))?;
    Ok(())
}

/// The `cd` target for a jump: a file's parent, otherwise the entry path.
fn jump_target(repo: &Repo) -> PathBuf {
    if repo.kind == RepoKind::Path && !is_dir_target(&repo.path) {
        return repo
            .path
            .parent()
            .map_or_else(|| repo.path.clone(), Path::to_path_buf);
    }
    repo.path.clone()
}

/// The slug from the captured external-subcommand arguments.
///
/// The deprecated `-s`/`--save-only` flag is still accepted and skipped, so an
/// old `hop <slug> -s` keeps working.
fn parse_jump_args(args: &[String]) -> Option<String> {
    args.iter().find(|arg| !arg.starts_with('-')).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_jump_args_takes_the_first_non_flag() {
        assert_eq!(
            parse_jump_args(&["hop".to_string()]),
            Some("hop".to_string())
        );
        assert_eq!(
            parse_jump_args(&["-s".to_string(), "hop".to_string()]),
            Some("hop".to_string())
        );
        assert_eq!(parse_jump_args(&["-s".to_string()]), None);
        assert_eq!(parse_jump_args(&[]), None);
    }

    #[test]
    fn jump_target_uses_parent_only_for_files() {
        let mut folder = Repo::new(PathBuf::from("/code/hop/"));
        folder.kind = RepoKind::Path;
        assert_eq!(jump_target(&folder), PathBuf::from("/code/hop/"));

        let mut file = Repo::new(PathBuf::from("/code/notes.md"));
        file.kind = RepoKind::Path;
        assert_eq!(jump_target(&file), PathBuf::from("/code"));

        // A git entry always jumps to the work tree itself.
        let git = Repo::new(PathBuf::from("/code/hop"));
        assert_eq!(jump_target(&git), PathBuf::from("/code/hop"));
    }
}
