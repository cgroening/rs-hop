//! The entry-adding commands: `hop add`, `hop scan` and `hop doctor`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use clap::ValueEnum;
use sparcli::{Select, SparcliError};

use crate::cli::context::CliContext;
use crate::cli::output::{CliError, CliResult, Streams, info, success};
use crate::domain::doctor;
use crate::domain::repo::{Repo, RepoKind};
use crate::service::repo_service::RepoService;
use crate::util::paths;
use crate::util::scan::{self, ScanOptions};

/// The entry kind accepted by `hop add --kind`.
///
/// A typed value rather than a string, so an unrecognised kind is a usage error
/// from clap instead of silently falling back to a path entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum KindArg {
    /// A git work tree.
    Git,
    /// A file or folder.
    Path,
}

impl From<KindArg> for RepoKind {
    fn from(kind: KindArg) -> Self {
        match kind {
            KindArg::Git => RepoKind::Git,
            KindArg::Path => RepoKind::Path,
        }
    }
}

/// What `hop add` was asked to register.
pub struct AddRequest {
    /// Path to add (default: the working directory).
    pub path: Option<PathBuf>,
    /// Slug for `hop <slug>`.
    pub slug: Option<String>,
    /// Section to group the entry under.
    pub section: Option<String>,
    /// Display name (default: the path's basename).
    pub name: Option<String>,
    /// Entry kind; auto-detected when absent.
    pub kind: Option<KindArg>,
}

/// The `hop scan` options, grouped so the handler keeps a short parameter list.
pub struct ScanRequest {
    /// Directory to search (default: the working directory).
    pub dir: Option<PathBuf>,
    /// How deep to descend, if limited.
    pub depth: Option<usize>,
    /// Whether to keep descending into a work tree already found.
    pub nested: bool,
    /// Only report what would be imported.
    pub dry_run: bool,
    /// Import everything found without asking, and the way to run `scan`
    /// without a terminal.
    pub assume_yes: bool,
}

/// Registers an entry for a path, auto-detecting the kind unless told.
///
/// # Errors
///
/// Returns an error if the entry cannot be stored, for instance because its
/// slug is already taken.
pub fn add(
    service: &mut RepoService,
    request: AddRequest,
    streams: &mut Streams,
) -> CliResult {
    let raw = request.path.unwrap_or_else(|| PathBuf::from("."));
    let expanded = paths::expand_tilde(&raw.to_string_lossy());
    let absolute = std::path::absolute(&expanded).unwrap_or(expanded);
    let kind = resolve_kind(request.kind, absolute.join(".git").exists());
    let mut repo = Repo::new(absolute.clone());
    repo.name = request.name;
    repo.slug = request.slug;
    repo.section = request.section.clone();
    repo.kind = kind;
    service.add(repo)?;
    if let Some(section) = &request.section {
        // A failure here would leave the entry pointing at a section the list
        // does not know, so it is reported rather than discarded.
        service.ensure_section(kind, section)?;
    }
    success(
        streams,
        format!("Added {} ({})", absolute.display(), kind.as_config_value()),
    );
    Ok(())
}

/// The entry kind for `hop add`: an explicit `--kind`, else auto-detected
/// (`Git` when the path holds a `.git`, otherwise `Path`).
fn resolve_kind(explicit: Option<KindArg>, has_git_dir: bool) -> RepoKind {
    match explicit {
        Some(kind) => kind.into(),
        None if has_git_dir => RepoKind::Git,
        None => RepoKind::Path,
    }
}

/// Discovers git work trees under a directory and imports the chosen ones.
///
/// # Errors
///
/// Returns a usage error when a choice is needed but there is no terminal to
/// ask on, or a runtime error if the entries cannot be stored.
pub fn scan(
    service: &mut RepoService,
    request: ScanRequest,
    ctx: CliContext,
    streams: &mut Streams,
) -> CliResult {
    let raw = request.dir.clone().unwrap_or_else(|| PathBuf::from("."));
    let expanded = paths::expand_tilde(&raw.to_string_lossy());
    let root = std::path::absolute(&expanded).unwrap_or(expanded);
    let found = scan::find_git_repos(
        &root,
        ScanOptions {
            max_depth: request.depth,
            nested: request.nested,
        },
    );
    let known: HashSet<String> =
        service.repos().iter().map(|r| canon_key(&r.path)).collect();
    let (new, duplicates) = partition_found(&found, &known, canon_key);

    if new.is_empty() {
        info(
            streams,
            format!("No new git repos under {}.", root.display()),
        );
        return Ok(());
    }
    if request.dry_run {
        streams.line(&format!(
            "{} new git repo(s) under {}:",
            new.len(),
            root.display()
        ))?;
        for path in &new {
            streams.line(&format!("  {}", path.display()))?;
        }
        if !duplicates.is_empty() {
            info(streams, format!("({} already in hop)", duplicates.len()));
        }
        return Ok(());
    }
    if !duplicates.is_empty() {
        info(streams, format!("{} already in hop.", duplicates.len()));
    }
    let chosen = choose_repos(&new, &root, &request, ctx, streams)?;
    if chosen.is_empty() {
        info(streams, "Nothing selected.");
        return Ok(());
    }
    let count = chosen.len();
    let repos: Vec<Repo> = chosen
        .into_iter()
        .map(|path| {
            let mut repo = Repo::new(path);
            repo.kind = RepoKind::Git;
            repo
        })
        .collect();
    service.add_many(repos)?;
    success(streams, format!("Added {count} git repo(s)."));
    Ok(())
}

/// Which of the discovered repos to import: everything under `--yes`, else the
/// picker's selection.
///
/// Without a terminal and without `--yes` this is a usage error naming the flag
/// that supplies the answer, rather than a prompt that would hang in a pipeline.
fn choose_repos(
    new: &[PathBuf],
    root: &Path,
    request: &ScanRequest,
    ctx: CliContext,
    streams: &mut Streams,
) -> Result<Vec<PathBuf>, CliError> {
    if request.assume_yes {
        return Ok(new.to_vec());
    }
    if !ctx.is_interactive {
        return Err(CliError::usage(
            "`hop scan` needs a terminal to choose the repos; pass --yes to \
             import all of them, or --dry-run to only list them",
        ));
    }
    match pick_repos_to_import(new, root) {
        Ok(Some(chosen)) => Ok(chosen),
        Ok(None) => {
            info(streams, "Cancelled.");
            Ok(Vec::new())
        }
        Err(error) => Err(CliError::runtime(format!("terminal: {error}"))),
    }
}

/// Asks which of the discovered `found` repos to import, all pre-checked.
/// Returns `None` when the user cancels.
///
/// # Errors
///
/// Returns a [`SparcliError`] when the prompt cannot run.
fn pick_repos_to_import(
    found: &[PathBuf],
    root: &Path,
) -> Result<Option<Vec<PathBuf>>, SparcliError> {
    let prompt =
        format!("{} new git repo(s) under {}", found.len(), root.display());
    let options = found.iter().map(|path| scan_option_label(path));
    let outcome = Select::new(prompt)
        .options(options)
        .checked(0..found.len())
        .multi()
        .run_multi()?;
    let Some(indices) = outcome.submitted() else {
        return Ok(None);
    };
    Ok(Some(
        indices
            .into_iter()
            .map(|index| found[index].clone())
            .collect(),
    ))
}

/// One picker row: the repo's directory name, then its full path.
fn scan_option_label(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name.is_empty() {
        return path.display().to_string();
    }
    format!("{name}  {}", path.display())
}

/// Reports entry problems, failing when any are found.
///
/// # Errors
///
/// Returns a runtime error when at least one issue was diagnosed, so a calling
/// script sees a non-zero exit.
pub fn run_doctor(service: &RepoService, streams: &mut Streams) -> CliResult {
    let issues = doctor::diagnose(
        service.repos(),
        |path| path.exists(),
        |path| path.join(".git").exists(),
    );
    if issues.is_empty() {
        success(streams, "hop doctor: no issues.");
        return Ok(());
    }
    // The issues themselves are the payload; the summary is the failure, so
    // it is not also announced separately.
    for issue in &issues {
        streams.line(&format!("{issue}"))?;
    }
    Err(CliError::runtime(format!(
        "{} entry problem(s) found",
        issues.len()
    )))
}

/// Splits `found` into (new, already-known) by comparing each path's canonical
/// key against `known`. The `canon` function is injected so it is testable.
fn partition_found(
    found: &[PathBuf],
    known: &HashSet<String>,
    canon: impl Fn(&Path) -> String,
) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut new = Vec::new();
    let mut duplicates = Vec::new();
    for path in found {
        if known.contains(&canon(path)) {
            duplicates.push(path.clone());
        } else {
            new.push(path.clone());
        }
    }
    (new, duplicates)
}

/// A canonical comparison key for a path (resolves symlinks/`..` when possible,
/// else the path as-is).
fn canon_key(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::output::EXIT_USAGE;

    #[test]
    fn partition_found_splits_new_from_known() {
        let found = vec![PathBuf::from("/a"), PathBuf::from("/b")];
        let known: HashSet<String> = ["/a".to_string()].into_iter().collect();
        let (new, duplicates) =
            partition_found(&found, &known, |p| p.display().to_string());
        assert_eq!(new, vec![PathBuf::from("/b")]);
        assert_eq!(duplicates, vec![PathBuf::from("/a")]);
    }

    #[test]
    fn resolve_kind_honours_explicit_then_auto_detects() {
        assert_eq!(resolve_kind(Some(KindArg::Path), true), RepoKind::Path);
        assert_eq!(resolve_kind(Some(KindArg::Git), false), RepoKind::Git);
        assert_eq!(resolve_kind(None, true), RepoKind::Git);
        assert_eq!(resolve_kind(None, false), RepoKind::Path);
    }

    #[test]
    fn scanning_without_a_terminal_is_a_usage_error_naming_the_flag() {
        // Off a TTY the picker can never be answered, so the run must fail
        // early with exit 2 rather than block in a pipeline.
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut streams = Streams {
            out: &mut out,
            err: &mut err,
        };
        let request = ScanRequest {
            dir: None,
            depth: None,
            nested: false,
            dry_run: false,
            assume_yes: false,
        };
        let ctx = CliContext {
            is_interactive: false,
            is_output_tty: false,
        };
        let error = choose_repos(
            &[PathBuf::from("/a")],
            Path::new("/"),
            &request,
            ctx,
            &mut streams,
        )
        .expect_err("no terminal, no --yes: must fail");
        assert_eq!(error.code(), EXIT_USAGE);
        assert!(error.to_string().contains("--yes"));
    }

    #[test]
    fn assume_yes_imports_everything_without_a_terminal() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut streams = Streams {
            out: &mut out,
            err: &mut err,
        };
        let request = ScanRequest {
            dir: None,
            depth: None,
            nested: false,
            dry_run: false,
            assume_yes: true,
        };
        let ctx = CliContext {
            is_interactive: false,
            is_output_tty: false,
        };
        let found = vec![PathBuf::from("/a"), PathBuf::from("/b")];
        let chosen =
            choose_repos(&found, Path::new("/"), &request, ctx, &mut streams)
                .unwrap();
        assert_eq!(chosen, found);
    }
}
