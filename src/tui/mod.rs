//! TUI front end: the three-tab list view, its key handling and overlays.
//!
//! [`App`] owns the resolved config, the [`RepoService`] and the view state
//! (active tab, cursor, sort, live filter and the current overlay). [`run`]
//! draws, polls events on a short timeout and drains background git-status
//! updates so the list shows immediately and fills in as `git` reports. Opening
//! an entry returns a [`RunOutcome`] the composition root acts on after the
//! terminal is restored.

pub mod colors;
pub mod form;
pub mod help;
pub mod navigation;
pub mod path_picker;
pub mod presentation;
pub mod table;
pub mod terminal;
pub mod text_input;
pub mod widgets;

use std::collections::HashSet;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

pub use terminal::Tui;

use crate::config::Config;
use crate::domain::filter::{Tab, belongs_to_tab, fuzzy_indices};
use crate::domain::path_repair::nearest_existing_on_disk;
use crate::domain::repo::{Repo, RepoKind};
use crate::domain::sort::{SortMode, sort_indices};
use crate::service::repo_service::RepoService;
use crate::service::status_service::spawn_refresh;
use crate::storage::cache;
use crate::storage::git_client::GitClient;
use crate::storage::ui_state;
use crate::tui::colors::{
    ACCENT, CHANGES, DANGER, DIM, MUTED, SELECTION_BG, tab_active,
};
use crate::tui::form::{FormResult, RepoDraft, RepoForm};
use crate::tui::path_picker::{PathPicker, PickerResult};
use crate::tui::presentation::{IconSet, footer_lines, render_empty_hint};
use crate::tui::widgets::{
    ConfirmModal, ConfirmResult, PromptResult, SelectModal, SelectResult,
    TextPrompt,
};

/// How long a transient status message stays visible.
const STATUS_TTL: Duration = Duration::from_secs(4);

/// What the user chose to do, acted on by the composition root after the
/// terminal is restored.
pub enum RunOutcome {
    /// Quit without opening anything.
    Quit,
    /// Path written; just exit so the shell can `cd`.
    Jumped,
    /// Path written; launch the git tool in this directory, then exit.
    LaunchGitTool(PathBuf),
    /// Open this file in the editor, then exit.
    OpenFile(PathBuf),
}

/// An active modal layered over the list.
enum Overlay {
    None,
    Help,
    Confirm(ConfirmModal, Vec<usize>),
    Prompt(TextPrompt, usize),
    Form(RepoForm, Option<usize>),
    Picker(PathPicker, PickerIntent),
    /// The list of errored entries; the vec maps rows to service indices.
    Errors(SelectModal, Vec<usize>),
    /// The action menu for an errored entry at the given service index.
    ErrorAction(SelectModal, usize),
}

/// Why the path picker is open.
enum PickerIntent {
    /// Repair the path of the entry at this index.
    Repair(usize),
    /// Choose a path for a new entry, then open the add form.
    Add,
}

/// The interactive application state.
pub struct App {
    config: Config,
    service: RepoService,
    icons: IconSet,
    git_client: Arc<dyn GitClient>,
    cache_path: PathBuf,
    ui_state_path: PathBuf,
    tab: Tab,
    cursor: usize,
    sort: SortMode,
    filtering: bool,
    filter: text_input::TextInput,
    overlay: Overlay,
    status_msg: Option<(String, Instant)>,
    loading: Option<(usize, usize)>,
    cache_generated_at: Option<DateTime<Local>>,
    last_fetched: Option<DateTime<Local>>,
    refresh_fetched: bool,
    status_rx: Option<Receiver<crate::service::status_service::StatusUpdate>>,
    /// Paths in the active refresh that have not been updated yet (drive the
    /// per-row spinner). Empty when no refresh is running.
    refreshing: HashSet<PathBuf>,
    /// When the active refresh started, for animating the spinner frame.
    refresh_started: Instant,
    /// Multi-selection by service index (survives sort/filter). Empty = none.
    selected: HashSet<usize>,
    /// Anchor display row for `Shift`-range selection.
    anchor: Option<usize>,
}

/// How the status is sourced on start.
pub enum StartupStatus {
    /// Show only the cache; do not run git at all (`--cached`).
    Cached,
    /// Refresh status in the background, optionally fetching first.
    Refresh {
        /// Whether to `git fetch` before gathering status.
        fetch: bool,
    },
}

impl App {
    /// Builds the app, hydrates status from the cache and, unless `--cached` or
    /// example mode, starts a background refresh (fetching first when asked).
    pub fn new(
        config: Config,
        service: RepoService,
        git_client: Arc<dyn GitClient>,
        cache_path: PathBuf,
        ui_state_path: PathBuf,
        startup: StartupStatus,
    ) -> Self {
        let icons = IconSet::new(config.icons);
        let cached = cache::load(&cache_path);
        let sort = ui_state::load_sort(&ui_state_path);
        let mut service = service;
        service.apply_git_infos(&cached.infos);
        let mut app = App {
            config,
            service,
            icons,
            git_client,
            cache_path,
            ui_state_path,
            tab: Tab::default(),
            cursor: 0,
            sort,
            filtering: false,
            filter: text_input::TextInput::new(""),
            overlay: Overlay::None,
            status_msg: None,
            loading: None,
            cache_generated_at: cached.generated_at,
            last_fetched: cached.fetched_at,
            refresh_fetched: false,
            status_rx: None,
            refreshing: HashSet::new(),
            refresh_started: Instant::now(),
            selected: HashSet::new(),
            anchor: None,
        };
        if let StartupStatus::Refresh { fetch } = startup
            && !app.config.example_mode
        {
            app.start_refresh(fetch);
        }
        app
    }

    /// The service indices belonging to the current tab (unsorted, unfiltered).
    fn tab_indices(&self) -> Vec<usize> {
        let repos = self.service.repos();
        (0..repos.len())
            .filter(|&i| belongs_to_tab(&repos[i], self.tab))
            .collect()
    }

    /// The ordered service indices visible in the current tab, after the sort
    /// or live fuzzy filter.
    fn ordered_view(&self) -> Vec<usize> {
        let repos = self.service.repos();
        let tab_indices = self.tab_indices();
        let query = self.filter.value();
        if self.filtering && !query.trim().is_empty() {
            let subset: Vec<Repo> =
                tab_indices.iter().map(|&i| repos[i].clone()).collect();
            return fuzzy_indices(&subset, &query)
                .into_iter()
                .map(|pos| tab_indices[pos])
                .collect();
        }
        let mut indices = tab_indices;
        sort_indices(repos, &mut indices, self.sort);
        indices
    }

    /// The selected service index, if the view is non-empty.
    fn selected_index(&self) -> Option<usize> {
        self.ordered_view().get(self.cursor).copied()
    }

    /// Clamps the cursor into the current view length.
    fn clamp_cursor(&mut self, view_len: usize) {
        if view_len == 0 {
            self.cursor = 0;
        } else if self.cursor >= view_len {
            self.cursor = view_len - 1;
        }
    }

    /// Sets a transient status message.
    fn set_status(&mut self, message: impl Into<String>) {
        self.status_msg = Some((message.into(), Instant::now()));
    }

    /// Expires the status message once its TTL passes.
    fn tick(&mut self) {
        if let Some((_, at)) = &self.status_msg
            && at.elapsed() > STATUS_TTL
        {
            self.status_msg = None;
        }
    }

    /// Starts a background refresh over the current tab's entries (with the
    /// progress bar).
    fn start_refresh(&mut self, fetch: bool) {
        let paths: Vec<PathBuf> = self
            .tab_indices()
            .iter()
            .filter_map(|&i| self.service.get(i).map(|r| r.path.clone()))
            .collect();
        self.refresh_paths(paths, fetch, true);
    }

    /// Starts a background refresh over `paths`, optionally fetching first.
    /// `show_bar` drives the full-width progress bar (and the global
    /// `fetched_at` update); a subset refresh passes `false`.
    fn refresh_paths(
        &mut self,
        paths: Vec<PathBuf>,
        fetch: bool,
        show_bar: bool,
    ) {
        if paths.is_empty() {
            return;
        }
        self.refreshing = paths.iter().cloned().collect();
        self.refresh_started = Instant::now();
        self.loading = if show_bar {
            Some((0, paths.len()))
        } else {
            None
        };
        // Only a full refresh updates the global "remote: fetched …" time.
        self.refresh_fetched = fetch && show_bar;
        self.status_rx =
            Some(spawn_refresh(Arc::clone(&self.git_client), paths, fetch));
    }

    /// Applies any pending background status updates without blocking.
    fn drain_status(&mut self) {
        let Some(rx) = self.status_rx.take() else {
            return;
        };
        let mut finished = false;
        loop {
            match rx.try_recv() {
                Ok(update) => {
                    self.service.set_git_info(&update.path, update.info);
                    self.refreshing.remove(&update.path);
                    if let Some((done, _)) = &mut self.loading {
                        *done += 1;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = true;
                    break;
                }
            }
        }
        if finished {
            let now = Local::now();
            self.cache_generated_at = Some(now);
            if self.refresh_fetched {
                self.last_fetched = Some(now);
            }
            // Persist the full current state (not just this pass's `collected`),
            // so a single-entry refresh never drops the other entries' cache.
            let infos: Vec<(PathBuf, crate::domain::repo::GitInfo)> = self
                .service
                .repos()
                .iter()
                .filter_map(|repo| {
                    repo.git_info.clone().map(|info| (repo.path.clone(), info))
                })
                .collect();
            let _ = cache::save(&self.cache_path, &infos, self.last_fetched);
            self.loading = None;
            self.refreshing.clear();
        } else {
            self.status_rx = Some(rx);
        }
    }

    /// Whether a background status refresh is currently running.
    fn is_refreshing(&self) -> bool {
        self.status_rx.is_some()
    }

    /// The current spinner frame glyph, if a refresh is running.
    fn spinner_frame(&self) -> Option<&'static str> {
        if self.refreshing.is_empty() {
            return None;
        }
        let frames = self.icons.spinner;
        let index = (self.refresh_started.elapsed().as_millis() / 120) as usize;
        Some(frames[index % frames.len()])
    }
}

/// Runs the TUI event loop until the user opens an entry or quits.
///
/// # Errors
/// Returns an I/O error if drawing or reading from the terminal fails.
pub fn run(mut app: App, tui: &mut Tui) -> io::Result<RunOutcome> {
    loop {
        tui.terminal.draw(|frame| app.render(frame))?;
        app.drain_status();
        // Poll faster while refreshing so the spinner animates smoothly.
        let timeout = if app.is_refreshing() { 80 } else { 150 };
        if event::poll(Duration::from_millis(timeout))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && let Some(outcome) = app.handle_key(key)
        {
            return Ok(outcome);
        }
        app.tick();
    }
}

impl App {
    /// Handles a key, returning an outcome when the loop should end.
    fn handle_key(&mut self, key: KeyEvent) -> Option<RunOutcome> {
        if is_global_quit(key) {
            return Some(RunOutcome::Quit);
        }
        match &mut self.overlay {
            Overlay::None => self.handle_list_key(key),
            _ => {
                self.handle_overlay_key(key);
                None
            }
        }
    }

    /// Handles a key for an open overlay, transitioning state as needed.
    fn handle_overlay_key(&mut self, key: KeyEvent) {
        let overlay = std::mem::replace(&mut self.overlay, Overlay::None);
        match overlay {
            Overlay::Help => {
                // Any key closes help except those that re-open it elsewhere.
                if !matches!(key.code, KeyCode::Char('?') | KeyCode::Esc) {
                    self.overlay = Overlay::Help;
                }
            }
            Overlay::Confirm(modal, targets) => match modal.handle_key(key) {
                ConfirmResult::Yes => self.do_delete(targets),
                ConfirmResult::No => {}
                ConfirmResult::Pending => {
                    self.overlay = Overlay::Confirm(modal, targets);
                }
            },
            Overlay::Prompt(mut prompt, index) => {
                match prompt.handle_key(key) {
                    PromptResult::Submit(value) => {
                        self.do_set_slug(index, value)
                    }
                    PromptResult::Cancel => {}
                    PromptResult::Pending => {
                        self.overlay = Overlay::Prompt(prompt, index);
                    }
                }
            }
            Overlay::Form(mut form, index) => match form.handle_key(key) {
                FormResult::Save(draft) => self.do_save_form(index, draft),
                FormResult::Cancel => {}
                FormResult::Pending => {
                    self.overlay = Overlay::Form(form, index)
                }
            },
            Overlay::Picker(mut picker, intent) => {
                match picker.handle_key(key) {
                    PickerResult::Selected(path) => {
                        self.do_picked(intent, path);
                    }
                    PickerResult::Cancel => {}
                    PickerResult::Pending => {
                        self.overlay = Overlay::Picker(picker, intent);
                    }
                }
            }
            Overlay::Errors(mut modal, indices) => {
                match modal.handle_key(key) {
                    SelectResult::Selected(row) => {
                        if let Some(&index) = indices.get(row) {
                            self.open_error_action(index);
                        }
                    }
                    SelectResult::Cancel => {}
                    SelectResult::Pending => {
                        self.overlay = Overlay::Errors(modal, indices);
                    }
                }
            }
            Overlay::ErrorAction(mut modal, index) => {
                match modal.handle_key(key) {
                    SelectResult::Selected(action) => {
                        self.run_error_action(index, action);
                    }
                    SelectResult::Cancel => {}
                    SelectResult::Pending => {
                        self.overlay = Overlay::ErrorAction(modal, index);
                    }
                }
            }
            Overlay::None => {}
        }
    }

    /// Handles a key for the list view (no overlay open).
    fn handle_list_key(&mut self, key: KeyEvent) -> Option<RunOutcome> {
        if self.filtering {
            return self.handle_filter_key(key);
        }
        match key.code {
            KeyCode::Char('1') => self.switch_tab(Tab::GitRepos),
            KeyCode::Char('2') => self.switch_tab(Tab::FilesAndFolders),
            KeyCode::Char('3') => self.switch_tab(Tab::Archive),
            KeyCode::Up => self.on_arrow(key, -1),
            KeyCode::Down => self.on_arrow(key, 1),
            KeyCode::Char(' ') => self.toggle_select(),
            KeyCode::Esc => self.clear_selection(),
            KeyCode::Enter => return self.open_selected(true),
            KeyCode::Char('o') => return self.open_selected(false),
            KeyCode::Char('q') => return Some(RunOutcome::Quit),
            KeyCode::Char('f') => self.filtering = true,
            KeyCode::Char('s') => self.cycle_sort(),
            KeyCode::Char('n') => self.open_add_picker(),
            KeyCode::Char('e') => self.open_edit_form(),
            KeyCode::Char('d') => self.open_delete_confirm(),
            KeyCode::Char('z') => self.toggle_fav(),
            KeyCode::Char('y') => self.copy_path(),
            KeyCode::Char('A') => self.toggle_archive(),
            KeyCode::Char('S') => self.open_slug_prompt(),
            KeyCode::Char('p') => self.open_repair_picker(),
            KeyCode::Char('!') => self.open_error_list(),
            KeyCode::Char('r') => self.reload_status(false),
            KeyCode::Char('R') => self.reload_status(true),
            KeyCode::Char('x') => self.refresh_targets(false),
            KeyCode::Char('X') => self.refresh_targets(true),
            KeyCode::Char('?') => self.overlay = Overlay::Help,
            _ => {}
        }
        None
    }

    /// Routes an arrow key: `Alt` reorders, `Shift` extends the selection,
    /// otherwise the cursor moves.
    fn on_arrow(&mut self, key: KeyEvent, delta: isize) {
        if key.modifiers.contains(KeyModifiers::ALT) {
            self.move_entry(delta);
        } else if key.modifiers.contains(KeyModifiers::SHIFT) {
            self.extend_selection(delta);
        } else {
            self.move_cursor(delta);
        }
    }

    /// Handles a key while the live filter is active.
    fn handle_filter_key(&mut self, key: KeyEvent) -> Option<RunOutcome> {
        match key.code {
            KeyCode::Esc => {
                self.filtering = false;
                self.filter = text_input::TextInput::new("");
                self.cursor = 0;
            }
            KeyCode::Up => self.move_cursor(-1),
            KeyCode::Down => self.move_cursor(1),
            KeyCode::Enter => return self.open_selected(true),
            _ => {
                if self.filter.handle_key(key) {
                    self.cursor = 0;
                }
            }
        }
        None
    }

    /// Switches to `tab`, resetting the cursor and clearing the selection.
    fn switch_tab(&mut self, tab: Tab) {
        self.tab = tab;
        self.cursor = 0;
        self.clear_selection();
    }

    /// Cycles the sort mode and persists it for the next run.
    fn cycle_sort(&mut self) {
        self.sort = self.sort.next();
        let _ = ui_state::save_sort(&self.ui_state_path, self.sort);
    }

    /// Moves the cursor cyclically within the current view; a plain move drops
    /// the range anchor so the next `Shift`-move re-anchors at the cursor.
    fn move_cursor(&mut self, delta: isize) {
        let len = self.ordered_view().len();
        self.cursor = navigation::cycle(self.cursor, len, delta);
        self.anchor = None;
    }

    /// Records the open, writes the handoff path and returns the outcome for
    /// the selected entry. `launch_tool` distinguishes Enter from the `o` jump.
    fn open_selected(&mut self, launch_tool: bool) -> Option<RunOutcome> {
        let index = self.selected_index()?;
        let repo = self.service.get(index)?.clone();
        if let Err(error) = self.service.mark_used(index) {
            self.set_status(format!("could not record usage: {error}"));
        }
        match repo.kind {
            RepoKind::File if launch_tool => {
                Some(RunOutcome::OpenFile(repo.path))
            }
            RepoKind::File => {
                let dir = repo
                    .path
                    .parent()
                    .map_or_else(|| repo.path.clone(), |p| p.to_path_buf());
                self.write_selected(&dir);
                Some(RunOutcome::Jumped)
            }
            RepoKind::Git if launch_tool => {
                self.write_selected(&repo.path);
                Some(RunOutcome::LaunchGitTool(repo.path))
            }
            _ => {
                self.write_selected(&repo.path);
                Some(RunOutcome::Jumped)
            }
        }
    }

    /// Writes the selected-repo handoff file, surfacing any error.
    fn write_selected(&mut self, path: &std::path::Path) {
        if let Err(error) = self.service.write_selected(path) {
            self.set_status(format!("could not write selected path: {error}"));
        }
    }

    /// Opens the path picker to choose a path for a new entry.
    fn open_add_picker(&mut self) {
        let start = crate::util::paths::home_dir()
            .unwrap_or_else(|| PathBuf::from("/"));
        self.overlay =
            Overlay::Picker(PathPicker::new(&start, true), PickerIntent::Add);
    }

    /// Opens the edit form for the selected entry.
    fn open_edit_form(&mut self) {
        if let Some(index) = self.selected_index() {
            self.edit_form_for(index);
        }
    }

    /// Opens the edit form for the entry at `index`.
    fn edit_form_for(&mut self, index: usize) {
        let Some(repo) = self.service.get(index) else {
            return;
        };
        let form = RepoForm::for_edit(
            repo.name.as_deref().unwrap_or(""),
            &repo.path.to_string_lossy(),
            repo.slug.as_deref().unwrap_or(""),
            repo.kind,
            repo.fav,
        );
        self.overlay = Overlay::Form(form, Some(index));
    }

    /// Opens the delete confirmation for the target entries (selection/cursor).
    fn open_delete_confirm(&mut self) {
        let targets = self.targets();
        if !targets.is_empty() {
            self.confirm_delete(targets);
        }
    }

    /// Opens the delete confirmation for a single entry at `index`.
    fn delete_confirm_for(&mut self, index: usize) {
        self.confirm_delete(vec![index]);
    }

    /// Opens a delete confirmation whose message names the count of `targets`.
    fn confirm_delete(&mut self, targets: Vec<usize>) {
        let message = if targets.len() == 1 {
            let name = self
                .service
                .get(targets[0])
                .map_or_else(String::new, Repo::display_name);
            format!("Delete \"{name}\" from the list?")
        } else {
            format!("Delete {} entries from the list?", targets.len())
        };
        self.overlay = Overlay::Confirm(
            ConfirmModal::new("Delete entries", message),
            targets,
        );
    }

    /// Opens the slug prompt for the selected entry.
    fn open_slug_prompt(&mut self) {
        let Some(index) = self.selected_index() else {
            return;
        };
        let current = self
            .service
            .get(index)
            .and_then(|r| r.slug.clone())
            .unwrap_or_default();
        self.overlay = Overlay::Prompt(
            TextPrompt::new("Set slug", "slug", &current),
            index,
        );
    }

    /// Opens the path picker to repair the selected entry's missing path.
    fn open_repair_picker(&mut self) {
        if let Some(index) = self.selected_index() {
            self.repair_picker_for(index);
        }
    }

    /// Opens the repair picker for the entry at `index`, starting at the nearest
    /// existing ancestor of its (missing) path.
    fn repair_picker_for(&mut self, index: usize) {
        let Some(repo) = self.service.get(index) else {
            return;
        };
        let start = nearest_existing_on_disk(&repo.path)
            .unwrap_or_else(|| PathBuf::from("/"));
        self.overlay = Overlay::Picker(
            PathPicker::new(&start, false),
            PickerIntent::Repair(index),
        );
    }

    /// Opens the popup listing all entries with a missing or invalid path.
    fn open_error_list(&mut self) {
        let repos = self.service.repos();
        let mut indices = Vec::new();
        let mut labels = Vec::new();
        for (index, repo) in repos.iter().enumerate() {
            if let Some(error) = repo.entry_error() {
                labels.push(format!("{} — {error}", repo.display_name()));
                indices.push(index);
            }
        }
        if indices.is_empty() {
            self.set_status("no errors");
            return;
        }
        self.overlay =
            Overlay::Errors(SelectModal::new("Errors", labels, 0), indices);
    }

    /// Opens the action menu for an errored entry at `index`.
    fn open_error_action(&mut self, index: usize) {
        let name = self
            .service
            .get(index)
            .map_or_else(String::new, Repo::display_name);
        let actions = vec![
            "Repair path".to_string(),
            "Edit".to_string(),
            "Delete".to_string(),
        ];
        self.overlay = Overlay::ErrorAction(
            SelectModal::new(format!("Fix \"{name}\""), actions, 0),
            index,
        );
    }

    /// Runs the chosen action menu entry for the errored entry at `index`.
    fn run_error_action(&mut self, index: usize, action: usize) {
        match action {
            0 => self.repair_picker_for(index),
            1 => self.edit_form_for(index),
            _ => self.delete_confirm_for(index),
        }
    }

    /// Copies the selected entry's path to the system clipboard.
    fn copy_path(&mut self) {
        let Some(index) = self.selected_index() else {
            return;
        };
        let Some(repo) = self.service.get(index) else {
            return;
        };
        let path = repo.path.to_string_lossy().into_owned();
        match crate::util::clipboard::copy(&path) {
            Ok(()) => self.set_status("copied path to clipboard"),
            Err(error) => self.set_status(format!("copy failed: {error}")),
        }
    }

    /// The entries an action applies to: the multi-selection, or the cursor
    /// entry when nothing is selected. Sorted ascending.
    fn targets(&self) -> Vec<usize> {
        if self.selected.is_empty() {
            return self.selected_index().into_iter().collect();
        }
        let mut indices: Vec<usize> = self.selected.iter().copied().collect();
        indices.sort_unstable();
        indices
    }

    /// Toggles the favourite flag of the target entries (all on, else all off).
    fn toggle_fav(&mut self) {
        let targets = self.targets();
        if targets.is_empty() {
            return;
        }
        let all_fav = targets
            .iter()
            .all(|&i| self.service.get(i).is_some_and(|r| r.fav));
        if let Err(error) = self.service.set_fav_many(&targets, !all_fav) {
            self.set_status(format!("could not change favourite: {error}"));
        }
        self.clear_selection();
    }

    /// Archives or restores the target entries (all archived, else all on) and
    /// keeps the cursor in range.
    fn toggle_archive(&mut self) {
        let targets = self.targets();
        if targets.is_empty() {
            return;
        }
        let all_archived = targets
            .iter()
            .all(|&i| self.service.get(i).is_some_and(|r| r.archived));
        if let Err(error) =
            self.service.set_archived_many(&targets, !all_archived)
        {
            self.set_status(format!("could not change archive: {error}"));
        }
        self.clear_selection();
        let len = self.ordered_view().len();
        self.clamp_cursor(len);
    }

    /// Restarts the full background refresh, optionally fetching first.
    fn reload_status(&mut self, fetch: bool) {
        if self.config.example_mode {
            self.set_status("example mode: live status is off");
            return;
        }
        self.set_status(if fetch {
            "fetching and reloading status…"
        } else {
            "reloading status…"
        });
        self.start_refresh(fetch);
    }

    /// Refreshes the target entries in the background, optionally fetching
    /// first. The global "remote: fetched …" line is left untouched.
    fn refresh_targets(&mut self, fetch: bool) {
        if self.config.example_mode {
            self.set_status("example mode: live status is off");
            return;
        }
        let targets = self.targets();
        let paths: Vec<PathBuf> = targets
            .iter()
            .filter_map(|&i| self.service.get(i).map(|r| r.path.clone()))
            .collect();
        if paths.is_empty() {
            return;
        }
        let message = if targets.len() == 1 {
            let name = self
                .service
                .get(targets[0])
                .map_or_else(String::new, Repo::display_name);
            format!("refreshing {name}…")
        } else {
            format!("refreshing {} entries…", targets.len())
        };
        self.set_status(message);
        self.refresh_paths(paths, fetch, false);
        self.clear_selection();
    }

    /// Toggles the selection of the cursor entry and re-anchors the range.
    fn toggle_select(&mut self) {
        if let Some(index) = self.selected_index() {
            if !self.selected.remove(&index) {
                self.selected.insert(index);
            }
            self.anchor = Some(self.cursor);
        }
    }

    /// Clears the multi-selection.
    fn clear_selection(&mut self) {
        self.selected.clear();
        self.anchor = None;
    }

    /// Extends the range selection by moving the cursor (clamped, not cyclic)
    /// and selecting every row between the anchor and the cursor.
    fn extend_selection(&mut self, delta: isize) {
        let view = self.ordered_view();
        if view.is_empty() {
            return;
        }
        let anchor = *self.anchor.get_or_insert(self.cursor);
        let last = view.len() - 1;
        let new =
            (self.cursor as isize + delta).clamp(0, last as isize) as usize;
        self.cursor = new;
        let (lo, hi) = (anchor.min(new).min(last), anchor.max(new).min(last));
        self.selected = view[lo..=hi].iter().copied().collect();
    }

    /// Moves the cursor entry within the custom order (only in custom sort).
    fn move_entry(&mut self, delta: isize) {
        if self.sort != SortMode::Custom {
            self.set_status("switch to custom sort (s) to reorder");
            return;
        }
        let view = self.ordered_view();
        if view.is_empty() {
            return;
        }
        let cur = self.cursor.min(view.len() - 1);
        let neighbor = cur as isize + delta;
        if neighbor < 0 || neighbor as usize >= view.len() {
            return;
        }
        let (a, b) = (view[cur], view[neighbor as usize]);
        let repos = self.service.repos();
        // Stay within the favourites / non-favourites group (favs keep on top).
        if repos[a].fav != repos[b].fav {
            return;
        }
        let moved = repos[a].path.clone();
        if self.service.swap_entries(a, b).is_ok()
            && let Some(pos) = self
                .ordered_view()
                .iter()
                .position(|&i| self.service.repos()[i].path == moved)
        {
            self.cursor = pos;
        }
    }

    /// Deletes the confirmed target entries.
    fn do_delete(&mut self, targets: Vec<usize>) {
        match self.service.delete_many(&targets) {
            Ok(()) => {
                self.clear_selection();
                let len = self.ordered_view().len();
                self.clamp_cursor(len);
                let count = targets.len();
                self.set_status(if count == 1 {
                    "deleted entry".to_string()
                } else {
                    format!("deleted {count} entries")
                });
            }
            Err(error) => self.set_status(format!("delete failed: {error}")),
        }
    }

    /// Sets or clears the slug of the entry at `index`.
    fn do_set_slug(&mut self, index: usize, value: String) {
        let slug = if value.trim().is_empty() {
            None
        } else {
            Some(value)
        };
        match self.service.set_slug(index, slug) {
            Ok(()) => self.set_status("slug updated"),
            Err(error) => self.set_status(format!("{error}")),
        }
    }

    /// Saves the add or edit form into a new or existing entry.
    fn do_save_form(&mut self, index: Option<usize>, draft: RepoDraft) {
        let path = crate::util::paths::expand_tilde(draft.path.trim());
        if draft.path.trim().is_empty() {
            self.set_status("path must not be empty");
            return;
        }
        match index {
            Some(index) => {
                let Some(mut repo) = self.service.get(index).cloned() else {
                    return;
                };
                apply_draft(&mut repo, draft, path);
                let result = self.service.update(index, repo);
                self.report(result, "entry updated");
            }
            None => {
                let mut repo = Repo::new(path.clone());
                apply_draft(&mut repo, draft, path);
                let result = self.service.add(repo);
                self.report(result, "entry added");
            }
        }
        if !self.config.example_mode {
            self.start_refresh(false);
        }
    }

    /// Applies a picked path to its intent (repair an entry, or add a new one).
    fn do_picked(&mut self, intent: PickerIntent, path: PathBuf) {
        match intent {
            PickerIntent::Repair(index) => {
                match self.service.set_path(index, path) {
                    Ok(()) => {
                        self.set_status("path repaired");
                        if !self.config.example_mode {
                            self.start_refresh(false);
                        }
                    }
                    Err(error) => {
                        self.set_status(format!("repair failed: {error}"))
                    }
                }
            }
            PickerIntent::Add => {
                let kind = guess_kind(&path);
                let form = RepoForm::for_add(&path.to_string_lossy(), kind);
                self.overlay = Overlay::Form(form, None);
            }
        }
    }

    /// Reports a service result as a transient status message.
    fn report(
        &mut self,
        result: crate::domain::error::Result<()>,
        ok_message: &str,
    ) {
        match result {
            Ok(()) => self.set_status(ok_message),
            Err(error) => self.set_status(format!("{error}")),
        }
    }
}

/// Copies the draft's fields onto `repo`, keeping its runtime/example fields.
fn apply_draft(repo: &mut Repo, draft: RepoDraft, path: PathBuf) {
    repo.name = draft.name;
    repo.path = path;
    repo.slug = draft.slug;
    repo.kind = draft.kind;
    repo.fav = draft.fav;
}

/// Guesses an entry kind from a chosen path: a git work tree, a folder or a
/// file.
fn guess_kind(path: &std::path::Path) -> RepoKind {
    if path.join(".git").exists() {
        RepoKind::Git
    } else if path.is_dir() {
        RepoKind::Folder
    } else {
        RepoKind::File
    }
}

/// Whether `key` is the global quit chord (`Ctrl+Q`).
fn is_global_quit(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && key.code == KeyCode::Char('q')
}

impl App {
    /// Renders the whole screen.
    fn render(&self, frame: &mut Frame) {
        let area = frame.area();
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4), // header box: 2 lines + rounded border
                Constraint::Min(1),
                Constraint::Length(2),
            ])
            .split(area);
        self.render_header(frame, rows[0]);
        // One column of margin on each side of the list.
        let body = Rect {
            x: rows[1].x.saturating_add(1),
            width: rows[1].width.saturating_sub(2),
            ..rows[1]
        };
        self.render_body(frame, body);
        self.render_footer(frame, rows[2]);
        self.render_overlay(frame, area);
    }

    /// Renders the rounded header box: brand + tabs on the first line, the
    /// combined info line (counts, sort, status and remote) on the second.
    fn render_header(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let lines = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(inner);
        self.render_tab_bar(frame, lines[0]);
        self.render_info(frame, lines[1]);
    }

    /// Renders the tab line: the `hop` brand (accent), then `[n] Label` tabs -
    /// the active one bold green, the rest dim, separated by a dim `│`.
    fn render_tab_bar(&self, frame: &mut Frame, area: Rect) {
        let mut spans = vec![Span::styled(
            " hop   ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        )];
        for (index, tab) in Tab::ALL.iter().enumerate() {
            if index > 0 {
                spans.push(Span::styled(
                    "  \u{2502}  ",
                    Style::default().fg(DIM),
                ));
            }
            let label = format!("[{}] {}", index + 1, tab.title());
            let style = if *tab == self.tab {
                Style::default()
                    .fg(tab_active())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(DIM)
            };
            spans.push(Span::styled(label, style));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    /// Renders the single info line - error count, entry count, sort, local
    /// status and remote fetch time, each behind its icon - or the progress bar
    /// while a refresh is running.
    fn render_info(&self, frame: &mut Frame, area: Rect) {
        if let Some((done, total)) = self.loading {
            render_progress(frame, area, done, total);
            return;
        }
        let icons = self.icons;
        let muted = Style::default().fg(MUTED);
        let sep = || Span::styled("   ", Style::default().fg(DIM));
        let mut spans = vec![Span::raw(" ")];

        let errors = self.error_count();
        if errors > 0 {
            spans.push(Span::styled(
                format!("{}{errors}", icons.missing),
                Style::default().fg(DANGER).add_modifier(Modifier::BOLD),
            ));
            spans.push(sep());
        }
        spans.push(Span::styled(
            format!("{} {}", icons.count, self.ordered_view().len()),
            muted,
        ));
        spans.push(sep());
        spans.push(Span::styled(
            format!("{} {}", icons.sort, self.sort.label()),
            muted,
        ));

        if self.config.example_mode {
            spans.push(sep());
            spans.push(Span::styled("example mode", muted));
        } else {
            if let Some(at) = self.cache_generated_at {
                let age = Local::now().signed_duration_since(at);
                spans.push(sep());
                spans.push(Span::styled(
                    format!(
                        "{} {} ({} ago)",
                        icons.clock,
                        at.format("%Y-%m-%d %H:%M"),
                        relative_age(age),
                    ),
                    muted,
                ));
            }
            spans.push(sep());
            spans.push(self.remote_span(icons.remote, muted));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    /// The remote-fetch segment of the info line: amber when over a day old or
    /// never fetched, muted otherwise.
    fn remote_span(&self, icon: &str, muted: Style) -> Span<'static> {
        match self.last_fetched {
            None => Span::styled(
                format!("{icon} never fetched"),
                Style::default().fg(CHANGES),
            ),
            Some(at) => {
                let age = Local::now().signed_duration_since(at);
                let stale = age.num_hours() >= 24;
                let suffix = if stale { "  (stale)" } else { "" };
                let text = format!(
                    "{icon} {} ({} ago){suffix}",
                    at.format("%Y-%m-%d %H:%M"),
                    relative_age(age),
                );
                let style = if stale {
                    Style::default().fg(CHANGES)
                } else {
                    muted
                };
                Span::styled(text, style)
            }
        }
    }

    /// The number of current-tab entries with a missing or invalid path.
    fn error_count(&self) -> usize {
        let repos = self.service.repos();
        self.tab_indices()
            .iter()
            .filter(|&&i| repos[i].entry_error().is_some())
            .count()
    }

    /// Renders the entry table, or an empty hint.
    fn render_body(&self, frame: &mut Frame, area: Rect) {
        let view = self.ordered_view();
        if view.is_empty() {
            render_empty_hint(frame, area, empty_hint(self.tab));
            return;
        }
        let repos = self.service.repos();
        let visible: Vec<&Repo> = view.iter().map(|&i| &repos[i]).collect();
        let cursor = self.cursor.min(visible.len() - 1);
        // Rows still in flight show an animated spinner in the status column.
        let spinner =
            self.spinner_frame().map(|glyph| (&self.refreshing, glyph));
        // Which visible rows are part of the multi-selection.
        let selected: Vec<bool> =
            view.iter().map(|i| self.selected.contains(i)).collect();
        let table_view = table::TableView {
            tab: self.tab,
            config: &self.config,
            icons: &self.icons,
            example_mode: self.config.example_mode,
            spinner,
            selected: &selected,
            has_selection: !self.selected.is_empty(),
        };
        table::render_table(frame, area, &visible, cursor, &table_view);
    }

    /// Renders the footer: the filter line, a status message, or key hints.
    fn render_footer(&self, frame: &mut Frame, area: Rect) {
        if self.filtering {
            let mut spans =
                vec![Span::styled("filter: ", Style::default().fg(ACCENT))];
            spans.extend(self.filter.render_line(Style::default()).spans);
            let lines = vec![
                Line::from(spans),
                Line::from(Span::styled(
                    " Enter open · Esc clear",
                    Style::default().fg(DIM),
                )),
            ];
            frame.render_widget(Paragraph::new(lines), area);
            return;
        }
        if let Some((message, _)) = &self.status_msg {
            let lines = vec![
                Line::from(Span::styled(
                    format!(" {message}"),
                    Style::default().fg(ACCENT),
                )),
                hint_line(self.tab),
            ];
            frame.render_widget(Paragraph::new(lines), area);
            return;
        }
        let lines = footer_lines(&hints(self.tab), area.width);
        frame.render_widget(Paragraph::new(lines), area);
    }

    /// Renders the active overlay, if any.
    fn render_overlay(&self, frame: &mut Frame, area: Rect) {
        match &self.overlay {
            Overlay::None => {}
            Overlay::Help => help::render(frame, area),
            Overlay::Confirm(modal, _) => modal.render(frame, area),
            Overlay::Prompt(prompt, _) => prompt.render(frame, area),
            Overlay::Form(form, _) => form.render(frame, area),
            Overlay::Picker(picker, _) => picker.render(frame, area),
            Overlay::Errors(modal, _) => modal.render(frame, area),
            Overlay::ErrorAction(modal, _) => modal.render(frame, area),
        }
    }
}

/// The placeholder text for an empty tab.
fn empty_hint(tab: Tab) -> &'static str {
    match tab {
        Tab::GitRepos => "No git repos. Press n to add one.",
        Tab::FilesAndFolders => "No folders or files. Press n to add one.",
        Tab::Archive => "Nothing archived.",
    }
}

/// The footer key hints for `tab` (only the keys relevant to that tab).
fn hints(tab: Tab) -> Vec<(&'static str, &'static str)> {
    let mut hints: Vec<(&str, &str)> = vec![
        ("Enter", "open"),
        ("o", "cd"),
        ("Space", "select"),
        ("f", "filter"),
        ("s", "sort"),
        ("n", "add"),
        ("e", "edit"),
        ("d", "del"),
        ("z", "fav"),
    ];
    // Archive tab restores; the others archive.
    hints.push(match tab {
        Tab::Archive => ("A", "restore"),
        _ => ("A", "archive"),
    });
    hints.push(("S", "slug"));
    hints.push(("y", "copy path"));
    hints.push(("p", "fix path"));
    // Git status refresh only makes sense where entries are git repositories.
    if tab == Tab::GitRepos {
        hints.push(("x/r", "refresh"));
    }
    hints.push(("!", "errors"));
    hints.push(("?", "help"));
    hints.push(("q", "quit"));
    hints
}

/// A single dim hint line pointing at the help overlay.
fn hint_line(_tab: Tab) -> Line<'static> {
    Line::from(Span::styled(" ? help · q quit", Style::default().fg(DIM)))
}

/// Renders a solid progress bar for an in-flight status refresh, filling the
/// whole `area` (full height and width) with a centred label. The label colour
/// is chosen per cell from whether it sits over the filled or unfilled part, so
/// it never ends up dark text on the dark (unfilled) background.
fn render_progress(frame: &mut Frame, area: Rect, done: usize, total: usize) {
    // Leave one blank cell of padding on each side of the bar.
    let area = Rect {
        x: area.x.saturating_add(1),
        width: area.width.saturating_sub(2),
        ..area
    };
    if area.width == 0 || area.height == 0 {
        return;
    }
    let ratio = if total == 0 {
        0.0
    } else {
        (done as f64 / total as f64).clamp(0.0, 1.0)
    };
    let filled = (f64::from(area.width) * ratio).round() as u16;
    let label: Vec<char> =
        format!("refreshing {done}/{total}").chars().collect();
    let label_width = label.len() as u16;
    let label_start = area.x + area.width.saturating_sub(label_width) / 2;
    let label_row = area.y + area.height / 2;

    let buf = frame.buffer_mut();
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let over_filled = (x - area.x) < filled;
            let bg = if over_filled { ACCENT } else { SELECTION_BG };
            let is_label = y == label_row
                && x >= label_start
                && x < label_start + label_width;
            let (symbol, fg) = if is_label {
                let ch = label[(x - label_start) as usize];
                // Dark text on the light filled bar, light text on the rest.
                let fg = if over_filled { Color::Black } else { ACCENT };
                (ch.to_string(), fg)
            } else {
                (" ".to_string(), bg)
            };
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(&symbol);
                cell.set_style(
                    Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
                );
            }
        }
    }
}

/// A short relative age like `2d`, `5h` or `3m` for the remote line.
fn relative_age(age: chrono::Duration) -> String {
    if age.num_days() >= 1 {
        return format!("{}d", age.num_days());
    }
    if age.num_hours() >= 1 {
        return format!("{}h", age.num_hours());
    }
    format!("{}m", age.num_minutes().max(0))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crossterm::event::KeyModifiers;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::config::{Config, IconVariant};
    use crate::domain::repo::GitInfo;
    use crate::storage::in_memory_repository::InMemoryRepoRepository;

    /// A git client that does nothing (the smoke test runs in example mode).
    struct NoGit;

    impl GitClient for NoGit {
        fn collect(&self, _path: &Path) -> GitInfo {
            GitInfo::default()
        }
        fn fetch(&self, _path: &Path) {}
    }

    fn sample_app() -> App {
        let mut git = Repo::new(PathBuf::from("/code/hop"));
        git.name = Some("hop".to_string());
        git.fav = true;
        let mut missing = Repo::new(PathBuf::from("/code/gone"));
        missing.slug = Some("gone".to_string());
        let mut folder = Repo::new(PathBuf::from("/notes"));
        folder.kind = RepoKind::Folder;
        let mut archived = Repo::new(PathBuf::from("/old"));
        archived.archived = true;
        let dir = std::env::temp_dir()
            .join(format!("hop-tui-test-{}", std::process::id()));
        let service = RepoService::new(
            Box::new(InMemoryRepoRepository::new(vec![
                git, missing, folder, archived,
            ])),
            dir.join("usage.toml"),
            dir.join("selected.txt"),
        )
        .unwrap();
        let config = Config {
            example_mode: true,
            icons: IconVariant::Ascii,
            ..Config::default()
        };
        App::new(
            config,
            service,
            Arc::new(NoGit),
            dir.join("cache.toml"),
            dir.join("ui-state.toml"),
            StartupStatus::Refresh { fetch: false },
        )
    }

    fn press(app: &mut App, code: KeyCode) {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    #[test]
    fn renders_every_tab_without_panicking() {
        let mut app = sample_app();
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        for tab in ['1', '2', '3'] {
            press(&mut app, KeyCode::Char(tab));
            terminal.draw(|frame| app.render(frame)).unwrap();
        }
    }

    #[test]
    fn filter_and_help_overlay_render() {
        let mut app = sample_app();
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        press(&mut app, KeyCode::Char('F'));
        press(&mut app, KeyCode::Char('h'));
        terminal.draw(|frame| app.render(frame)).unwrap();
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('?'));
        terminal.draw(|frame| app.render(frame)).unwrap();
    }

    #[test]
    fn opening_a_git_repo_returns_launch_outcome() {
        let mut app = sample_app();
        // The first git-tab entry is the git repo "hop".
        let outcome =
            app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(outcome, Some(RunOutcome::LaunchGitTool(_))));
    }

    #[test]
    fn ctrl_q_quits() {
        let mut app = sample_app();
        let outcome = app.handle_key(KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::CONTROL,
        ));
        assert!(matches!(outcome, Some(RunOutcome::Quit)));
    }
}
