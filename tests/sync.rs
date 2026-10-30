//! Drives `hop pull` and `hop push` against real throwaway repositories.
//!
//! The decision logic is unit-tested on fakes; what only a real `git` can
//! prove is the rest: that the commands run non-interactively, that their
//! output is read correctly ("Already up to date", a rejected push, a pull
//! that cannot fast-forward), and that a dirty clone is left alone.
//!
//! Each test builds a bare "remote" plus two clones in its own temp
//! directory. git runs with the system and global config switched off, so a
//! developer's `pull.rebase` or `commit.gpgsign` cannot change the outcome.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The compiled binary under test, as cargo built it for this run.
const BIN: &str = env!("CARGO_BIN_EXE_hop");

/// A remote with two clones, `a` (the colleague) and `b` (the entry in hop).
struct Fixture {
    root: PathBuf,
    a: PathBuf,
    b: PathBuf,
    config: PathBuf,
}

impl Fixture {
    /// Sets up the remote with one commit, cloned twice, and a hop config
    /// whose only entry is clone `b` under the slug `b`.
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir()
            .join(format!("hop-sync-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temp dir is creatable");
        let remote = root.join("remote.git");
        git(
            &root,
            &["init", "--quiet", "--bare", "-b", "main", "remote.git"],
        );
        let a = clone(&root, &remote, "a");
        commit(&a, "first");
        git(&a, &["push", "--quiet", "-u", "origin", "main"]);
        let b = clone(&root, &remote, "b");
        let config = root.join("config.toml");
        let body = format!(
            "[[repos]]\npath = \"{}\"\nkind = \"git\"\nslug = \"b\"\n",
            b.display()
        );
        fs::write(&config, body).expect("config is writable");
        Fixture { root, a, b, config }
    }

    /// Runs `hop <args>` against the fixture's config.
    fn hop(&self, args: &[&str]) -> (i32, String, String) {
        let output = isolated(Command::new(BIN))
            .arg("-C")
            .arg(&self.config)
            .args(args)
            .env("SPARCLI_NO_TTY", "1")
            .env("NO_COLOR", "1")
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env_remove("HOP_CONFIG")
            .output()
            .expect("the hop binary runs");
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// `command` with the developer's git configuration shut out.
fn isolated(mut command: Command) -> Command {
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com");
    command
}

/// Runs `git <args>` in `dir`, failing the test when git fails.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = isolated(Command::new("git"))
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Clones `remote` into `root/name`.
fn clone(root: &Path, remote: &Path, name: &str) -> PathBuf {
    let remote = remote.to_str().expect("temp paths are UTF-8");
    git(root, &["clone", "--quiet", remote, name]);
    root.join(name)
}

/// Commits a new file named after `message` in `repo`.
fn commit(repo: &Path, message: &str) {
    fs::write(repo.join(format!("{message}.txt")), message)
        .expect("work tree is writable");
    git(repo, &["add", "."]);
    git(repo, &["commit", "--quiet", "-m", message]);
}

/// The commit `repo` has checked out.
fn head(repo: &Path) -> String {
    git(repo, &["rev-parse", "HEAD"])
}

#[test]
fn pull_fast_forwards_a_clean_clone() {
    let fixture = Fixture::new("pull");
    commit(&fixture.a, "second");
    git(&fixture.a, &["push", "--quiet"]);

    let (code, out, err) = fixture.hop(&["pull", "b"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.is_empty());
    assert!(err.contains("pulled"), "{err}");
    assert_eq!(head(&fixture.b), head(&fixture.a));

    let (code, _, err) = fixture.hop(&["pull", "b"]);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("up to date"), "{err}");
}

#[test]
fn pull_leaves_a_dirty_clone_alone() {
    let fixture = Fixture::new("dirty");
    commit(&fixture.a, "second");
    git(&fixture.a, &["push", "--quiet"]);
    let before = head(&fixture.b);
    fs::write(fixture.b.join("scratch.txt"), "wip").expect("writable");

    let (code, _, err) = fixture.hop(&["pull", "b"]);
    assert_eq!(code, 0, "a skip is not a failure: {err}");
    assert!(err.contains("skipped: uncommitted changes"), "{err}");
    assert_eq!(head(&fixture.b), before, "the clone was not touched");
}

#[test]
fn push_publishes_commits_ahead() {
    let fixture = Fixture::new("push");
    commit(&fixture.b, "local");

    let (code, _, err) = fixture.hop(&["push", "b"]);
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("pushed"), "{err}");
    git(&fixture.a, &["pull", "--quiet", "--ff-only"]);
    assert_eq!(head(&fixture.a), head(&fixture.b));
}

#[test]
fn a_diverged_clone_fails_both_ways_with_a_reason() {
    let fixture = Fixture::new("diverged");
    commit(&fixture.a, "theirs");
    git(&fixture.a, &["push", "--quiet"]);
    commit(&fixture.b, "ours");
    // Learn about the remote commit, so b counts as ahead and behind.
    git(&fixture.b, &["fetch", "--quiet"]);

    let (code, _, err) = fixture.hop(&["push", "b"]);
    assert_eq!(code, 1, "a rejected push fails the run: {err}");
    assert!(err.contains("rejected: remote has newer commits"), "{err}");

    let (code, _, err) = fixture.hop(&["pull", "b"]);
    assert_eq!(code, 1, "a pull that cannot fast-forward fails: {err}");
    assert!(err.contains("diverged from upstream"), "{err}");
}
