//! CLI layer: the `clap` parser and the composition root that dispatches to
//! one handler per command.
//!
//! A bare invocation opens the interactive TUI; `hop <slug>` jumps straight to
//! an entry. This module resolves the config path, wires the TOML repository
//! into the [`RepoService`] and the subprocess git client, then dispatches. The
//! handlers themselves live in [`list`], [`scan`] and [`jump`]; they return a
//! [`CliResult`] and never exit the process, so the translation into a message
//! and an exit code happens exactly once, in [`run`].

pub mod context;
mod demo;
mod jump;
mod list;
pub mod output;
mod scan;

use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;

use crate::cli::context::CliContext;
use crate::cli::output::{
    CliError, CliResult, EXIT_OK, Streams, apply_sparcli_theme, report,
};
use crate::cli::scan::{AddRequest, KindArg, ScanRequest};
use crate::config::Config;
use crate::config::loader::load_config;
use crate::domain::repo::RepoKind;
use crate::service::repo_service::RepoService;
use crate::storage::in_memory_repository::InMemoryRepoRepository;
use crate::storage::repository::RepoRepository;
use crate::storage::subprocess_git_client::SubprocessGitClient;
use crate::storage::toml_repo_repository::TomlRepoRepository;
use crate::tui::{self, App, RunOutcome, StartupStatus, Tui};
use crate::util::app_info::{APP_ABOUT, APP_NAME};
use crate::util::opener::{
    launch_git_tool, open_default_app, open_in_editor, resolve_editor,
};
use crate::util::paths;

/// Environment override for the config file path.
const CONFIG_ENV: &str = "HOP_CONFIG";

/// The examples block appended to the top-level help, so the two forms that are
/// not obvious from the command list - the bare TUI and the slug jump - are
/// discoverable without the README.
const EXAMPLES: &str = "\
Examples:
  hop                      Opens the interactive picker (the default)
  hop myrepo               Jumps to the entry with the slug 'myrepo'
  hop add ~/code/project   Registers a path, detecting whether it is a repo
  hop scan ~/code --yes    Imports every git repo found, without asking
  hop list --json | jq .   Prints the entries as machine-readable JSON

Exit codes:
  0  success
  1  runtime error
  2  usage error (unknown option, missing value, no terminal to prompt on)
  130  interrupted with Ctrl+C";

/// The hop command line.
#[derive(Debug, Parser)]
#[command(
    name = APP_NAME,
    version,
    about = APP_ABOUT,
    after_help = EXAMPLES,
)]
pub struct Cli {
    /// Use a specific config file (default: $XDG_CONFIG_HOME/hop/config.toml).
    #[arg(short = 'C', long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,
    /// Deprecated/no-op: `hop <slug>` only writes the path (`cd`) anyway.
    #[arg(short = 's', long, global = true, hide = true)]
    save_only: bool,
    /// Run `git fetch` before gathering status (overrides `fetch_on_start`).
    #[arg(long, global = true, overrides_with = "no_fetch")]
    fetch: bool,
    /// Skip `git fetch` even when `fetch_on_start` is enabled.
    #[arg(long, global = true)]
    no_fetch: bool,
    /// Use cached status only; do not run git.
    #[arg(long, global = true)]
    cached: bool,
    /// Launch with built-in demo data for screenshots (no real git/config).
    #[arg(long, global = true)]
    demo: bool,
    /// Print more progress to stderr; repeat for more detail.
    #[arg(short = 'v', long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,
    /// Suppress everything except errors.
    #[arg(short = 'q', long, global = true, conflicts_with = "verbose")]
    quiet: bool,
    /// Disable colored output (as does setting NO_COLOR).
    #[arg(long, global = true)]
    no_color: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

impl Cli {
    /// How verbose the log should be, from `-v`/`-q`.
    ///
    /// Diagnostics go to the log file, never to stdout, so raising this never
    /// changes what a caller reads from the payload stream.
    pub fn log_level(&self) -> log::LevelFilter {
        if self.quiet {
            return log::LevelFilter::Error;
        }
        match self.verbose {
            0 => log::LevelFilter::Info,
            1 => log::LevelFilter::Debug,
            _ => log::LevelFilter::Trace,
        }
    }

    /// Whether color must be suppressed for this run.
    pub fn is_color_disabled(&self) -> bool {
        self.no_color
    }
}

/// The available commands; absence opens the interactive TUI.
#[derive(Debug, Subcommand)]
enum Command {
    /// Lists entries as plain text, a table, or JSON.
    List {
        /// Print the entries as a JSON document instead.
        #[arg(long)]
        json: bool,
    },
    /// Prints the resolved config file path.
    #[command(name = "show-config", alias = "config-path")]
    ShowConfig,
    /// Adds an entry for a path (default: the current directory).
    Add {
        /// Path to add (default: `.`).
        path: Option<PathBuf>,
        /// Slug for `hop <slug>`.
        #[arg(long)]
        slug: Option<String>,
        /// Section to group the entry under.
        #[arg(long)]
        section: Option<String>,
        /// Display name (default: the path's basename).
        #[arg(long)]
        name: Option<String>,
        /// Entry kind (default: auto-detect).
        #[arg(long, value_enum)]
        kind: Option<KindArg>,
    },
    /// Scans a directory for git repos and imports the chosen ones.
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
        /// Import everything found without asking (also the way to run this
        /// without a terminal).
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// Reports problems with the entries (missing paths, bad/duplicate slugs).
    Doctor,
    /// Prints a shell completion script.
    Completions {
        /// The shell to generate for.
        #[arg(value_enum)]
        shell: Shell,
    },
    /// Prints the manpage in roff format.
    Man,
    /// Jumps to a slug (`hop <slug>`).
    #[command(external_subcommand)]
    Jump(Vec<String>),
}

/// Parses arguments and runs the requested command.
///
/// This is the single place a failure becomes a message and an exit code; the
/// handlers only ever return a [`CliError`].
#[must_use]
pub fn run(cli: Cli) -> ExitCode {
    let stdout = io::stdout();
    let stderr = io::stderr();
    let mut out = stdout.lock();
    let mut err = stderr.lock();
    let mut streams = Streams {
        out: &mut out,
        err: &mut err,
    };
    match dispatch(&cli, &mut streams) {
        Ok(Outcome::Code(code)) => ExitCode::from(code),
        Err(error) => report(&error, &mut io::stderr().lock()),
    }
}

/// What a dispatched run ended with, so the TUI can hand back its own code.
enum Outcome {
    /// Finished; end the process with this code.
    Code(u8),
}

/// Resolves the config, builds the service and runs the requested command.
fn dispatch(cli: &Cli, streams: &mut Streams) -> Result<Outcome, CliError> {
    let config_path = resolve_config_path(cli);
    let ctx = CliContext::detect();

    // These two need neither the config nor the service.
    match &cli.command {
        Some(Command::ShowConfig) => {
            streams.line(&config_path.display().to_string())?;
            return Ok(Outcome::Code(EXIT_OK));
        }
        Some(Command::Completions { shell }) => {
            let mut command = Cli::command();
            clap_complete::generate(
                *shell,
                &mut command,
                APP_NAME,
                &mut streams.out,
            );
            return Ok(Outcome::Code(EXIT_OK));
        }
        Some(Command::Man) => {
            clap_mangen::Man::new(Cli::command())
                .render(&mut streams.out)
                .map_err(CliError::from)?;
            return Ok(Outcome::Code(EXIT_OK));
        }
        _ => {}
    }

    let config = load_config(&config_path)?;
    // Style sparcli's CLI output from the same config palette as the TUI.
    apply_sparcli_theme(&config);
    if cli.command.is_none() && cli.demo {
        return run_demo(config).map(Outcome::Code);
    }
    let mut service = build_service(&config_path)?;

    match &cli.command {
        None => {
            let startup = startup_status(cli, &config);
            run_tui(config, service, startup).map(Outcome::Code)
        }
        Some(Command::List { json }) => {
            list::run(&service, ctx, *json, streams)?;
            Ok(Outcome::Code(EXIT_OK))
        }
        Some(Command::Jump(args)) => {
            jump::run(&config, &mut service, args, cli.fetch)?;
            Ok(Outcome::Code(EXIT_OK))
        }
        Some(Command::Add {
            path,
            slug,
            section,
            name,
            kind,
        }) => {
            scan::add(
                &mut service,
                AddRequest {
                    path: path.clone(),
                    slug: slug.clone(),
                    section: section.clone(),
                    name: name.clone(),
                    kind: *kind,
                },
                streams,
            )?;
            Ok(Outcome::Code(EXIT_OK))
        }
        Some(Command::Scan {
            dir,
            depth,
            nested,
            dry_run,
            yes,
        }) => {
            scan::scan(
                &mut service,
                ScanRequest {
                    dir: dir.clone(),
                    depth: *depth,
                    nested: *nested,
                    dry_run: *dry_run,
                    assume_yes: *yes,
                },
                ctx,
                streams,
            )?;
            Ok(Outcome::Code(EXIT_OK))
        }
        Some(Command::Doctor) => {
            scan::run_doctor(&service, streams)?;
            Ok(Outcome::Code(EXIT_OK))
        }
        // Handled above, before the config is loaded.
        Some(
            Command::ShowConfig | Command::Completions { .. } | Command::Man,
        ) => Ok(Outcome::Code(EXIT_OK)),
    }
}

/// Derives the TUI's startup status from the flags and config.
///
/// `--cached` shows only the cache. Otherwise a refresh runs, fetching first
/// when `--fetch` is set or `fetch_on_start` is enabled - unless `--no-fetch`
/// overrides the config, so the command line always wins.
fn startup_status(cli: &Cli, config: &Config) -> StartupStatus {
    if cli.cached {
        return StartupStatus::Cached;
    }
    let fetch = if cli.no_fetch {
        false
    } else {
        cli.fetch || config.fetch_on_start
    };
    StartupStatus::Refresh { fetch }
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
fn build_service(config_path: &Path) -> Result<RepoService, CliError> {
    let repository = TomlRepoRepository::new(config_path.to_path_buf());
    RepoService::new(
        Box::new(repository),
        paths::usage_file(),
        paths::selected_repo_file(),
    )
    .map_err(CliError::from)
}

/// Opens the interactive TUI and performs the chosen post-exit action.
fn run_tui(
    config: Config,
    service: RepoService,
    startup: StartupStatus,
) -> Result<u8, CliError> {
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
fn run_demo(mut config: Config) -> Result<u8, CliError> {
    config.example_mode = true;
    let dir = std::env::temp_dir().join("hop-demo");
    let repository = InMemoryRepoRepository::new(demo::repos());
    repository.save_sections(RepoKind::Git, &demo::git_sections())?;
    repository.save_sections(RepoKind::Path, &demo::sections())?;
    let service = RepoService::new(
        Box::new(repository),
        dir.join("usage.toml"),
        dir.join("selected-repo.txt"),
    )?;
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
fn run_app(config: &Config, app: App) -> Result<u8, CliError> {
    let mut tui = Tui::new()
        .map_err(|error| CliError::runtime(format!("terminal: {error}")))?;
    let outcome = tui::run(app, &mut tui);
    drop(tui);
    let outcome = outcome
        .map_err(|error| CliError::runtime(format!("terminal: {error}")))?;
    perform_outcome(config, outcome)
}

/// Acts on the TUI outcome after the terminal is restored.
fn perform_outcome(
    config: &Config,
    outcome: RunOutcome,
) -> Result<u8, CliError> {
    match outcome {
        RunOutcome::Quit | RunOutcome::Jumped => Ok(EXIT_OK),
        RunOutcome::LaunchGitTool(path) => {
            launch_tool(config, &path)?;
            Ok(EXIT_OK)
        }
        // The run loop handles the inline overlay and never returns it here.
        RunOutcome::LaunchGitToolInline(_) => Ok(EXIT_OK),
        RunOutcome::OpenFile(path) => {
            let editor = resolve_editor(config.editor.as_deref());
            open_in_editor(&editor, &path)?;
            Ok(EXIT_OK)
        }
        RunOutcome::OpenWith(path) => {
            open_default_app(&path)?;
            Ok(EXIT_OK)
        }
    }
}

/// Launches the configured git tool in `dir`.
///
/// # Errors
///
/// Returns an error when the tool is configured but cannot be started, so the
/// run does not report success after failing to open anything.
fn launch_tool(config: &Config, dir: &Path) -> CliResult {
    let Some(program) = &config.git_program else {
        return Ok(());
    };
    launch_git_tool(program, dir).map(|_| ()).map_err(|error| {
        CliError::runtime(format!("could not launch {program}: {error}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::slug::RESERVED;

    #[test]
    fn the_parser_definition_is_internally_consistent() {
        // Catches conflicting flags, duplicate shorts and bad defaults at test
        // time rather than on the user's first run.
        Cli::command().debug_assert();
    }

    #[test]
    fn every_subcommand_name_is_a_reserved_slug() {
        // A slug that shadows a command would make that command unreachable.
        for command in Cli::command().get_subcommands() {
            let name = command.get_name();
            if name == "help" {
                continue;
            }
            assert!(
                RESERVED.contains(&name),
                "subcommand '{name}' is missing from domain::slug::RESERVED"
            );
        }
    }

    #[test]
    fn verbosity_flags_pick_the_log_level() {
        let quiet = Cli::parse_from(["hop", "--quiet"]);
        assert_eq!(quiet.log_level(), log::LevelFilter::Error);
        let plain = Cli::parse_from(["hop"]);
        assert_eq!(plain.log_level(), log::LevelFilter::Info);
        let verbose = Cli::parse_from(["hop", "-v"]);
        assert_eq!(verbose.log_level(), log::LevelFilter::Debug);
        let louder = Cli::parse_from(["hop", "-vv"]);
        assert_eq!(louder.log_level(), log::LevelFilter::Trace);
    }

    #[test]
    fn the_command_line_overrides_fetch_on_start_in_both_directions() {
        // The precedence rule: an option always beats the config file.
        let mut config = Config {
            fetch_on_start: true,
            ..Config::default()
        };
        let off = Cli::parse_from(["hop", "--no-fetch"]);
        assert!(matches!(
            startup_status(&off, &config),
            StartupStatus::Refresh { fetch: false }
        ));

        config.fetch_on_start = false;
        let on = Cli::parse_from(["hop", "--fetch"]);
        assert!(matches!(
            startup_status(&on, &config),
            StartupStatus::Refresh { fetch: true }
        ));
    }

    #[test]
    fn the_legacy_config_path_name_still_resolves() {
        // Renaming a published command must not break existing scripts.
        let cli = Cli::parse_from(["hop", "config-path"]);
        assert!(matches!(cli.command, Some(Command::ShowConfig)));
    }

    #[test]
    fn an_unknown_kind_is_rejected_rather_than_silently_becoming_a_path() {
        assert!(
            Cli::try_parse_from(["hop", "add", "--kind", "gti", "."]).is_err()
        );
        let ok = Cli::parse_from(["hop", "add", "--kind", "git", "."]);
        assert!(matches!(
            ok.command,
            Some(Command::Add {
                kind: Some(KindArg::Git),
                ..
            })
        ));
    }
}
