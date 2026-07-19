//! XDG path resolution and `~` expansion.
//!
//! Resolves the config and state directories under the application name. Home
//! resolution prefers `$HOME` (Unix), falling back to the Windows variables
//! when it is unset.

use std::env;
use std::path::{Path, PathBuf};

use crate::util::app_info::APP_NAME;

/// Reads an environment variable, treating an empty value as unset.
///
/// Injected into the resolvers below so their rules can be tested without
/// mutating the process environment, which every test in the binary shares.
type EnvLookup<'a> = &'a dyn Fn(&str) -> Option<String>;

/// The real process environment.
fn process_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.is_empty())
}

/// Resolves the user's home directory.
pub fn home_dir() -> Option<PathBuf> {
    resolve_home(&process_env)
}

/// Resolves the home directory from `lookup`: `$HOME` first, then the Windows
/// variables.
fn resolve_home(lookup: EnvLookup) -> Option<PathBuf> {
    if let Some(home) = lookup("HOME") {
        return Some(PathBuf::from(home));
    }
    if let Some(profile) = lookup("USERPROFILE") {
        return Some(PathBuf::from(profile));
    }
    match (lookup("HOMEDRIVE"), lookup("HOMEPATH")) {
        (Some(drive), Some(path)) => {
            Some(PathBuf::from(format!("{drive}{path}")))
        }
        _ => None,
    }
}

/// Expands a leading `~` (or `~/`) in `path` against the home directory.
pub fn expand_tilde(path: &str) -> PathBuf {
    expand_tilde_from(path, home_dir())
}

/// Expands a leading `~` against `home`, leaving the path alone when there is
/// no home to expand against.
fn expand_tilde_from(path: &str, home: Option<PathBuf>) -> PathBuf {
    if path == "~" {
        return home.unwrap_or_else(|| PathBuf::from(path));
    }
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = home
    {
        return home.join(rest);
    }
    PathBuf::from(path)
}

/// Returns `$XDG_<var>_HOME/hop`, else `<home>/<fallback>/hop`.
fn xdg_dir(env_var: &str, fallback: &str) -> PathBuf {
    resolve_xdg_dir(env_var, fallback, &process_env)
}

/// Resolves an XDG directory from `lookup`.
///
/// Per the XDG specification a value that is empty or not absolute is invalid
/// and is ignored rather than used, so a stray relative setting cannot scatter
/// state through the working directory.
fn resolve_xdg_dir(
    env_var: &str,
    fallback: &str,
    lookup: EnvLookup,
) -> PathBuf {
    if let Some(base) = lookup(env_var)
        && Path::new(&base).is_absolute()
    {
        return PathBuf::from(base).join(APP_NAME);
    }
    let home = resolve_home(lookup).unwrap_or_else(|| PathBuf::from("."));
    home.join(fallback).join(APP_NAME)
}

/// On Windows, `%<env_var>%/hop` when the variable is set.
#[cfg(windows)]
fn windows_dir(env_var: &str) -> Option<PathBuf> {
    env::var(env_var)
        .ok()
        .filter(|base| !base.is_empty())
        .map(|base| PathBuf::from(base).join(APP_NAME))
}

/// Config directory: `%APPDATA%\hop` on Windows, else `$XDG_CONFIG_HOME/hop`
/// or `~/.config/hop`.
pub fn config_dir() -> PathBuf {
    #[cfg(windows)]
    if let Some(dir) = windows_dir("APPDATA") {
        return dir;
    }
    xdg_dir("XDG_CONFIG_HOME", ".config")
}

/// State directory: `%LOCALAPPDATA%\hop` on Windows, else `$XDG_STATE_HOME/hop`
/// or `~/.local/state/hop`.
pub fn state_dir() -> PathBuf {
    #[cfg(windows)]
    if let Some(dir) = windows_dir("LOCALAPPDATA") {
        return dir;
    }
    xdg_dir("XDG_STATE_HOME", ".local/state")
}

/// Config file path (`<config_dir>/config.toml`).
pub fn config_file() -> PathBuf {
    config_dir().join("config.toml")
}

/// Git-status cache file (`<state_dir>/git-info-cache.toml`).
pub fn cache_file() -> PathBuf {
    state_dir().join("git-info-cache.toml")
}

/// Usage-state file tracking last-used and open-count (`<state_dir>/usage.toml`).
pub fn usage_file() -> PathBuf {
    state_dir().join("usage.toml")
}

/// Selected-repo path the shell wrapper reads to `cd`
/// (`<state_dir>/selected-repo.txt`).
pub fn selected_repo_file() -> PathBuf {
    state_dir().join("selected-repo.txt")
}

/// Log file (`<state_dir>/hop.log`).
pub fn log_file() -> PathBuf {
    state_dir().join("hop.log")
}

/// Persisted UI state file (`<state_dir>/ui-state.toml`), e.g. the sort mode.
pub fn ui_state_file() -> PathBuf {
    state_dir().join("ui-state.toml")
}

/// Path of the project-statistics cache in the state directory.
pub fn stats_file() -> PathBuf {
    state_dir().join("stats-cache.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake environment, so the rules can be checked without touching the
    /// real one (which every other test in this binary shares).
    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |name| {
            owned
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
                .filter(|value| !value.is_empty())
        }
    }

    #[test]
    fn home_prefers_the_unix_variable_then_the_windows_ones() {
        let both = env_of(&[("HOME", "/home/c"), ("USERPROFILE", "C:\\Users")]);
        assert_eq!(resolve_home(&both), Some(PathBuf::from("/home/c")));

        let windows = env_of(&[("USERPROFILE", "C:\\Users\\c")]);
        assert_eq!(resolve_home(&windows), Some(PathBuf::from("C:\\Users\\c")));

        let split = env_of(&[("HOMEDRIVE", "C:"), ("HOMEPATH", "\\Users\\c")]);
        assert_eq!(resolve_home(&split), Some(PathBuf::from("C:\\Users\\c")));

        assert_eq!(resolve_home(&env_of(&[])), None);
    }

    #[test]
    fn an_empty_home_counts_as_unset() {
        assert_eq!(resolve_home(&env_of(&[("HOME", "")])), None);
    }

    #[test]
    fn an_absolute_xdg_value_is_used() {
        let env = env_of(&[("XDG_CONFIG_HOME", "/xdg/config")]);
        assert_eq!(
            resolve_xdg_dir("XDG_CONFIG_HOME", ".config", &env),
            PathBuf::from("/xdg/config/hop")
        );
    }

    #[test]
    fn an_empty_or_relative_xdg_value_is_ignored_per_the_specification() {
        // A relative value is invalid; honouring it would scatter state
        // through whatever directory the user happened to start hop in.
        for value in ["", "relative/path", "./state"] {
            let pairs = [("XDG_STATE_HOME", value), ("HOME", "/home/c")];
            let env = env_of(&pairs);
            assert_eq!(
                resolve_xdg_dir("XDG_STATE_HOME", ".local/state", &env),
                PathBuf::from("/home/c/.local/state/hop"),
                "{value:?} must fall back to the home default"
            );
        }
    }

    #[test]
    fn a_missing_home_leaves_the_xdg_fallback_relative_to_the_cwd() {
        assert_eq!(
            resolve_xdg_dir("XDG_CONFIG_HOME", ".config", &env_of(&[])),
            PathBuf::from("./.config/hop")
        );
    }

    #[test]
    fn tilde_expands_only_at_the_start_of_a_path() {
        let home = Some(PathBuf::from("/home/c"));
        assert_eq!(
            expand_tilde_from("~", home.clone()),
            PathBuf::from("/home/c")
        );
        assert_eq!(
            expand_tilde_from("~/code/hop", home.clone()),
            PathBuf::from("/home/c/code/hop")
        );
        // Not a leading `~/`, so it stays verbatim.
        assert_eq!(
            expand_tilde_from("/tmp/~/x", home.clone()),
            PathBuf::from("/tmp/~/x")
        );
        assert_eq!(
            expand_tilde_from("~user/x", home),
            PathBuf::from("~user/x")
        );
    }

    #[test]
    fn tilde_stays_verbatim_when_there_is_no_home() {
        assert_eq!(expand_tilde_from("~", None), PathBuf::from("~"));
        assert_eq!(expand_tilde_from("~/code", None), PathBuf::from("~/code"));
    }
}
