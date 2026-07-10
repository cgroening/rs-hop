//! CLI layer: the `clap` parser and one handler per command.
//!
//! A bare invocation opens the interactive TUI; `hop <slug>` jumps straight to
//! an entry; `list`, `import` and `config-path` are utility commands. This is
//! the composition root for commands: it resolves the config path, wires the
//! TOML repository into the [`RepoService`] and the subprocess git client, then
//! dispatches.

mod demo;
mod output;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use sparcli::{
    Alert, Cell, Color as UiColor, Column, Renderable, Select, SparcliError,
    Style as SpStyle, Table, Theme, set_theme,
};

use crate::config::Config;
use crate::config::loader::load_config;
use crate::domain::doctor;
use crate::domain::repo::{Repo, RepoKind, is_dir_target};
use crate::service::repo_service::RepoService;
use crate::storage::git_client::GitClient;
use crate::storage::in_memory_repository::InMemoryRepoRepository;
use crate::storage::repository::RepoRepository;
use crate::storage::subprocess_git_client::SubprocessGitClient;
use crate::storage::toml_repo_repository::TomlRepoRepository;
use crate::theme::{Color, GlyphVariant};
use crate::tui::{self, App, RunOutcome, StartupStatus, Tui};
use crate::util::app_info::{APP_ABOUT, APP_NAME, APP_VERSION};
use crate::util::opener::{
    launch_git_tool, open_default_app, open_in_editor, resolve_editor,
};
use crate::util::paths;
use crate::util::scan::{self, ScanOptions};

/// Environment override for the config file path.
const CONFIG_ENV: &str = "HOP_CONFIG";

/// The hop command line.
#[derive(Debug, Parser)]
#[command(name = APP_NAME, version = APP_VERSION, about = APP_ABOUT)]
pub struct Cli {
    /// Use a specific config file (default: $XDG_CONFIG_HOME/hop/config.toml).
    #[arg(short = 'C', long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,
    /// Deprecated/no-op: `hop <slug>` only writes the path (`cd`) anyway.
    #[arg(short = 's', long, global = true)]
    save_only: bool,
    /// Run `git fetch` before gathering status.
    #[arg(long, global = true)]
    fetch: bool,
    /// Use cached status only; do not run git.
    #[arg(long, global = true)]
    cached: bool,
    /// Launch with built-in demo data for screenshots (no real git/config).
    #[arg(long, global = true)]
    demo: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

/// The available commands; absence opens the interactive TUI.
#[derive(Debug, Subcommand)]
enum Command {
    /// List entries as plain text (for scripting).
    List,
    /// Print the resolved config file path.
    #[command(name = "config-path")]
    ConfigPath,
    /// Add an entry for a path (default: the current directory).
    Add {
        /// Path to add (default: `.`).
        path: Option<PathBuf>,
        /// Slug for `hop <slug>`.
        #[arg(long)]
        slug: Option<String>,
        /// Section (Files tab grouping).
        #[arg(long)]
        section: Option<String>,
        /// Display name (default: the path's basename).
        #[arg(long)]
        name: Option<String>,
        /// Entry kind: `git` or `path` (default: auto-detect).
        #[arg(long)]
        kind: Option<String>,
    },
    /// Scan a directory for git repos and import the chosen ones.
    Scan {
        /// Directory to scan (default: `.`).
        dir: Option<PathBuf>,
        /// Maximum recursion depth below the directory.
        #[arg(long)]
        depth: Option<usize>,
        /// Keep descending into found repos (find nested repos/submodules).
        #[arg(long)]
        nested: bool,
        /// Only list what would be imported; do not change the config.
        #[arg(long)]
        dry_run: bool,
    },
    /// Report problems with the entries (missing paths, bad/duplicate slugs).
    Doctor,
    /// Jump to a slug (`hop <slug>`).
    #[command(external_subcommand)]
    Jump(Vec<String>),
}

/// Parses arguments and runs the requested command.
#[must_use]
pub fn run(cli: Cli) -> ExitCode {
    let config_path = resolve_config_path(&cli);
    match &cli.command {
        Some(Command::ConfigPath) => {
            println!("{}", config_path.display());
            ExitCode::SUCCESS
        }
        _ => run_with_service(cli, config_path),
    }
}

/// Builds the service and dispatches the commands that need it.
fn run_with_service(cli: Cli, config_path: PathBuf) -> ExitCode {
    let config = match load_config(&config_path) {
        Ok(config) => config,
        Err(error) => return output::report_error(&error),
    };
    // Style sparcli's CLI output from the same config palette as the TUI.
    apply_sparcli_theme(&config);
    if cli.command.is_none() && cli.demo {
        return run_demo(config);
    }
    let service = match build_service(&config_path) {
        Ok(service) => service,
        Err(error) => return output::report_error(&error),
    };
    match &cli.command {
        None => {
            let startup = startup_status(&cli, &config);
            run_tui(config, service, startup)
        }
        Some(Command::List) => cmd_list(&config, &service, &cli),
        Some(Command::Jump(args)) => cmd_jump(&config, service, args, &cli),
        Some(Command::Add {
            path,
            slug,
            section,
            name,
            kind,
        }) => cmd_add(service, path.clone(), slug, section, name, kind),
        Some(Command::Scan {
            dir,
            depth,
            nested,
            dry_run,
        }) => cmd_scan(
            service,
            ScanRequest {
                dir: dir.clone(),
                depth: *depth,
                nested: *nested,
                dry_run: *dry_run,
            },
        ),
        Some(Command::Doctor) => cmd_doctor(&service),
        // ConfigPath is handled before the service is built.
        Some(Command::ConfigPath) => ExitCode::SUCCESS,
    }
}

/// Derives the TUI's startup status source from the flags and config:
/// `--cached` shows only the cache, otherwise refresh (fetching first when
/// `--fetch` is set or `fetch_on_start` is enabled).
fn startup_status(cli: &Cli, config: &Config) -> StartupStatus {
    if cli.cached {
        return StartupStatus::Cached;
    }
    StartupStatus::Refresh {
        fetch: cli.fetch || config.fetch_on_start,
    }
}

/// Resolves the config path: `--config`, then `HOP_CONFIG`, then the default.
fn resolve_config_path(cli: &Cli) -> PathBuf {
    if let Some(path) = &cli.config {
        return path.clone();
    }
    if let Ok(value) = std::env::var(CONFIG_ENV)
        && !value.is_empty()
    {
        return PathBuf::from(value);
    }
    paths::config_file()
}

/// Builds a [`RepoService`] backed by the TOML file at `config_path`.
fn build_service(
    config_path: &Path,
) -> crate::domain::error::Result<RepoService> {
    let repository = TomlRepoRepository::new(config_path.to_path_buf());
    RepoService::new(
        Box::new(repository),
        paths::usage_file(),
        paths::selected_repo_file(),
    )
}

/// Opens the interactive TUI and performs the chosen post-exit action.
fn run_tui(
    config: Config,
    service: RepoService,
    startup: StartupStatus,
) -> ExitCode {
    let git_client =
        Arc::new(SubprocessGitClient::new(config.github_username.clone()));
    let app = App::new(
        config.clone(),
        service,
        git_client,
        paths::cache_file(),
        paths::ui_state_file(),
        startup,
    );
    run_app(&config, app)
}

/// Launches the TUI from the built-in demo dataset for screenshots: example
/// mode is forced on (no git, no refresh) and all persisted state is routed to
/// a throwaway temp directory, so a demo run never reads or writes real files.
fn run_demo(mut config: Config) -> ExitCode {
    config.example_mode = true;
    let dir = std::env::temp_dir().join("hop-demo");
    let repository = InMemoryRepoRepository::new(demo::repos());
    let _ = repository.save_sections(&demo::sections());
    let service = match RepoService::new(
        Box::new(repository),
        dir.join("usage.toml"),
        dir.join("selected-repo.txt"),
    ) {
        Ok(service) => service,
        Err(error) => return output::report_error(&error),
    };
    let git_client =
        Arc::new(SubprocessGitClient::new(config.github_username.clone()));
    let app = App::new(
        config.clone(),
        service,
        git_client,
        dir.join("git-info-cache.toml"),
        dir.join("ui-state.toml"),
        StartupStatus::Cached,
    );
    run_app(&config, app)
}

/// Runs the prepared app in a terminal and performs the post-exit action.
fn run_app(config: &Config, app: App) -> ExitCode {
    let mut tui = match Tui::new() {
        Ok(tui) => tui,
        Err(error) => return output::fail(&format!("terminal: {error}")),
    };
    let outcome = tui::run(app, &mut tui);
    drop(tui);
    match outcome {
        Ok(outcome) => perform_outcome(config, outcome),
        Err(error) => output::fail(&format!("terminal: {error}")),
    }
}

/// Acts on the TUI outcome after the terminal is restored.
fn perform_outcome(config: &Config, outcome: RunOutcome) -> ExitCode {
    match outcome {
        RunOutcome::Quit | RunOutcome::Jumped => ExitCode::SUCCESS,
        RunOutcome::LaunchGitTool(path) => {
            launch_tool(config, &path);
            ExitCode::SUCCESS
        }
        // The run loop handles the inline overlay and never returns it here.
        RunOutcome::LaunchGitToolInline(_) => ExitCode::SUCCESS,
        RunOutcome::OpenFile(path) => {
            let editor = resolve_editor(config.editor.as_deref());
            let _ = open_in_editor(&editor, &path);
            ExitCode::SUCCESS
        }
        RunOutcome::OpenWith(path) => {
            let _ = open_default_app(&path);
            ExitCode::SUCCESS
        }
    }
}

/// Launches the configured git tool in `dir`, reporting if it is missing.
fn launch_tool(config: &Config, dir: &Path) {
    let Some(program) = &config.git_program else {
        return;
    };
    if let Err(error) = launch_git_tool(program, dir) {
        eprintln!("{APP_NAME}: could not launch {program}: {error}");
    }
}

/// Handles `hop list`, printing entries as plain text.
fn cmd_list(_config: &Config, service: &RepoService, cli: &Cli) -> ExitCode {
    let repos = service.repos();
    if repos.is_empty() {
        let _ = Alert::info(
            "No entries yet. Run hop to add some, or hop add <path>.",
        )
        .print();
        return ExitCode::SUCCESS;
    }
    // On a terminal, render a styled sparcli table; when piped, keep the plain
    // tab-separated lines so scripts consuming `hop list` are unaffected.
    if sparcli::terminal::is_output_tty() {
        print_list_table(repos);
    } else {
        for repo in repos {
            println!("{}", list_line(repo));
        }
    }
    // `list` prints stored fields only (no git status), so the status flags do
    // not apply here; they affect the TUI and `hop <slug> --fetch`.
    let _ = (cli.fetch, cli.cached);
    ExitCode::SUCCESS
}

/// Renders the entry list as a styled sparcli table.
fn print_list_table(repos: &[Repo]) {
    let mut table = Table::new().columns([
        Column::new("Slug"),
        Column::new("Name"),
        Column::new("Kind"),
        Column::new("Path"),
        Column::new("Flags"),
    ]);
    for repo in repos {
        table = table.row([
            Cell::new(repo.slug.clone().unwrap_or_default()),
            Cell::new(repo.display_name().to_string()),
            Cell::new(repo.kind.as_config_value().to_string()),
            Cell::new(repo.path.display().to_string()),
            Cell::new(entry_flags(repo)),
        ]);
    }
    let _ = table.print();
}

/// The comma-separated `fav`/`archived` flags of an entry (empty when none).
fn entry_flags(repo: &Repo) -> String {
    let mut flags = Vec::new();
    if repo.fav {
        flags.push("fav");
    }
    if repo.archived {
        flags.push("archived");
    }
    flags.join(", ")
}

/// A single plain-text list line for an entry (the piped, script-friendly form).
fn list_line(repo: &Repo) -> String {
    let slug = repo
        .slug
        .as_deref()
        .map(|s| format!("[{s}] "))
        .unwrap_or_default();
    let flags = entry_flags(repo);
    let flags = if flags.is_empty() {
        String::new()
    } else {
        format!(" ({flags})")
    };
    format!(
        "{slug}{}\t{}\t{}{flags}",
        repo.display_name(),
        repo.kind.as_config_value(),
        repo.path.display(),
    )
}

/// Builds a sparcli theme from the config palette and installs it globally, so
/// CLI output shares the TUI's colors. `NO_COLOR` and non-terminal output are
/// handled by sparcli itself.
fn apply_sparcli_theme(config: &Config) {
    let palette = config.palette();
    let mut theme = Theme {
        accent: map_color(palette.accent),
        unicode: matches!(config.appearance.glyphs, GlyphVariant::Unicode),
        ..Theme::default()
    };
    theme.success = SpStyle::new().fg(map_color(palette.success));
    theme.error = SpStyle::new().fg(map_color(palette.error));
    theme.warning = SpStyle::new().fg(map_color(palette.warning));
    theme.info = SpStyle::new().fg(map_color(palette.info));
    theme.secondary = SpStyle::new().fg(map_color(palette.foreground_dim));
    set_theme(theme);
}

/// Maps a resolved palette [`Color`] to sparcli's color (truecolor, else reset).
fn map_color(color: Color) -> UiColor {
    match color.rgb() {
        Some((red, green, blue)) => UiColor::Rgb(red, green, blue),
        None => UiColor::Reset,
    }
}

/// Handles `hop add`: registers an entry for a path (default the current
/// directory), auto-detecting the kind unless `--kind` is given.
fn cmd_add(
    mut service: RepoService,
    path: Option<PathBuf>,
    slug: &Option<String>,
    section: &Option<String>,
    name: &Option<String>,
    kind: &Option<String>,
) -> ExitCode {
    let raw = path.unwrap_or_else(|| PathBuf::from("."));
    let expanded = paths::expand_tilde(&raw.to_string_lossy());
    let absolute = std::path::absolute(&expanded).unwrap_or(expanded);
    let kind = resolve_kind(kind.as_deref(), absolute.join(".git").exists());
    let mut repo = Repo::new(absolute.clone());
    repo.name = name.clone();
    repo.slug = slug.clone();
    repo.section = section.clone();
    repo.kind = kind;
    if let Err(error) = service.add(repo) {
        return output::report_error(&error);
    }
    if let Some(section) = section {
        let _ = service.ensure_section(section);
    }
    let _ = Alert::success(format!(
        "Added {} ({})",
        absolute.display(),
        kind.as_config_value()
    ))
    .print();
    ExitCode::SUCCESS
}

/// The entry kind for `hop add`: an explicit `--kind` value, else auto-detected
/// (`Git` when the path holds a `.git`, otherwise `Path`).
fn resolve_kind(explicit: Option<&str>, has_git_dir: bool) -> RepoKind {
    match explicit {
        Some(value) => RepoKind::from_config_value(value),
        None if has_git_dir => RepoKind::Git,
        None => RepoKind::Path,
    }
}

/// Handles `hop scan`: discovers git repos under a directory and imports the
/// ones chosen in the picker (or all found, with `--dry-run` printing only).
/// The `hop scan` options, grouped so the handler keeps a short parameter list.
struct ScanRequest {
    /// Directory to search (default: the working directory).
    dir: Option<PathBuf>,
    /// How deep to descend, if limited.
    depth: Option<usize>,
    /// Whether to keep descending into a work tree already found.
    nested: bool,
    /// Only report what would be imported.
    dry_run: bool,
}

fn cmd_scan(mut service: RepoService, request: ScanRequest) -> ExitCode {
    let raw = request.dir.unwrap_or_else(|| PathBuf::from("."));
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
        let _ =
            Alert::info(format!("No new git repos under {}.", root.display()))
                .print();
        return ExitCode::SUCCESS;
    }
    if request.dry_run {
        println!("{} new git repo(s) under {}:", new.len(), root.display());
        for path in &new {
            println!("  {}", path.display());
        }
        if !duplicates.is_empty() {
            println!("({} already in hop)", duplicates.len());
        }
        return ExitCode::SUCCESS;
    }

    if !duplicates.is_empty() {
        let _ = Alert::info(format!("{} already in hop.", duplicates.len()))
            .print();
    }
    let chosen = match pick_repos_to_import(&new, &root) {
        Ok(Some(chosen)) => chosen,
        Ok(None) => {
            let _ = Alert::info("Cancelled.").print();
            return ExitCode::SUCCESS;
        }
        Err(SparcliError::NoTerminal) => {
            return output::fail(
                "`hop scan` needs a terminal to choose the repos; \
                 pass --dry-run to only list them",
            );
        }
        Err(error) => return output::fail(&format!("terminal: {error}")),
    };
    if chosen.is_empty() {
        let _ = Alert::info("Nothing selected.").print();
        return ExitCode::SUCCESS;
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
    if let Err(error) = service.add_many(repos) {
        return output::report_error(&error);
    }
    let _ = Alert::success(format!("Added {count} git repo(s).")).print();
    ExitCode::SUCCESS
}

/// Asks which of the discovered `found` repos to import, all pre-checked.
/// Returns `None` when the user cancels.
///
/// # Errors
/// Returns [`SparcliError::NoTerminal`] off a terminal, or an I/O error when the
/// prompt cannot run.
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

/// Handles `hop doctor`: reports entry problems and exits non-zero when any are
/// found.
fn cmd_doctor(service: &RepoService) -> ExitCode {
    let issues = doctor::diagnose(
        service.repos(),
        |path| path.exists(),
        |path| path.join(".git").exists(),
    );
    if issues.is_empty() {
        let _ = Alert::success("hop doctor: no issues.").print();
        return ExitCode::SUCCESS;
    }
    let _ = Alert::warning(format!("hop doctor: {} issue(s)", issues.len()))
        .print();
    for issue in &issues {
        println!("  {issue}");
    }
    ExitCode::FAILURE
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

/// Handles `hop <slug>`: records the open and writes the handoff path. It only
/// `cd`s (it does not launch the git tool or open files), mirroring the TUI's
/// Enter.
fn cmd_jump(
    config: &Config,
    mut service: RepoService,
    args: &[String],
    cli: &Cli,
) -> ExitCode {
    // `--save-only`/`-s` is now redundant (jumping only `cd`s anyway); it is
    // still accepted and stripped so an old `hop <slug> -s` keeps working.
    let (slug, _save_only) = parse_jump_args(args, cli.save_only);
    let Some(slug) = slug else {
        return output::fail("no slug given");
    };
    let Some(index) = service.index_by_slug(&slug) else {
        return output::fail(&format!(
            "unknown command or slug '{slug}' (see `hop --help`)"
        ));
    };
    let Some(repo) = service.get(index).cloned() else {
        return output::fail("entry vanished");
    };
    if let Err(error) = service.mark_used(index) {
        return output::report_error(&error);
    }
    // `--fetch`: update the repo's remote refs so the next status is fresh.
    if cli.fetch && repo.kind == RepoKind::Git {
        SubprocessGitClient::new(config.github_username.clone())
            .fetch(&repo.path);
    }
    perform_jump(&service, &repo)
}

/// Writes the handoff `cd` path for a jump target (no tool/editor launch).
fn perform_jump(service: &RepoService, repo: &Repo) -> ExitCode {
    let target = jump_target(repo);
    if let Err(error) = service.write_selected(&target) {
        return output::report_error(&error);
    }
    ExitCode::SUCCESS
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

/// Extracts the slug and whether save-only was requested from the captured
/// external-subcommand arguments.
fn parse_jump_args(
    args: &[String],
    cli_save_only: bool,
) -> (Option<String>, bool) {
    let mut slug = None;
    let mut save_only = cli_save_only;
    for arg in args {
        if arg == "-s" || arg == "--save-only" {
            save_only = true;
        } else if slug.is_none() && !arg.starts_with('-') {
            slug = Some(arg.clone());
        }
    }
    (slug, save_only)
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;
    use crate::domain::slug;

    #[test]
    fn every_subcommand_name_is_a_reserved_slug() {
        // A slug must never shadow a real command, so each clap subcommand
        // (including the auto-generated `help`) must be in `slug::RESERVED`.
        for sub in Cli::command().get_subcommands() {
            let name = sub.get_name();
            assert!(
                slug::RESERVED.contains(&name),
                "subcommand '{name}' is not in slug::RESERVED"
            );
        }
    }

    fn entry(name: &str, kind: RepoKind, path: &str) -> Repo {
        let mut repo = Repo::new(PathBuf::from(path));
        repo.name = Some(name.to_string());
        repo.kind = kind;
        repo
    }

    #[test]
    fn parse_jump_args_extracts_slug_and_save_only() {
        assert_eq!(
            parse_jump_args(&["hop".to_string()], false),
            (Some("hop".to_string()), false)
        );
        assert_eq!(
            parse_jump_args(&["-s".to_string(), "hop".to_string()], false),
            (Some("hop".to_string()), true)
        );
        // The CLI flag forces save-only even without `-s` in the args.
        assert_eq!(
            parse_jump_args(&["hop".to_string()], true),
            (Some("hop".to_string()), true)
        );
        assert_eq!(parse_jump_args(&[], false), (None, false));
    }

    #[test]
    fn jump_target_uses_parent_only_for_files() {
        // A git entry jumps to its own path.
        let git = entry("hop", RepoKind::Git, "/code/hop");
        assert_eq!(jump_target(&git), PathBuf::from("/code/hop"));
        // A folder path (trailing slash) jumps to itself.
        let folder = entry("docs", RepoKind::Path, "/code/docs/");
        assert_eq!(jump_target(&folder), PathBuf::from("/code/docs/"));
        // A file path jumps to its parent directory.
        let file = entry("readme", RepoKind::Path, "/code/hop/README.md");
        assert_eq!(jump_target(&file), PathBuf::from("/code/hop"));
    }

    #[test]
    fn list_line_includes_slug_kind_and_flags() {
        let mut repo = entry("hop", RepoKind::Git, "/code/hop");
        repo.slug = Some("hp".to_string());
        repo.fav = true;
        let line = list_line(&repo);
        assert!(line.starts_with("[hp] hop\t"));
        assert!(line.contains("\tgit\t/code/hop"));
        assert!(line.ends_with("(fav)"));
    }

    #[test]
    fn partition_found_splits_new_from_known() {
        let found = vec![
            PathBuf::from("/code/a"),
            PathBuf::from("/code/b"),
            PathBuf::from("/code/c"),
        ];
        let known: HashSet<String> =
            ["/code/b".to_string()].into_iter().collect();
        // Identity canon so the test stays pure (no filesystem).
        let (new, dups) = partition_found(&found, &known, |p| {
            p.to_string_lossy().into_owned()
        });
        assert_eq!(
            new,
            vec![PathBuf::from("/code/a"), PathBuf::from("/code/c")]
        );
        assert_eq!(dups, vec![PathBuf::from("/code/b")]);
    }

    #[test]
    fn resolve_kind_honours_explicit_then_auto_detects() {
        assert_eq!(resolve_kind(Some("git"), false), RepoKind::Git);
        assert_eq!(resolve_kind(Some("path"), true), RepoKind::Path);
        assert_eq!(resolve_kind(Some("folder"), false), RepoKind::Path);
        assert_eq!(resolve_kind(None, true), RepoKind::Git);
        assert_eq!(resolve_kind(None, false), RepoKind::Path);
    }
}
