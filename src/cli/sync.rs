//! `hop pull` and `hop push`: sync the named git entries, or every active one.
//!
//! Each repository gets one line on stderr as it finishes, then a summary;
//! stdout stays empty, because nothing here is payload a caller would pipe on.

use std::path::PathBuf;

use sparcli::{Confirm, List, SparcliError, Style, Theme};
use unicode_width::UnicodeWidthStr;

use crate::cli::context::CliContext;
use crate::cli::output::{CliError, CliResult, Streams, info};
use crate::domain::repo::RepoKind;
use crate::domain::sync::{Decision, Summary, SyncOp, SyncOutcome};
use crate::service::repo_service::RepoService;
use crate::service::sync_service::{self, SyncPlan};
use crate::storage::git_client::GitClient;
use crate::util::interrupt;

/// Columns between the repo name and its outcome.
const NAME_GAP: usize = 2;

/// The leading mark of a result line, telling success from a problem at a
/// glance. A line per repository is a list item rather than an alert, which
/// would draw a box around every single line.
#[derive(Clone, Copy)]
enum Mark {
    /// Pulled or pushed.
    Updated,
    /// Nothing to move.
    Idle,
    /// Left alone on purpose.
    Skipped,
    /// The sync failed.
    Failed,
    /// `--dry-run`: would run.
    Planned,
}

impl Mark {
    /// The mark for a finished repository.
    fn of(outcome: &SyncOutcome) -> Self {
        match outcome {
            SyncOutcome::Updated => Mark::Updated,
            SyncOutcome::UpToDate => Mark::Idle,
            SyncOutcome::Skipped(_) => Mark::Skipped,
            SyncOutcome::Failed(_) => Mark::Failed,
        }
    }

    /// The glyph, from the Unicode or the ASCII tier.
    fn glyph(self, is_unicode: bool) -> &'static str {
        let (unicode, ascii) = match self {
            Mark::Updated => ("\u{2713}", "+"),
            Mark::Idle => ("\u{b7}", "="),
            Mark::Skipped => ("!", "!"),
            Mark::Failed => ("\u{2717}", "x"),
            Mark::Planned => ("\u{2192}", ">"),
        };
        if is_unicode { unicode } else { ascii }
    }

    /// The colour role of the glyph.
    fn style(self, theme: &Theme) -> Style {
        match self {
            Mark::Updated => theme.success,
            Mark::Idle => theme.secondary,
            Mark::Skipped => theme.warning,
            Mark::Failed => theme.error,
            Mark::Planned => theme.info,
        }
    }
}

/// What `hop pull` / `hop push` was asked to do.
pub struct SyncRequest {
    /// The direction and network settings of the run.
    pub plan: SyncPlan,
    /// The slugs to sync; empty means every active git entry.
    pub slugs: Vec<String>,
    /// With no slugs, also sync the archived git entries.
    pub is_including_archived: bool,
    /// Skip the question before syncing every entry.
    pub assume_yes: bool,
    /// Only show what would happen.
    pub dry_run: bool,
}

/// Where the repositories come from and how git reaches them.
#[derive(Clone, Copy)]
pub struct Repos<'a> {
    /// The entries, to resolve slugs and list the git repos.
    pub service: &'a RepoService,
    /// Runs the pulls and pushes.
    pub client: &'a dyn GitClient,
}

/// A repository picked for the run.
struct Target {
    /// The name shown in the output.
    name: String,
    /// The work tree.
    path: PathBuf,
}

/// Pulls or pushes the requested git entries, one after another.
///
/// # Errors
///
/// Returns a usage error for an unknown or non-git slug, or when the run would
/// need to ask but has no terminal; a runtime error when any repository fails.
pub fn run(
    repos: Repos,
    request: SyncRequest,
    ctx: CliContext,
    streams: &mut Streams,
) -> CliResult {
    let op = request.plan.op;
    let client = repos.client;
    let targets = resolve_targets(repos.service, &request)?;
    if targets.is_empty() {
        info(streams, format!("No git repos to {}.", op.verb()));
        return Ok(());
    }
    if !is_confirmed(&request, targets.len(), ctx, streams)? {
        return Ok(());
    }
    let width = name_width(&targets);
    if request.dry_run {
        return preview(client, &targets, op, width, streams);
    }
    let mut outcomes = Vec::with_capacity(targets.len());
    for target in targets {
        if interrupt::is_interrupted() {
            break;
        }
        let report = sync_service::sync_one(client, target.path, request.plan);
        report_line(streams, &target.name, width, op, &report.outcome);
        outcomes.push(report.outcome);
    }
    finish(streams, Summary::of(op, &outcomes))
}

/// The repositories the request names: its slugs in order, else every git
/// entry that is not archived (all of them with `--archive`).
fn resolve_targets(
    service: &RepoService,
    request: &SyncRequest,
) -> Result<Vec<Target>, CliError> {
    if request.slugs.is_empty() {
        return Ok(service
            .repos()
            .iter()
            .filter(|repo| repo.kind == RepoKind::Git)
            .filter(|repo| request.is_including_archived || !repo.archived)
            .map(|repo| Target {
                name: repo.display_name(),
                path: repo.path.clone(),
            })
            .collect());
    }
    request
        .slugs
        .iter()
        .map(|slug| target_for_slug(service, slug))
        .collect()
}

/// The git entry carrying `slug`.
fn target_for_slug(
    service: &RepoService,
    slug: &str,
) -> Result<Target, CliError> {
    let repo = service
        .index_by_slug(slug)
        .and_then(|index| service.get(index))
        .ok_or_else(|| {
            CliError::usage(format!("unknown slug '{slug}' (see `hop list`)"))
        })?;
    if repo.kind != RepoKind::Git {
        return Err(CliError::usage(format!(
            "'{slug}' is not a git repo, so it cannot be pulled or pushed"
        )));
    }
    Ok(Target {
        name: repo.display_name(),
        path: repo.path.clone(),
    })
}

/// Whether the run may go ahead. Naming slugs, `--yes` and `--dry-run` need
/// no answer; syncing every entry asks first, and fails early without a
/// terminal to ask on.
fn is_confirmed(
    request: &SyncRequest,
    count: usize,
    ctx: CliContext,
    streams: &mut Streams,
) -> Result<bool, CliError> {
    if !request.slugs.is_empty() || request.assume_yes || request.dry_run {
        return Ok(true);
    }
    let verb = request.plan.op.verb();
    if !ctx.is_interactive {
        return Err(CliError::usage(format!(
            "`hop {verb}` asks before touching every repo and needs a \
             terminal for that; pass --yes to {verb} them all, or name the \
             slugs to {verb}"
        )));
    }
    match ask(request.plan.op, count) {
        Ok(true) => Ok(true),
        Ok(false) => {
            info(streams, "Cancelled.");
            Ok(false)
        }
        Err(error) => Err(CliError::runtime(format!("terminal: {error}"))),
    }
}

/// Asks "Pull 12 repos?", defaulting to no.
///
/// # Errors
///
/// Returns a [`SparcliError`] when the prompt cannot run.
fn ask(op: SyncOp, count: usize) -> Result<bool, SparcliError> {
    let noun = if count == 1 { "repo" } else { "repos" };
    let question = format!("{} {count} {noun}?", op.title());
    let outcome = Confirm::new(question).run()?;
    Ok(outcome.submitted().unwrap_or(false))
}

/// Writes what each repository would get, without pulling or pushing.
fn preview(
    client: &dyn GitClient,
    targets: &[Target],
    op: SyncOp,
    width: usize,
    streams: &mut Streams,
) -> CliResult {
    for target in targets {
        let (mark, text) =
            match sync_service::preview_one(client, &target.path, op) {
                Decision::Run => {
                    (Mark::Planned, format!("would {}", op.verb()))
                }
                Decision::Settled(outcome) => {
                    (Mark::of(&outcome), outcome.describe(op))
                }
            };
        write_marked(streams, mark, padded(&target.name, width, &text))?;
    }
    Ok(())
}

/// Writes one repository's outcome, its icon telling success from a problem.
fn report_line(
    streams: &mut Streams,
    name: &str,
    width: usize,
    op: SyncOp,
    outcome: &SyncOutcome,
) {
    let line = padded(name, width, &outcome.describe(op));
    // A closed stderr must not stop the run: the next repo still syncs.
    let _ = write_marked(streams, Mark::of(outcome), line);
}

/// Writes `line` to stderr behind `mark`, in the theme's colours.
fn write_marked(streams: &mut Streams, mark: Mark, line: String) -> CliResult {
    let theme = sparcli::theme();
    let item = List::new()
        .bullet(mark.glyph(theme.unicode))
        .marker_style(mark.style(&theme))
        .item(line);
    streams.message(&item)
}

/// Writes the summary and turns any failure into the exit code.
fn finish(streams: &mut Streams, summary: Summary) -> CliResult {
    info(streams, summary.to_string());
    if !summary.has_failures() {
        return Ok(());
    }
    let noun = if summary.failed == 1 { "repo" } else { "repos" };
    Err(CliError::runtime(format!(
        "{} {noun} failed to {}",
        summary.failed,
        summary.op.verb()
    )))
}

/// The display width of the longest target name.
fn name_width(targets: &[Target]) -> usize {
    targets
        .iter()
        .map(|target| UnicodeWidthStr::width(target.name.as_str()))
        .max()
        .unwrap_or(0)
}

/// `name` padded to `width`, a gap, then `text`.
fn padded(name: &str, width: usize, text: &str) -> String {
    let padding = width.saturating_sub(UnicodeWidthStr::width(name));
    format!("{name}{}{text}", " ".repeat(padding + NAME_GAP))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::domain::repo::{GitInfo, Repo};
    use crate::domain::stats::GitStats;
    use crate::service::sync_service::SyncOptions;
    use crate::storage::in_memory_repository::InMemoryRepoRepository;

    /// A client that reports every repo as one commit ahead and behind; a
    /// push to a path containing "fail" is rejected.
    struct FakeClient;

    impl GitClient for FakeClient {
        fn collect(&self, _path: &Path) -> GitInfo {
            GitInfo {
                valid: true,
                current_branch_name: Some("main".to_string()),
                changes: Some(0),
                ahead: Some(1),
                behind: Some(1),
                ..GitInfo::default()
            }
        }
        fn fetch(&self, _path: &Path) {}
        fn pull(&self, _path: &Path, _options: SyncOptions) -> SyncOutcome {
            SyncOutcome::Updated
        }
        fn push(&self, path: &Path, _options: SyncOptions) -> SyncOutcome {
            if path.to_string_lossy().contains("fail") {
                return SyncOutcome::Failed("rejected".to_string());
            }
            SyncOutcome::Updated
        }
        fn log(&self, _path: &Path, _max: usize) -> Vec<String> {
            Vec::new()
        }
        fn stats(&self, _path: &Path) -> GitStats {
            GitStats::default()
        }
    }

    /// A git entry at `path` with `slug`.
    fn git(path: &str, slug: &str) -> Repo {
        let mut repo = Repo::new(PathBuf::from(path));
        repo.kind = RepoKind::Git;
        repo.slug = Some(slug.to_string());
        repo
    }

    fn service(repos: Vec<Repo>) -> RepoService {
        let dir = std::env::temp_dir()
            .join(format!("hop-cli-sync-{}", std::process::id()));
        RepoService::new(
            Box::new(InMemoryRepoRepository::new(repos)),
            dir.join("usage.toml"),
            dir.join("selected-repo.txt"),
        )
        .expect("an in-memory service always loads")
    }

    fn request(op: SyncOp, slugs: &[&str]) -> SyncRequest {
        SyncRequest {
            plan: SyncPlan {
                op,
                options: SyncOptions { timeout_secs: 1 },
            },
            slugs: slugs.iter().map(|slug| (*slug).to_string()).collect(),
            is_including_archived: false,
            assume_yes: false,
            dry_run: false,
        }
    }

    const NO_TERMINAL: CliContext = CliContext {
        is_interactive: false,
        is_output_tty: false,
    };

    /// Runs `request` and returns the result with stdout and stderr.
    fn run_with(
        service: &RepoService,
        request: SyncRequest,
    ) -> (CliResult, String, String) {
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let result = {
            let mut streams = Streams {
                out: &mut out,
                err: &mut err,
            };
            let repos = Repos {
                service,
                client: &FakeClient,
            };
            run(repos, request, NO_TERMINAL, &mut streams)
        };
        let text = |bytes: Vec<u8>| String::from_utf8(bytes).expect("utf-8");
        (result, text(out), text(err))
    }

    #[test]
    fn named_slugs_sync_without_asking_and_keep_stdout_empty() {
        let service = service(vec![git("/a", "a"), git("/b", "b")]);
        let (result, out, err) =
            run_with(&service, request(SyncOp::Pull, &["b"]));
        assert!(result.is_ok());
        assert!(out.is_empty());
        assert!(err.contains("b  pulled"), "{err}");
        assert!(!err.contains("a  pulled"), "{err}");
        assert!(err.contains("pulled 1"), "{err}");
    }

    #[test]
    fn syncing_everything_without_a_terminal_names_the_flag() {
        let service = service(vec![git("/a", "a")]);
        let (result, _, _) = run_with(&service, request(SyncOp::Push, &[]));
        let error = result.expect_err("no terminal to ask on");
        assert_eq!(error.code(), crate::cli::output::EXIT_USAGE);
        assert!(error.to_string().contains("--yes"));
    }

    #[test]
    fn a_failed_repo_fails_the_run_after_the_others_ran() {
        let service = service(vec![git("/fail", "x"), git("/ok", "y")]);
        let mut push_all = request(SyncOp::Push, &[]);
        push_all.assume_yes = true;
        let (result, _, err) = run_with(&service, push_all);
        let error = result.expect_err("one push was rejected");
        assert_eq!(error.code(), crate::cli::output::EXIT_ERROR);
        assert!(err.contains("ok    pushed"), "{err}");
        assert!(err.contains("failed: rejected"), "{err}");
    }

    #[test]
    fn archived_repos_join_only_with_the_flag() {
        let mut archived = git("/old", "old");
        archived.archived = true;
        let service = service(vec![git("/new", "new"), archived]);
        let mut pull_all = request(SyncOp::Pull, &[]);
        pull_all.assume_yes = true;
        let (_, _, err) = run_with(&service, pull_all);
        assert!(err.contains("pulled 1"), "{err}");
        let mut with_archive = request(SyncOp::Pull, &[]);
        with_archive.assume_yes = true;
        with_archive.is_including_archived = true;
        let (_, _, err) = run_with(&service, with_archive);
        assert!(err.contains("pulled 2"), "{err}");
    }

    #[test]
    fn a_dry_run_only_says_what_would_happen() {
        let service = service(vec![git("/a", "a")]);
        let mut dry = request(SyncOp::Pull, &[]);
        dry.dry_run = true;
        let (result, out, err) = run_with(&service, dry);
        assert!(result.is_ok());
        assert!(out.is_empty());
        assert!(err.contains("would pull"), "{err}");
    }

    #[test]
    fn an_unknown_or_non_git_slug_is_a_usage_error() {
        let mut folder = Repo::new(PathBuf::from("/notes"));
        folder.kind = RepoKind::Path;
        folder.slug = Some("notes".to_string());
        let service = service(vec![git("/a", "a"), folder]);
        for slug in ["nope", "notes"] {
            let (result, _, _) =
                run_with(&service, request(SyncOp::Pull, &[slug]));
            let error = result.expect_err("not a git slug");
            assert_eq!(error.code(), crate::cli::output::EXIT_USAGE, "{slug}");
        }
    }
}
