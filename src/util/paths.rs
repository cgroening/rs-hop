//! XDG path resolution and `~` expansion.
//!
//! Resolves the config and state directories under the application name. Home
//! resolution prefers `$HOME` (Unix), falling back to the Windows variables
//! when it is unset.

use std::env;
use std::path::PathBuf;

use crate::util::app_info::APP_NAME;

/// Resolves the user's home directory.
pub fn home_dir() -> Option<PathBuf> {
    if let Ok(home) = env::var("HOME")
        && !home.is_empty()
    {
        return Some(PathBuf::from(home));
    }
    if let Ok(profile) = env::var("USERPROFILE")
        && !profile.is_empty()
    {
        return Some(PathBuf::from(profile));
    }
    match (env::var("HOMEDRIVE"), env::var("HOMEPATH")) {
        (Ok(drive), Ok(path)) if !drive.is_empty() && !path.is_empty() => {
            Some(PathBuf::from(format!("{drive}{path}")))
        }
        _ => None,
    }
}

/// Expands a leading `~` (or `~/`) in `path` against the home directory.
pub fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        return home_dir().unwrap_or_else(|| PathBuf::from(path));
    }
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(path)
}

/// Returns `$XDG_<var>_HOME/hop`, else `<home>/<fallback>/hop`.
fn xdg_dir(env_var: &str, fallback: &str) -> PathBuf {
    if let Ok(base) = env::var(env_var)
        && !base.is_empty()
    {
        return PathBuf::from(base).join(APP_NAME);
    }
    let home = home_dir().unwrap_or_else(|| PathBuf::from("."));
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
