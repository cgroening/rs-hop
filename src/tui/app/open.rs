//! Leaving the TUI for something else: the git tool, the editor, the platform's
//! default application, the clipboard or a GitHub tab.

use super::*;
use crate::tui::terminal::Tui;

use std::io;
use std::path::Path;

use crate::domain::repo::{self, Repo, RepoKind};
use crate::tui::git_columns::effective_info;
use crate::tui::presentation::github_url;
use crate::util::opener::launch_git_tool;

impl App {
    /// Records the open, writes the handoff path and returns the outcome for
    /// the selected entry. `launch_tool` distinguishes Enter from the `o` jump.
    pub(super) fn open_selected(
        &mut self,
        launch_tool: bool,
    ) -> Option<RunOutcome> {
        let index = self.selected_index()?;
        let repo = self.service.get(index)?.clone();
        if let Err(error) = self.service.mark_used(index) {
            self.set_status(format!("could not record usage: {error}"));
        }
        match repo.kind {
            RepoKind::Git if launch_tool => {
                self.write_selected(&repo.path);
                Some(RunOutcome::LaunchGitTool(repo.path))
            }
            RepoKind::Git => {
                self.write_selected(&repo.path);
                Some(RunOutcome::Jumped)
            }
            RepoKind::Path => self.open_path_entry(repo, launch_tool),
        }
    }

    /// Opens the selected Git entry's tool as an overlay: the run loop suspends
    /// the terminal, runs the tool to completion, then returns to the list and
    /// refreshes that entry. Non-Git entries are ignored.
    pub(super) fn open_git_inline(&mut self) -> Option<RunOutcome> {
        let index = self.selected_index()?;
        let repo = self.service.get(index)?.clone();
        if repo.kind != RepoKind::Git {
            self.set_status("not a git repo");
            return None;
        }
        if let Err(error) = self.service.mark_used(index) {
            self.set_status(format!("could not record usage: {error}"));
        }
        Some(RunOutcome::LaunchGitToolInline(repo.path))
    }

    /// Runs the git tool for `dir` with the terminal suspended, then refreshes
    /// only that entry's status in the background (no fetch, no progress bar).
    pub(super) fn run_git_inline(
        &mut self,
        tui: &mut Tui,
        dir: &Path,
    ) -> io::Result<()> {
        let Some(program) = self.config.git_program.clone() else {
            self.set_status("no git_program configured");
            return Ok(());
        };
        tui.suspend(|| {
            if let Err(error) = launch_git_tool(&program, dir) {
                log::error!("could not launch {program}: {error}");
            }
        })?;
        if !self.config.example_mode {
            self.refresh_paths(vec![dir.to_path_buf()], false, false);
        }
        Ok(())
    }

    /// Opens a file/folder entry: a folder `cd`s; on `o` (no launch) a file
    /// `cd`s to its parent; on Enter a text file opens in the editor and any
    /// other file in the default application.
    pub(super) fn open_path_entry(
        &mut self,
        repo: Repo,
        launch_tool: bool,
    ) -> Option<RunOutcome> {
        let class =
            repo::classify_path(&repo.path, &self.config.editor_extensions);
        if class == repo::PathClass::Folder {
            self.write_selected(&repo.path);
            return Some(RunOutcome::Jumped);
        }
        if !launch_tool {
            // Jump-only on a file lands the shell in its parent directory.
            let dir = repo
                .path
                .parent()
                .map_or_else(|| repo.path.clone(), Path::to_path_buf);
            self.write_selected(&dir);
            return Some(RunOutcome::Jumped);
        }
        match class {
            repo::PathClass::TextFile => Some(RunOutcome::OpenFile(repo.path)),
            _ => Some(RunOutcome::OpenWith(repo.path)),
        }
    }

    /// Opens the selected entry with the platform's default application,
    /// regardless of its kind (forces a text file into its GUI app, or reveals
    /// a folder in the file manager). Does not `cd`.
    pub(super) fn force_open_with(&mut self) -> Option<RunOutcome> {
        let index = self.selected_index()?;
        let repo = self.service.get(index)?.clone();
        if let Err(error) = self.service.mark_used(index) {
            self.set_status(format!("could not record usage: {error}"));
        }
        Some(RunOutcome::OpenWith(repo.path))
    }

    /// Writes the selected-repo handoff file, surfacing any error.
    pub(super) fn write_selected(&mut self, path: &std::path::Path) {
        if let Err(error) = self.service.write_selected(path) {
            self.set_status(format!("could not write selected path: {error}"));
        }
    }

    /// Copies the selected entry's path to the system clipboard.
    pub(super) fn copy_path(&mut self) {
        let paths = self.target_paths();
        if paths.is_empty() {
            return;
        }
        let count = paths.len();
        let text = paths.join("\n");
        match crate::util::clipboard::copy(&text) {
            Ok(()) => self.set_status(if count == 1 {
                "copied path to clipboard".to_string()
            } else {
                format!("copied {count} paths to clipboard")
            }),
            Err(error) => self.set_status(format!("copy failed: {error}")),
        }
    }

    /// Opens the selected git entry's GitHub page in the browser (a non-blocking
    /// GUI handoff, so the TUI stays up).
    pub(super) fn open_on_github(&mut self) {
        let targets = self.targets();
        let urls: Vec<String> = targets
            .iter()
            .filter_map(|&index| self.github_url_for(index))
            .collect();
        if urls.is_empty() {
            self.set_status("no GitHub remote");
            return;
        }
        let mut opened = 0;
        for url in &urls {
            match crate::util::opener::open_url(url) {
                Ok(_) => opened += 1,
                Err(error) => {
                    self.set_status(format!("could not open browser: {error}"));
                }
            }
        }
        if opened == 0 {
            return;
        }
        let skipped = targets.len() - urls.len();
        if opened == 1 && skipped == 0 {
            self.set_status(format!("opening {}", urls[0]));
        } else if skipped == 0 {
            self.set_status(format!("opening {opened} GitHub pages"));
        } else {
            self.set_status(format!(
                "opening {opened} GitHub pages ({skipped} skipped)"
            ));
        }
    }

    /// The GitHub URL for the entry at `index`, if it is a git repo with a
    /// resolvable remote (non-git entries and repos without a remote yield
    /// `None`).
    pub(super) fn github_url_for(&self, index: usize) -> Option<String> {
        let repo = self.service.get(index)?;
        let info = effective_info(repo, self.config.example_mode);
        let name = info.and_then(|info| info.github_repo_name.clone())?;
        github_url(&name, self.config.github_username.as_deref())
    }
}
