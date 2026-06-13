//! CLI layer: the `clap` parser and one handler per command.
//!
//! A bare invocation opens the interactive TUI; `hop <slug>` jumps straight to
//! an entry; `list`, `import` and `config-path` are utility commands. This is
//! the composition root for commands: it resolves the config path, wires the
//! TOML repository into the [`RepoService`] and the subprocess git client, then
//! dispatches.

mod output;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, Subcommand};

use crate::config::loader::load_config;
use crate::config::{Config, migrate};
use crate::domain::repo::{self, PathClass, Repo, RepoKind, is_dir_target};
use crate::service::repo_service::RepoService;
use crate::storage::git_client::GitClient;
use crate::storage::subprocess_git_client::SubprocessGitClient;
use crate::storage::toml_repo_repository::TomlRepoRepository;
use crate::tui::{self, App, RunOutcome, StartupStatus, Tui};
use crate::util::app_info::{APP_ABOUT, APP_NAME, APP_VERSION};
use crate::util::opener::{
    launch_git_tool, open_default_app, open_in_editor, resolve_editor,
};
use crate::util::paths;

/// Environment override for the config file path.
const CONFIG_ENV: &str = "HOP_CONFIG";

/// The hop command line.
#[derive(Debug, Parser)]
#[command(name = APP_NAME, version = APP_VERSION, about = APP_ABOUT)]
pub struct Cli {
    /// Use a specific config file (default: $XDG_CONFIG_HOME/hop/config.toml).
    #[arg(short = 'C', long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,
    /// Jump only: write the path without launching the git tool.
    #[arg(short = 's', long, global = true)]
    save_only: bool,
    /// Run `git fetch` before gathering status.
    #[arg(long, global = true)]
    fetch: bool,
    /// Use cached status only; do not run git.
    #[arg(long, global = true)]
    cached: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

/// The available commands; absence opens the interactive TUI.
#[derive(Debug, Subcommand)]
enum Command {
    /// List entries as plain text (for scripting).
    List,
    /// Import a git-repo-jumper config.yaml into hop's config.toml.
    Import {
        /// Source YAML (default: $XDG_CONFIG_HOME/hop/config.yaml).
        #[arg(long, value_name = "PATH")]
        from: Option<PathBuf>,
    },
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
        Some(Command::Import { from }) => {
            cmd_import(from.clone(), &config_path)
        }
        _ => run_with_service(cli, config_path),
    }
}

/// Builds the service and dispatches the commands that need it.
fn run_with_service(cli: Cli, config_path: PathBuf) -> ExitCode {
    auto_migrate_if_needed(&cli, &config_path);
    let config = match load_config(&config_path) {
        Ok(config) => config,
        Err(error) => return output::report_error(&error),
    };
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
        // ConfigPath and Import are handled before the service is built.
        Some(Command::ConfigPath | Command::Import { .. }) => ExitCode::SUCCESS,
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

/// Imports the default legacy YAML into the default config on first start.
fn auto_migrate_if_needed(cli: &Cli, config_path: &Path) {
    // Only for the default path, never a `--config` override.
    if cli.config.is_some() || config_path.exists() {
        return;
    }
    let legacy = paths::legacy_config_file();
    if !legacy.exists() {
        return;
    }
    match migrate::migrate_file(&legacy, config_path) {
        Ok(()) => eprintln!(
            "{APP_NAME}: imported {} into {}",
            legacy.display(),
            config_path.display()
        ),
        Err(error) => eprintln!("{APP_NAME}: import skipped: {error}"),
    }
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
    let mut tui = match Tui::new() {
        Ok(tui) => tui,
        Err(error) => return output::fail(&format!("terminal: {error}")),
    };
    let outcome = tui::run(app, &mut tui);
    drop(tui);
    match outcome {
        Ok(outcome) => perform_outcome(&config, outcome),
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
        println!("No entries yet. Run hop to add some, or hop import.");
        return ExitCode::SUCCESS;
    }
    for repo in repos {
        println!("{}", list_line(repo));
    }
    // `list` prints stored fields only (no git status), so the status flags do
    // not apply here; they affect the TUI and `hop <slug> --fetch`.
    let _ = (cli.fetch, cli.cached);
    ExitCode::SUCCESS
}

/// A single plain-text list line for an entry.
fn list_line(repo: &Repo) -> String {
    let slug = repo
        .slug
        .as_deref()
        .map(|s| format!("[{s}] "))
        .unwrap_or_default();
    let mut flags = Vec::new();
    if repo.fav {
        flags.push("fav");
    }
    if repo.archived {
        flags.push("archived");
    }
    let flags = if flags.is_empty() {
        String::new()
    } else {
        format!(" ({})", flags.join(", "))
    };
    format!(
        "{slug}{}\t{}\t{}{flags}",
        repo.display_name(),
        repo.kind.as_config_value(),
        repo.path.display(),
    )
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
    println!("Added {} ({})", absolute.display(), kind.as_config_value());
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

/// Handles `hop <slug>`: records the open, writes the handoff path and launches
/// the git tool (unless `--save-only`).
fn cmd_jump(
    config: &Config,
    mut service: RepoService,
    args: &[String],
    cli: &Cli,
) -> ExitCode {
    let (slug, save_only) = parse_jump_args(args, cli.save_only);
    let Some(slug) = slug else {
        return output::fail("no slug given");
    };
    let Some(index) = service.index_by_slug(&slug) else {
        return output::fail(&format!("unknown slug '{slug}'"));
    };
    let Some(repo) = service.get(index).cloned() else {
        return output::fail("entry vanished");
    };
    if let Err(error) = service.mark_used(index) {
        return output::report_error(&error);
    }
    // `--fetch`: update the repo's remote refs before launching the tool.
    if cli.fetch && repo.kind == RepoKind::Git {
        SubprocessGitClient::new(config.github_username.clone())
            .fetch(&repo.path);
    }
    perform_jump(config, &service, &repo, save_only)
}

/// Writes the handoff path and launches the tool/editor for a jump target.
fn perform_jump(
    config: &Config,
    service: &RepoService,
    repo: &Repo,
    save_only: bool,
) -> ExitCode {
    let target = jump_target(repo);
    if let Err(error) = service.write_selected(&target) {
        return output::report_error(&error);
    }
    if save_only {
        return ExitCode::SUCCESS;
    }
    match repo.kind {
        RepoKind::Git => launch_tool(config, &repo.path),
        RepoKind::Path => open_path_target(config, repo),
    }
    ExitCode::SUCCESS
}

/// Opens a file/folder jump target: a folder needs nothing further (the path is
/// already written), a text file opens in the editor, any other file in the
/// default application.
fn open_path_target(config: &Config, repo: &Repo) {
    match repo::classify_path(&repo.path, &config.editor_extensions) {
        PathClass::Folder => {}
        PathClass::TextFile => {
            let editor = resolve_editor(config.editor.as_deref());
            let _ = open_in_editor(&editor, &repo.path);
        }
        PathClass::OtherFile => {
            let _ = open_default_app(&repo.path);
        }
    }
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

/// Handles `hop import`, converting a git-repo-jumper YAML to hop's TOML.
fn cmd_import(from: Option<PathBuf>, config_path: &Path) -> ExitCode {
    let from = from.unwrap_or_else(paths::legacy_config_file);
    if !from.exists() {
        return output::fail(&format!("no file at {}", from.display()));
    }
    match migrate::migrate_file(&from, config_path) {
        Ok(()) => {
            println!(
                "Imported {} into {}",
                from.display(),
                config_path.display()
            );
            ExitCode::SUCCESS
        }
        Err(error) => output::report_error(&error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn resolve_kind_honours_explicit_then_auto_detects() {
        assert_eq!(resolve_kind(Some("git"), false), RepoKind::Git);
        assert_eq!(resolve_kind(Some("path"), true), RepoKind::Path);
        assert_eq!(resolve_kind(Some("folder"), false), RepoKind::Path);
        assert_eq!(resolve_kind(None, true), RepoKind::Git);
        assert_eq!(resolve_kind(None, false), RepoKind::Path);
    }
}
