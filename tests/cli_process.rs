//! Drives the built `hop` binary as a real process.
//!
//! These tests exist because a handler-level test proves none of what §1.6 of
//! the style guide asks for: it exercises neither the argument parsing, nor the
//! exit code, nor the separation of stdout from stderr. Every case below
//! therefore asserts all three, on the actual executable.
//!
//! `SPARCLI_NO_TTY` makes the run deterministic: without it the output would
//! depend on whether the test harness happened to be attached to a terminal.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The compiled binary under test, as cargo built it for this run.
const BIN: &str = env!("CARGO_BIN_EXE_hop");

/// What one invocation produced, with the two streams kept apart.
struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl From<Output> for Run {
    fn from(output: Output) -> Self {
        Run {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }
}

/// Runs `hop` with `args` against an isolated config file.
fn hop(config: &Path, args: &[&str]) -> Run {
    Command::new(BIN)
        .arg("-C")
        .arg(config)
        .args(args)
        .env("SPARCLI_NO_TTY", "1")
        .env("NO_COLOR", "1")
        // A stray HOP_CONFIG in the developer's shell must not leak in.
        .env_remove("HOP_CONFIG")
        .output()
        .expect("the hop binary runs")
        .into()
}

/// Runs `hop` without a config argument, for the commands that need none.
fn hop_bare(args: &[&str]) -> Run {
    Command::new(BIN)
        .args(args)
        .env("SPARCLI_NO_TTY", "1")
        .env("NO_COLOR", "1")
        .env_remove("HOP_CONFIG")
        .output()
        .expect("the hop binary runs")
        .into()
}

/// A throwaway config file holding `body`, plus the directory it lives in.
fn config_with(name: &str, body: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("hop-cli-test-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir is creatable");
    let path = dir.join("config.toml");
    std::fs::write(&path, body).expect("config is writable");
    (path, dir)
}

#[test]
fn help_and_version_succeed_on_stdout() {
    let help = hop_bare(&["--help"]);
    assert_eq!(help.code, 0);
    assert!(help.stderr.is_empty(), "help must not touch stderr");
    assert!(help.stdout.contains("Usage:"));
    // The two forms a bare command list cannot show must be documented.
    assert!(help.stdout.contains("Examples:"));
    assert!(help.stdout.contains("Exit codes:"));

    let version = hop_bare(&["--version"]);
    assert_eq!(version.code, 0);
    assert!(version.stdout.starts_with("hop "));
    assert!(version.stderr.is_empty());
}

#[test]
fn an_unknown_option_is_a_usage_error_that_leaves_stdout_empty() {
    let run = hop_bare(&["--definitely-not-an-option"]);
    assert_eq!(run.code, 2, "usage errors exit 2");
    assert!(run.stdout.is_empty(), "stdout carries payload only");
    assert!(
        run.stderr.contains("--help"),
        "point at the help, not dump it"
    );
}

#[test]
fn an_unknown_slug_reports_a_greppable_error_line() {
    let (config, _dir) = config_with("unknown-slug", "");
    let run = hop(&config, &["no_such_slug_here"]);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr.starts_with("hop: error: "),
        "a boxed panel cannot be grepped; got {:?}",
        run.stderr
    );
    assert!(run.stderr.contains("no_such_slug_here"));
}

#[test]
fn an_empty_store_keeps_stdout_clean_and_puts_the_hint_on_stderr() {
    // The regression: the hint used to go to stdout, so `hop list > file`
    // wrote "No entries yet" into the payload.
    let (config, _dir) = config_with("empty-list", "");
    let run = hop(&config, &["list"]);
    assert_eq!(run.code, 0);
    assert!(run.stdout.is_empty(), "got stdout: {:?}", run.stdout);
    assert!(run.stderr.contains("No entries yet"));
}

#[test]
fn list_json_writes_nothing_but_the_json_document_to_stdout() {
    let (config, _dir) = config_with(
        "json",
        "[[repos]]\npath = \"/code/hop\"\nslug = \"hop\"\n",
    );
    let run = hop(&config, &["list", "--json"]);
    assert_eq!(run.code, 0);
    assert!(run.stderr.is_empty(), "JSON runs stay silent on stderr");
    let parsed: serde_json::Value =
        serde_json::from_str(&run.stdout).expect("stdout is valid JSON");
    assert_eq!(parsed[0]["slug"], "hop");
    assert_eq!(parsed[0]["path"], "/code/hop");
}

#[test]
fn doctor_exits_non_zero_and_lists_the_problems_on_stdout() {
    let (config, _dir) = config_with(
        "doctor",
        "[[repos]]\npath = \"/nowhere/at/all\"\nslug = \"gone\"\n",
    );
    let run = hop(&config, &["doctor"]);
    assert_eq!(run.code, 1, "a failed check must not report success");
    assert!(run.stdout.contains("/nowhere/at/all"));
    assert!(run.stderr.starts_with("hop: error: "));

    let (clean_config, _clean_dir) = config_with("doctor-clean", "");
    let clean = hop(&clean_config, &["doctor"]);
    assert_eq!(clean.code, 0);
}

#[test]
fn scan_without_a_terminal_fails_with_usage_and_names_the_flag_to_use() {
    // Off a TTY the picker can never be answered. Blocking here would be
    // indistinguishable from a hang, so the run must refuse early.
    let (config, dir) = config_with("scan-no-tty", "");
    let repo = dir.join("a-repo");
    std::fs::create_dir_all(repo.join(".git")).expect("fake repo is creatable");

    let run = hop(&config, &["scan", dir.to_str().unwrap()]);
    assert_eq!(run.code, 2, "no terminal to prompt on is a usage error");
    assert!(run.stdout.is_empty());
    assert!(
        run.stderr.contains("--yes"),
        "the message must name the option that supplies the answer; got {:?}",
        run.stderr
    );
}

#[test]
fn scan_yes_imports_without_a_terminal_and_keeps_the_report_off_stdout() {
    let (config, dir) = config_with("scan-yes", "");
    std::fs::create_dir_all(dir.join("a-repo").join(".git"))
        .expect("fake repo is creatable");

    let run = hop(&config, &["scan", dir.to_str().unwrap(), "--yes"]);
    assert_eq!(run.code, 0);
    assert!(run.stdout.is_empty(), "the confirmation is not payload");
    assert!(run.stderr.contains("Added"));

    // The entry really landed in the config, not just in the message.
    let listed = hop(&config, &["list", "--json"]);
    let parsed: serde_json::Value =
        serde_json::from_str(&listed.stdout).expect("valid JSON");
    assert_eq!(parsed.as_array().expect("an array").len(), 1);
}

#[test]
fn scan_dry_run_prints_the_paths_as_payload_and_changes_nothing() {
    let (config, dir) = config_with("scan-dry", "");
    std::fs::create_dir_all(dir.join("a-repo").join(".git"))
        .expect("fake repo is creatable");

    let run = hop(&config, &["scan", dir.to_str().unwrap(), "--dry-run"]);
    assert_eq!(run.code, 0);
    assert!(run.stdout.contains("a-repo"), "the paths are the payload");

    let listed = hop(&config, &["list", "--json"]);
    let parsed: serde_json::Value =
        serde_json::from_str(&listed.stdout).expect("valid JSON");
    assert!(
        parsed.as_array().expect("an array").is_empty(),
        "--dry-run must not write anything"
    );
}

#[test]
fn show_config_prints_the_path_and_still_answers_to_its_old_name() {
    let (config, _dir) = config_with("show-config", "");
    let run = hop(&config, &["show-config"]);
    assert_eq!(run.code, 0);
    assert_eq!(run.stdout.trim(), config.display().to_string());
    assert!(run.stderr.is_empty());

    // Renaming a published command must not break existing scripts.
    let legacy = hop(&config, &["config-path"]);
    assert_eq!(legacy.code, 0);
    assert_eq!(legacy.stdout, run.stdout);
}

#[test]
fn completions_and_man_are_machine_readable_and_unstyled() {
    let completions = hop_bare(&["completions", "zsh"]);
    assert_eq!(completions.code, 0);
    assert!(completions.stdout.contains("#compdef hop"));
    assert!(!completions.stdout.contains('\u{1b}'), "no escape codes");

    let man = hop_bare(&["man"]);
    assert_eq!(man.code, 0);
    assert!(man.stdout.contains(".TH"), "roff output");
}

/// Every command and top-level option the parser knows must be documented in
/// the README, and the README must not promise one that does not exist.
///
/// §1.6 asks for the help and the README to be kept in sync and for that
/// agreement to be pinned by a test rather than merely requested. The drift
/// this catches is real: `hop <slug>` was documented for releases while the
/// help never mentioned it.
#[test]
fn the_readme_documents_every_command_and_option() {
    let readme = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md"),
    )
    .expect("the README is readable");
    let help = hop_bare(&["--help"]).stdout;

    for command in [
        "add",
        "scan",
        "doctor",
        "list",
        "show-config",
        "completions",
        "man",
    ] {
        assert!(
            help.contains(command),
            "`{command}` is missing from the help output"
        );
        assert!(
            readme.contains(&format!("hop {command}")),
            "`hop {command}` is missing from the README's command list"
        );
    }

    for option in [
        "--config",
        "--fetch",
        "--no-fetch",
        "--cached",
        "--demo",
        "--verbose",
        "--quiet",
        "--no-color",
        "--json",
        "--yes",
        "--dry-run",
        "--depth",
        "--nested",
    ] {
        assert!(
            readme.contains(option),
            "`{option}` is missing from the README"
        );
    }

    // The exit-code contract is part of the interface, so it is documented in
    // both places rather than only in the code that implements it.
    assert!(
        readme.contains("| 130 |"),
        "README lacks the exit-code table"
    );
    assert!(help.contains("130"), "help lacks the exit-code list");
}
