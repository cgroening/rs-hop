//! Launches external programs (the git tool, an editor) for a repo or file.
//!
//! Each launcher runs the child to completion with the terminal handed over, so
//! callers must restore the terminal (drop the `Tui` guard) before calling. The
//! command is built with explicit `.arg()` values (never a shell string) so
//! paths with spaces are safe.

use std::io;
use std::path::Path;
use std::process::{Command, ExitStatus};

/// Runs the git tool `program` with its working directory set to `dir`.
///
/// # Errors
///
/// Returns an I/O error if the program cannot be spawned (e.g. not installed).
pub fn launch_git_tool(program: &str, dir: &Path) -> io::Result<ExitStatus> {
    Command::new(program).current_dir(dir).status()
}

/// An editor command: the program to run plus the arguments it was configured
/// with.
///
/// `$EDITOR` very often carries arguments (`code --wait`, `emacsclient -nw`).
/// Those are split off here rather than passed to `Command::new` as one string,
/// which would look for a binary literally named `code --wait`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorCommand {
    /// The executable to spawn.
    pub program: String,
    /// The arguments preceding the file, in order.
    pub args: Vec<String>,
}

impl EditorCommand {
    /// Splits an editor setting into its program and arguments.
    ///
    /// The split is purely lexical whitespace separation - deliberately *not* a
    /// shell parse. Handing the string to a shell would turn a hostile
    /// `$EDITOR` into command injection; splitting it cannot, because every
    /// part stays a separate `argv` entry.
    pub fn parse(setting: &str) -> Self {
        let mut parts = setting.split_whitespace().map(str::to_string);
        let program = parts.next().unwrap_or_else(|| "vi".to_string());
        EditorCommand {
            program,
            args: parts.collect(),
        }
    }
}

/// Opens `file` in `editor`, with the working directory set to its parent.
///
/// # Errors
///
/// Returns an I/O error if the editor cannot be spawned.
pub fn open_in_editor(
    editor: &EditorCommand,
    file: &Path,
) -> io::Result<ExitStatus> {
    let mut command = Command::new(&editor.program);
    command.args(&editor.args);
    command.arg(file);
    if let Some(parent) = file.parent() {
        command.current_dir(parent);
    }
    command.status()
}

/// Opens `path` with the platform's default application (macOS `open`, Linux
/// `xdg-open`, Windows `start`), e.g. an image in the system previewer.
///
/// # Errors
///
/// Returns an I/O error if the opener program cannot be spawned.
pub fn open_default_app(path: &Path) -> io::Result<ExitStatus> {
    if cfg!(target_os = "windows") {
        // `start` is a cmd builtin; the empty "" is its (ignored) window title.
        return Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(path)
            .status();
    }
    let program = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    Command::new(program).arg(path).status()
}

/// Opens `url` in the default browser (macOS `open`, Linux `xdg-open`, Windows
/// `start`).
///
/// # Errors
///
/// Returns an I/O error if the opener program cannot be spawned.
pub fn open_url(url: &str) -> io::Result<ExitStatus> {
    if cfg!(target_os = "windows") {
        return Command::new("cmd").args(["/C", "start", "", url]).status();
    }
    let program = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    Command::new(program).arg(url).status()
}

/// Resolves the editor to use: an explicit `configured` value, then `$VISUAL`,
/// then `$EDITOR`, falling back to `vi`.
pub fn resolve_editor(configured: Option<&str>) -> EditorCommand {
    EditorCommand::parse(&editor_setting(configured, |name| {
        std::env::var(name).ok()
    }))
}

/// The raw editor setting, by precedence, before it is split into a command.
///
/// The environment lookup is injected so the precedence can be tested without
/// mutating process-wide state, which would race every other test in the
/// binary (they all share one environment).
fn editor_setting(
    configured: Option<&str>,
    lookup: impl Fn(&str) -> Option<String>,
) -> String {
    if let Some(editor) = configured
        && !editor.trim().is_empty()
    {
        return editor.to_string();
    }
    for var in ["VISUAL", "EDITOR"] {
        if let Some(value) = lookup(var)
            && !value.trim().is_empty()
        {
            return value;
        }
    }
    "vi".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_editor_has_no_arguments() {
        let editor = EditorCommand::parse("vim");
        assert_eq!(editor.program, "vim");
        assert!(editor.args.is_empty());
    }

    #[test]
    fn an_editor_with_arguments_keeps_them_as_separate_argv_entries() {
        // The bug this fixes: `Command::new("code --wait")` looked for a binary
        // of that literal name and failed with NotFound.
        let editor = EditorCommand::parse("code --wait");
        assert_eq!(editor.program, "code");
        assert_eq!(editor.args, ["--wait"]);

        let emacs = EditorCommand::parse("emacsclient -nw -c");
        assert_eq!(emacs.program, "emacsclient");
        assert_eq!(emacs.args, ["-nw", "-c"]);
    }

    #[test]
    fn surrounding_whitespace_does_not_become_an_argument() {
        let editor = EditorCommand::parse("  vim   -p  ");
        assert_eq!(editor.program, "vim");
        assert_eq!(editor.args, ["-p"]);
    }

    #[test]
    fn an_empty_setting_falls_back_to_vi() {
        assert_eq!(EditorCommand::parse("").program, "vi");
        assert_eq!(EditorCommand::parse("   ").program, "vi");
    }

    #[test]
    fn shell_metacharacters_stay_inert_arguments() {
        // No shell is involved, so a hostile setting cannot inject a command:
        // the parts remain literal argv entries that the editor will reject.
        let editor = EditorCommand::parse("vim; rm -rf /");
        assert_eq!(editor.program, "vim;");
        assert_eq!(editor.args, ["rm", "-rf", "/"]);
    }

    /// A fake environment, so the precedence can be checked without touching
    /// the real one (which every other test in this binary shares).
    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |name| {
            owned
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        }
    }

    /// The parse alone would not prove the fix: this actually spawns an editor
    /// with arguments and checks what landed in its `argv`.
    #[test]
    fn an_editor_with_arguments_really_receives_them_when_spawned() {
        let dir = std::env::temp_dir()
            .join(format!("hop-editor-spawn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir is creatable");
        let script = dir.join("fake-editor");
        let log = dir.join("argv.txt");
        std::fs::write(
            &script,
            format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > {}\n", log.display()),
        )
        .expect("script is writable");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                &script,
                std::fs::Permissions::from_mode(0o755),
            )
            .expect("script is executable");
        }
        let file = dir.join("note.md");
        std::fs::write(&file, "x").expect("file is writable");

        let editor = EditorCommand::parse(&format!(
            "{} --wait --reuse",
            script.display()
        ));
        open_in_editor(&editor, &file).expect("the fake editor spawns");

        let argv = std::fs::read_to_string(&log).expect("the editor ran");
        let args: Vec<&str> = argv.lines().collect();
        assert_eq!(
            args,
            ["--wait", "--reuse", file.to_str().expect("utf-8 path")],
            "the configured arguments must precede the file"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_configured_and_nothing_in_the_environment_falls_back_to_vi() {
        assert_eq!(editor_setting(None, env(&[])), "vi");
    }

    #[test]
    fn visual_outranks_editor_and_a_config_value_outranks_both() {
        let both = [("VISUAL", "code --wait"), ("EDITOR", "nano")];
        assert_eq!(editor_setting(None, env(&both)), "code --wait");
        assert_eq!(editor_setting(Some("helix"), env(&both)), "helix");
        // A blank config value is not a choice, so it does not win.
        assert_eq!(editor_setting(Some("   "), env(&both)), "code --wait");
    }

    #[test]
    fn editor_is_used_when_visual_is_unset_or_blank() {
        assert_eq!(editor_setting(None, env(&[("EDITOR", "nano")])), "nano");
        let blank = [("VISUAL", "  "), ("EDITOR", "nano")];
        assert_eq!(editor_setting(None, env(&blank)), "nano");
    }
}
