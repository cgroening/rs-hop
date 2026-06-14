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
pub mod preview;
pub mod scan_picker;
pub mod sections_modal;
pub mod sections_view;
pub mod table;
pub mod terminal;
pub mod text_input;
pub mod widgets;

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
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
use crate::domain::repo::{self, Repo, RepoKind};
use crate::domain::sections;
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
use crate::tui::presentation::{
    IconSet, footer_lines, github_url, render_empty_hint,
};
use crate::tui::preview::PreviewMode;
use crate::tui::sections_modal::{SectionsAction, SectionsModal};
use crate::tui::widgets::{
    ConfirmModal, ConfirmResult, PromptResult, SelectModal, SelectResult,
    TextPrompt,
};
use crate::util::opener::launch_git_tool;

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
    /// Launch the git tool in this directory as an overlay (the loop suspends
    /// the terminal, runs the tool, then returns to the list).
    LaunchGitToolInline(PathBuf),
    /// Open this file in the editor, then exit.
    OpenFile(PathBuf),
    /// Open this file with the platform's default application, then exit.
    OpenWith(PathBuf),
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
    /// The jump-to-section picker; the vec maps rows to entry display positions.
    SectionJump(SelectModal, Vec<usize>),
    /// The manage-sections list.
    Sections(SectionsModal),
    /// A prompt to add or rename a section.
    SectionPrompt(TextPrompt, SectionPromptKind),
    /// A confirm dialog to delete the named section.
    SectionDelete(ConfirmModal, String),
}

/// Why the section prompt is open.
enum SectionPromptKind {
    /// Create a new section.
    New,
    /// Rename the section with this current name.
    Rename(String),
}

/// Why the path picker is open.
enum PickerIntent {
    /// Repair the path of the entry at this index.
    Repair(usize),
    /// Fill the path field of a form already in progress.
    FormPath(Box<RepoForm>, Option<usize>),
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
    /// Scroll offset of the sectioned Files list, kept across frames so the
    /// cursor pages within the viewport (the git tabs scroll statelessly).
    list_offset: std::cell::Cell<usize>,
    /// Whether the startup mode permits background refreshes (drives the
    /// first-visit refresh of the Git Repos / Archive tabs).
    auto_refresh: bool,
    /// Tabs whose entries have been refreshed since start, so a tab is only
    /// auto-refreshed on its first visit.
    refreshed_tabs: HashSet<Tab>,
    /// Paths of `Path`-kind entries found missing by the on-demand existence
    /// check (`r` on the Files tab). Empty until checked; never run on start.
    files_missing: HashSet<PathBuf>,
    /// Whether slugs are shown inline (dim, italic) after the entry name.
    show_slugs: bool,
    /// When on, only git entries with a status change are shown (session-only).
    changes_only: bool,
    /// The cursor entry's path per tab, restored when returning to that tab.
    tab_focus: HashMap<Tab, PathBuf>,
    /// The last rendered list height (entry rows), for page-wise navigation.
    list_height: std::cell::Cell<usize>,
    /// Where the detail/preview panel sits (persisted).
    preview: PreviewMode,
    /// Cached `git log` excerpts for the preview, keyed by entry path.
    preview_log: HashMap<PathBuf, Vec<String>>,
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
        let ui = ui_state::load(&ui_state_path);
        let mut service = service;
        service.apply_git_infos(&cached.infos);
        let mut app = App {
            config,
            service,
            icons,
            git_client,
            cache_path,
            ui_state_path,
            tab: ui.tab,
            cursor: 0,
            sort: ui.sort,
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
            list_offset: std::cell::Cell::new(0),
            auto_refresh: false,
            refreshed_tabs: HashSet::new(),
            files_missing: HashSet::new(),
            show_slugs: ui.show_slugs,
            changes_only: false,
            tab_focus: HashMap::new(),
            list_height: std::cell::Cell::new(1),
            preview: PreviewMode::from_key(&ui.preview),
            preview_log: HashMap::new(),
        };
        if let StartupStatus::Refresh { fetch } = startup
            && !app.config.example_mode
        {
            app.auto_refresh = true;
            // Honour fetch-on-start for a git tab; the Files tab does its
            // existence check via the first-visit hook below instead.
            if app.tab != Tab::FilesAndFolders {
                app.start_refresh(fetch);
            }
        }
        // The active tab counts as its first visit (e.g. the Files existence
        // check, or a git refresh that was skipped above).
        app.refresh_tab_on_first_visit();
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
        let mut indices = if self.filtering_active() {
            let subset: Vec<Repo> =
                tab_indices.iter().map(|&i| repos[i].clone()).collect();
            fuzzy_indices(&subset, &query)
                .into_iter()
                .map(|pos| tab_indices[pos])
                .collect()
        } else if self.is_sectioned() {
            // The Files tab groups entries by section (favourites floated
            // within each group).
            sections::flatten(&self.section_groups())
        } else {
            // The git tabs apply the chosen sort mode.
            let mut indices = tab_indices;
            sort_indices(
                repos,
                &mut indices,
                self.sort,
                Local::now().timestamp(),
            );
            indices
        };
        if self.changes_only {
            indices.retain(|&i| self.shows_change(&repos[i]));
        }
        indices
    }

    /// Whether `repo` passes the changes-only filter (see [`repo_has_change`]).
    fn shows_change(&self, repo: &Repo) -> bool {
        repo_has_change(repo, self.config.example_mode)
    }

    /// Whether the live fuzzy filter is currently narrowing the list.
    fn filtering_active(&self) -> bool {
        self.filtering && !self.filter.value().trim().is_empty()
    }

    /// Whether the current view groups entries into sections (the Files tab,
    /// when not live-filtering).
    fn is_sectioned(&self) -> bool {
        self.tab == Tab::FilesAndFolders && !self.filtering_active()
    }

    /// The display-ordered sections for the Files tab: entries grouped by
    /// section (in the stored section order), favourites first within each
    /// group, the rest in stored order, with Ungrouped last.
    fn section_groups(&self) -> Vec<sections::SectionGroup> {
        let repos = self.service.repos();
        let mut indices = self.tab_indices();
        // Stable: favourites float to the top of their section.
        indices.sort_by_key(|&i| !repos[i].fav);
        sections::group(self.service.sections(), &indices, |i| {
            repos[i].section.clone()
        })
    }

    /// Opens the jump-to-section picker for the Files tab.
    fn open_section_jump(&mut self) {
        let groups = self.section_groups();
        if groups.len() < 2 {
            self.set_status("no other sections");
            return;
        }
        let starts = sections::section_starts(&groups);
        let labels: Vec<String> =
            groups.iter().map(|g| g.label.clone()).collect();
        let current =
            sections::current_section(&starts, self.cursor).unwrap_or(0);
        self.overlay = Overlay::SectionJump(
            SelectModal::new("Jump to section", labels, current),
            starts,
        );
    }

    /// Moves the cursor to the start of the adjacent section (Ctrl+arrow).
    fn jump_section(&mut self, delta: isize) {
        let groups = self.section_groups();
        let starts = sections::section_starts(&groups);
        let dir = if delta < 0 {
            sections::SectionJump::Previous
        } else {
            sections::SectionJump::Next
        };
        if let Some(target) = sections::jump_target(&starts, self.cursor, dir) {
            self.cursor = target;
        }
    }

    /// Opens the manage-sections overlay at `cursor` over the current sections.
    fn open_sections_manager_at(&mut self, cursor: usize) {
        let names = self.service.sections().to_vec();
        self.overlay = Overlay::Sections(SectionsModal::new(names, cursor));
    }

    /// Opens the manage-sections overlay at the first section.
    fn open_sections_manager(&mut self) {
        self.open_sections_manager_at(0);
    }

    /// Runs a section-manager action, reporting errors and re-opening the
    /// manager (or a sub-prompt) afterwards.
    fn run_sections_action(
        &mut self,
        modal: SectionsModal,
        action: SectionsAction,
    ) {
        match action {
            SectionsAction::Pending => {
                self.overlay = Overlay::Sections(modal);
            }
            SectionsAction::Close => {}
            SectionsAction::New => {
                self.overlay = Overlay::SectionPrompt(
                    TextPrompt::new("New section", "name", ""),
                    SectionPromptKind::New,
                );
            }
            SectionsAction::Rename(old) => {
                self.overlay = Overlay::SectionPrompt(
                    TextPrompt::new("Rename section", "name", &old),
                    SectionPromptKind::Rename(old),
                );
            }
            SectionsAction::Delete(name) => {
                let message = format!(
                    "Delete section \"{name}\"? Entries become Ungrouped."
                );
                self.overlay = Overlay::SectionDelete(
                    ConfirmModal::new("Delete section", message),
                    name,
                );
            }
            SectionsAction::Move { from, to } => {
                if let Err(error) = self.service.move_section(from, to) {
                    self.set_status(format!("{error}"));
                }
                self.open_sections_manager_at(to);
            }
        }
    }

    /// Applies a submitted section prompt (new or rename) and re-opens the
    /// manager.
    fn submit_section_prompt(
        &mut self,
        kind: SectionPromptKind,
        value: String,
    ) {
        let result = match &kind {
            SectionPromptKind::New => self.service.add_section(&value),
            SectionPromptKind::Rename(old) => {
                self.service.rename_section(old, &value)
            }
        };
        if let Err(error) = result {
            self.set_status(format!("{error}"));
        }
        self.open_sections_manager();
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
        self.refreshed_tabs.insert(self.tab);
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
            // A tab switched to mid-refresh deferred its first-visit refresh;
            // run it now that the channel is free.
            self.refresh_tab_on_first_visit();
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
        app.ensure_preview_log();
        tui.terminal.draw(|frame| app.render(frame))?;
        app.drain_status();
        // Poll faster while refreshing so the spinner animates smoothly.
        let timeout = if app.is_refreshing() { 80 } else { 150 };
        if event::poll(Duration::from_millis(timeout))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && let Some(outcome) = app.handle_key(key)
        {
            match outcome {
                RunOutcome::LaunchGitToolInline(path) => {
                    app.run_git_inline(tui, &path)?;
                }
                other => return Ok(other),
            }
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
                FormResult::PickPath => self.open_form_path_picker(form, index),
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
            Overlay::SectionJump(mut modal, starts) => {
                match modal.handle_key(key) {
                    SelectResult::Selected(row) => {
                        if let Some(&pos) = starts.get(row) {
                            self.cursor = pos;
                        }
                    }
                    SelectResult::Cancel => {}
                    SelectResult::Pending => {
                        self.overlay = Overlay::SectionJump(modal, starts);
                    }
                }
            }
            Overlay::Sections(mut modal) => {
                let action = modal.handle_key(key);
                self.run_sections_action(modal, action);
            }
            Overlay::SectionPrompt(mut prompt, kind) => {
                match prompt.handle_key(key) {
                    PromptResult::Submit(value) => {
                        self.submit_section_prompt(kind, value);
                    }
                    PromptResult::Cancel => self.open_sections_manager(),
                    PromptResult::Pending => {
                        self.overlay = Overlay::SectionPrompt(prompt, kind);
                    }
                }
            }
            Overlay::SectionDelete(confirm, name) => {
                match confirm.handle_key(key) {
                    ConfirmResult::Yes => {
                        if let Err(error) = self.service.delete_section(&name) {
                            self.set_status(format!("{error}"));
                        }
                        self.open_sections_manager();
                    }
                    ConfirmResult::No => self.open_sections_manager(),
                    ConfirmResult::Pending => {
                        self.overlay = Overlay::SectionDelete(confirm, name);
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
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('1') => self.switch_tab(Tab::GitRepos),
            KeyCode::Char('2') => self.switch_tab(Tab::FilesAndFolders),
            KeyCode::Char('3') => self.switch_tab(Tab::Archive),
            KeyCode::Tab => self.cycle_tab(1),
            KeyCode::BackTab => self.cycle_tab(-1),
            KeyCode::Up => self.on_arrow(key, -1),
            KeyCode::Down => self.on_arrow(key, 1),
            KeyCode::Char('g') => self.cursor_to_edge(false),
            KeyCode::Char('G') => self.cursor_to_edge(true),
            KeyCode::PageUp => self.page(-1, false),
            KeyCode::PageDown => self.page(1, false),
            KeyCode::Char('u') if ctrl => self.page(-1, true),
            KeyCode::Char('d') if ctrl => self.page(1, true),
            KeyCode::Char('u') => self.undo(),
            KeyCode::Char(' ') => self.toggle_select(),
            KeyCode::Esc => self.clear_selection(),
            KeyCode::Enter => return self.open_selected(true),
            KeyCode::Char('l') => return self.open_git_inline(),
            KeyCode::Char('o') => return self.open_selected(false),
            KeyCode::Char('O') => return self.force_open_with(),
            KeyCode::Char('q') => return Some(RunOutcome::Quit),
            KeyCode::Char('f') => self.filtering = true,
            KeyCode::Char('F') => self.toggle_changes_only(),
            KeyCode::Char('s') if self.is_sectioned() => {
                self.open_section_jump()
            }
            KeyCode::Char('s') => self.cycle_sort(),
            KeyCode::Char('M') if self.tab == Tab::FilesAndFolders => {
                self.open_sections_manager()
            }
            KeyCode::Char('n') => self.open_add(),
            KeyCode::Char('e') => self.open_edit_form(),
            KeyCode::Char('d') | KeyCode::Delete | KeyCode::Backspace => {
                self.open_delete_confirm()
            }
            KeyCode::Char('z') => self.toggle_fav(),
            KeyCode::Char('y') => self.copy_path(),
            KeyCode::Char('b') => self.open_on_github(),
            KeyCode::Char('v') => self.cycle_preview(),
            KeyCode::Char('i') => self.toggle_slugs(),
            KeyCode::Char('A') => self.toggle_archive(),
            KeyCode::Char('S') => self.open_slug_prompt(),
            KeyCode::Char('p') => self.open_repair_picker(),
            KeyCode::Char('!') => self.open_error_list(),
            KeyCode::Char('r' | 'R') if self.tab == Tab::FilesAndFolders => {
                self.check_files_existence()
            }
            KeyCode::Char('r') => self.reload_status(false),
            KeyCode::Char('R') => self.reload_status(true),
            KeyCode::Char('x') => self.refresh_targets(false),
            KeyCode::Char('X') => self.refresh_targets(true),
            KeyCode::Char('?') => self.overlay = Overlay::Help,
            _ => {}
        }
        None
    }

    /// Routes an arrow key: `Alt` reorders, `Ctrl` jumps section-to-section (on
    /// the Files tab), `Shift` extends the selection, otherwise the cursor
    /// moves.
    fn on_arrow(&mut self, key: KeyEvent, delta: isize) {
        if key.modifiers.contains(KeyModifiers::ALT) {
            self.move_entry(delta);
        } else if key.modifiers.contains(KeyModifiers::CONTROL)
            && self.is_sectioned()
        {
            self.jump_section(delta);
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

    /// Switches to `tab`, remembering the current tab's cursor entry and
    /// restoring the target tab's, clearing the selection and persisting state.
    fn switch_tab(&mut self, tab: Tab) {
        if tab == self.tab {
            return;
        }
        self.remember_focus();
        self.tab = tab;
        self.clear_selection();
        self.restore_focus();
        self.save_ui_state();
        self.refresh_tab_on_first_visit();
    }

    /// Cycles to the next/previous tab (`Tab`/`Shift+Tab`).
    fn cycle_tab(&mut self, delta: isize) {
        let current = Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
        let next = navigation::cycle(current, Tab::ALL.len(), delta);
        self.switch_tab(Tab::ALL[next]);
    }

    /// Records the current tab's cursor entry by path, to restore on return.
    fn remember_focus(&mut self) {
        if let Some(index) = self.selected_index()
            && let Some(repo) = self.service.get(index)
        {
            let path = repo.path.clone();
            self.tab_focus.insert(self.tab, path);
        }
    }

    /// Restores the cursor to the remembered entry for the current tab (by
    /// path), or the top when none is remembered or it is no longer visible.
    fn restore_focus(&mut self) {
        self.cursor = 0;
        let Some(path) = self.tab_focus.get(&self.tab).cloned() else {
            return;
        };
        let view = self.ordered_view();
        let repos = self.service.repos();
        if let Some(pos) = view.iter().position(|&i| repos[i].path == path) {
            self.cursor = pos;
        }
    }

    /// Runs the per-tab first-visit work: the Files tab checks that its paths
    /// still exist; the git tabs refresh status (without fetching), mirroring
    /// the startup refresh of the initially active tab. Each runs once per
    /// session; a git refresh is deferred while another is in flight (and
    /// retried when it finishes) so switching tabs never aborts it.
    fn refresh_tab_on_first_visit(&mut self) {
        if self.refreshed_tabs.contains(&self.tab) {
            return;
        }
        if self.tab == Tab::FilesAndFolders {
            self.refreshed_tabs.insert(self.tab);
            if !self.config.example_mode {
                self.check_files_existence();
            }
            return;
        }
        if !self.auto_refresh || self.is_refreshing() {
            return;
        }
        self.start_refresh(false);
    }

    /// Cycles the sort mode and persists it for the next run.
    fn cycle_sort(&mut self) {
        self.sort = self.sort.next();
        self.save_ui_state();
    }

    /// Toggles the inline slug display and persists it for the next run.
    fn toggle_slugs(&mut self) {
        self.show_slugs = !self.show_slugs;
        self.save_ui_state();
    }

    /// Toggles the changes-only filter (git entries with a status change),
    /// keeping the cursor in range. Session-only (not persisted).
    fn toggle_changes_only(&mut self) {
        self.changes_only = !self.changes_only;
        self.clear_selection();
        let len = self.ordered_view().len();
        self.clamp_cursor(len);
        self.set_status(if self.changes_only {
            "showing changed repos only"
        } else {
            "showing all entries"
        });
    }

    /// Cycles the detail/preview panel (off -> right -> bottom) and persists it.
    fn cycle_preview(&mut self) {
        self.preview = self.preview.next();
        self.save_ui_state();
    }

    /// Persists the sort mode, active tab, slug display and preview mode.
    fn save_ui_state(&self) {
        let _ = ui_state::save(
            &self.ui_state_path,
            &ui_state::UiState {
                sort: self.sort,
                tab: self.tab,
                show_slugs: self.show_slugs,
                preview: self.preview.as_key().to_string(),
            },
        );
    }

    /// Moves the cursor cyclically within the current view; a plain move drops
    /// the range anchor so the next `Shift`-move re-anchors at the cursor.
    fn move_cursor(&mut self, delta: isize) {
        let len = self.ordered_view().len();
        self.cursor = navigation::cycle(self.cursor, len, delta);
        self.anchor = None;
    }

    /// Moves the cursor by `delta` without wrapping, clamped into the view.
    fn move_clamped(&mut self, delta: isize) {
        let len = self.ordered_view().len();
        if len == 0 {
            return;
        }
        let last = len as isize - 1;
        self.cursor = (self.cursor as isize + delta).clamp(0, last) as usize;
        self.anchor = None;
    }

    /// Jumps the cursor to the first (`g`) or last (`G`) entry.
    fn cursor_to_edge(&mut self, to_end: bool) {
        let len = self.ordered_view().len();
        self.cursor = if to_end { len.saturating_sub(1) } else { 0 };
        self.anchor = None;
    }

    /// Moves the cursor by whole (`pages` != 0) or half pages, using the last
    /// rendered list height.
    fn page(&mut self, pages: isize, half: bool) {
        let height = self.list_height.get().max(1) as isize;
        let step = if half { (height / 2).max(1) } else { height };
        self.move_clamped(pages.signum() * step);
    }

    /// Reverts the last config mutation, keeping the cursor in range.
    fn undo(&mut self) {
        match self.service.undo() {
            Ok(Some(label)) => {
                self.clear_selection();
                let len = self.ordered_view().len();
                self.clamp_cursor(len);
                self.set_status(format!("undid: {label}"));
            }
            Ok(None) => self.set_status("nothing to undo"),
            Err(error) => self.set_status(format!("undo failed: {error}")),
        }
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
    fn open_git_inline(&mut self) -> Option<RunOutcome> {
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
    fn run_git_inline(&mut self, tui: &mut Tui, dir: &Path) -> io::Result<()> {
        let Some(program) = self.config.git_program.clone() else {
            self.set_status("no git_program configured");
            return Ok(());
        };
        tui.suspended(|| {
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
    fn open_path_entry(
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
    fn force_open_with(&mut self) -> Option<RunOutcome> {
        let index = self.selected_index()?;
        let repo = self.service.get(index)?.clone();
        if let Err(error) = self.service.mark_used(index) {
            self.set_status(format!("could not record usage: {error}"));
        }
        Some(RunOutcome::OpenWith(repo.path))
    }

    /// Writes the selected-repo handoff file, surfacing any error.
    fn write_selected(&mut self, path: &std::path::Path) {
        if let Err(error) = self.service.write_selected(path) {
            self.set_status(format!("could not write selected path: {error}"));
        }
    }

    /// Starts adding an entry: the form opens directly with a kind guessed from
    /// the active tab. The path is a plain text field; `^O` opens the picker.
    fn open_add(&mut self) {
        let kind = match self.tab {
            Tab::GitRepos => RepoKind::Git,
            _ => RepoKind::Path,
        };
        let form = RepoForm::for_add("", kind, self.service.sections());
        self.overlay = Overlay::Form(form, None);
    }

    /// Opens the path picker to fill the path field of `form`, seeded near the
    /// path typed so far.
    fn open_form_path_picker(&mut self, form: RepoForm, index: Option<usize>) {
        let typed = form.path_value();
        let start = if typed.trim().is_empty() {
            crate::util::paths::home_dir().unwrap_or_else(|| PathBuf::from("/"))
        } else {
            crate::util::paths::expand_tilde(&typed)
        };
        self.overlay = Overlay::Picker(
            PathPicker::new(&start, true),
            PickerIntent::FormPath(Box::new(form), index),
        );
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
        let form = RepoForm::for_edit(repo, self.service.sections());
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
            if let Some(error) = self.path_error(repo) {
                labels.push(format!("{} - {error}", repo.display_name()));
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

    /// Opens the selected git entry's GitHub page in the browser (a non-blocking
    /// GUI handoff, so the TUI stays up).
    fn open_on_github(&mut self) {
        let name = self.selected_index().and_then(|index| {
            let repo = self.service.get(index)?;
            let info = if self.config.example_mode {
                repo.example_git_info.as_ref()
            } else {
                repo.git_info.as_ref()
            };
            info.and_then(|info| info.github_repo_name.clone())
        });
        let url = name.and_then(|name| {
            github_url(&name, self.config.github_username.as_deref())
        });
        let Some(url) = url else {
            self.set_status("no GitHub remote");
            return;
        };
        match crate::util::opener::open_url(&url) {
            Ok(_) => self.set_status(format!("opening {url}")),
            Err(error) => {
                self.set_status(format!("could not open browser: {error}"))
            }
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

    /// Toggles the favourite flag of the target entries (all on, else all off),
    /// keeping the cursor on the same entry even as favourites re-sort.
    fn toggle_fav(&mut self) {
        let targets = self.targets();
        if targets.is_empty() {
            return;
        }
        let focus = self.cursor_path();
        let all_fav = targets
            .iter()
            .all(|&i| self.service.get(i).is_some_and(|r| r.fav));
        if let Err(error) = self.service.set_fav_many(&targets, !all_fav) {
            self.set_status(format!("could not change favourite: {error}"));
        }
        self.clear_selection();
        self.refocus(focus);
    }

    /// The path of the entry under the cursor, if any.
    fn cursor_path(&self) -> Option<PathBuf> {
        self.selected_index()
            .and_then(|index| self.service.get(index))
            .map(|repo| repo.path.clone())
    }

    /// Moves the cursor onto the entry with `path`, if it is still visible.
    fn refocus(&mut self, path: Option<PathBuf>) {
        let Some(path) = path else {
            return;
        };
        let view = self.ordered_view();
        let repos = self.service.repos();
        if let Some(pos) = view.iter().position(|&i| repos[i].path == path) {
            self.cursor = pos;
        }
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
        // The Files tab reorders within a section regardless of sort mode; the
        // git tabs require the custom sort.
        if !self.is_sectioned() && self.sort != SortMode::Custom {
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
        // On the Files tab, only reorder within the same section.
        if self.is_sectioned() && repos[a].section != repos[b].section {
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

    /// Saves the add or edit form into a new or existing entry, registering a
    /// newly typed section name.
    fn do_save_form(&mut self, index: Option<usize>, draft: RepoDraft) {
        let path = crate::util::paths::expand_tilde(draft.path.trim());
        if draft.path.trim().is_empty() {
            self.set_status("path must not be empty");
            return;
        }
        let section = draft.section.clone();
        let kind = draft.kind;
        // A folder needs a trailing slash to be recognised before it exists.
        let assumed_file = kind == RepoKind::Path
            && !draft.path.trim().ends_with('/')
            && !path.exists();
        // A git entry refreshes its own status only when its path changed (or
        // when it is newly added); other edits never touch git.
        let path_changed = match index {
            Some(index) => {
                self.service.get(index).map(|r| &r.path) != Some(&path)
            }
            None => true,
        };
        let new_path = path.clone();
        let (result, ok_message) = match index {
            Some(index) => {
                let Some(mut repo) = self.service.get(index).cloned() else {
                    return;
                };
                apply_draft(&mut repo, draft, path);
                (self.service.update(index, repo), "entry updated")
            }
            None => {
                let mut repo = Repo::new(path.clone());
                apply_draft(&mut repo, draft, path);
                (self.service.add(repo), "entry added")
            }
        };
        let saved = result.is_ok();
        if saved && let Some(name) = section {
            let _ = self.service.ensure_section(&name);
        }
        self.report(result, ok_message);
        if saved && assumed_file {
            self.set_status(
                "no trailing / - treated as a file (end with / for a folder)",
            );
        }
        if saved
            && kind == RepoKind::Git
            && path_changed
            && !self.config.example_mode
        {
            self.refresh_paths(vec![new_path], false, false);
        }
    }

    /// Applies a picked path to its intent (repair an entry, or fill a form).
    fn do_picked(&mut self, intent: PickerIntent, path: PathBuf) {
        match intent {
            PickerIntent::Repair(index) => {
                let repaired = path.clone();
                match self.service.set_path(index, path) {
                    Ok(()) => {
                        self.set_status("path repaired");
                        if !self.config.example_mode {
                            self.clear_repaired_error(repaired);
                        }
                    }
                    Err(error) => {
                        self.set_status(format!("repair failed: {error}"))
                    }
                }
            }
            PickerIntent::FormPath(mut form, index) => {
                form.set_path(&path.to_string_lossy());
                self.overlay = Overlay::Form(*form, index);
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
    repo.section = draft.section;
    repo.kind = draft.kind;
    repo.fav = draft.fav;
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
                Constraint::Length(self.footer_height(area.width)),
            ])
            .split(area);
        self.render_header(frame, rows[0]);
        // One column of margin on each side of the list.
        let body = Rect {
            x: rows[1].x.saturating_add(1),
            width: rows[1].width.saturating_sub(2),
            ..rows[1]
        };
        let (list_area, preview_area) = self.split_preview(body);
        self.render_body(frame, list_area);
        if let Some(preview_area) = preview_area {
            self.render_preview(frame, preview_area);
        }
        self.render_footer(frame, rows[2]);
        self.render_overlay(frame, area);
    }

    /// Splits `body` into the list area and an optional preview area, per the
    /// active [`PreviewMode`] (right pane, bottom pane, or none).
    fn split_preview(&self, body: Rect) -> (Rect, Option<Rect>) {
        match self.preview {
            PreviewMode::Off => (body, None),
            PreviewMode::Right => {
                let parts = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Min(20),
                        Constraint::Percentage(40),
                    ])
                    .split(body);
                (parts[0], Some(parts[1]))
            }
            PreviewMode::Bottom => {
                let parts = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(3), Constraint::Length(9)])
                    .split(body);
                (parts[0], Some(parts[1]))
            }
        }
    }

    /// Caches the `git log` for the cursor entry when the preview is visible
    /// (once per path; skipped in example mode). Runs in the event loop, never
    /// during rendering.
    fn ensure_preview_log(&mut self) {
        if self.preview == PreviewMode::Off || self.config.example_mode {
            return;
        }
        let Some(index) = self.selected_index() else {
            return;
        };
        let Some(repo) = self.service.get(index) else {
            return;
        };
        if repo.kind != RepoKind::Git {
            return;
        }
        let path = repo.path.clone();
        if self.preview_log.contains_key(&path) {
            return;
        }
        let log = self.git_client.log(&path, 5);
        self.preview_log.insert(path, log);
    }

    /// Renders the detail/preview panel for the cursor entry.
    fn render_preview(&self, frame: &mut Frame, area: Rect) {
        let repo = self.selected_index().and_then(|i| self.service.get(i));
        let log = repo
            .map(|r| self.preview_log.get(&r.path).map(Vec::as_slice))
            .unwrap_or(None);
        preview::render(
            frame,
            area,
            preview::PreviewContext {
                repo,
                icons: &self.icons,
                example_mode: self.config.example_mode,
                log: log.unwrap_or(&[]),
            },
        );
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
        // Entry count, as shown/total when a filter narrows the list.
        let shown = self.ordered_view().len();
        let total = self.tab_indices().len();
        let count = if shown == total {
            format!("{} {shown}", icons.count)
        } else {
            format!("{} {shown}/{total}", icons.count)
        };
        spans.push(Span::styled(count, muted));
        spans.push(sep());
        spans.push(Span::styled(
            format!("{} {}", icons.sort, self.sort.label()),
            muted,
        ));
        // Active view lenses (filter / changes-only / slugs) in the accent.
        let mut lenses: Vec<&str> = Vec::new();
        if self.filtering_active() {
            lenses.push("filter");
        }
        if self.changes_only {
            lenses.push("changes");
        }
        if self.show_slugs {
            lenses.push("slugs");
        }
        if !lenses.is_empty() {
            spans.push(sep());
            spans.push(Span::styled(
                lenses.join(" · "),
                Style::default().fg(ACCENT),
            ));
        }

        // The status/remote times are git-specific, so skip them on the Files
        // and Folders tab.
        if self.config.example_mode {
            spans.push(sep());
            spans.push(Span::styled("example mode", muted));
        } else if self.tab != Tab::FilesAndFolders {
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

    /// The number of current-tab entries flagged with a path error.
    fn error_count(&self) -> usize {
        let repos = self.service.repos();
        self.tab_indices()
            .iter()
            .filter(|&&i| self.path_error(&repos[i]).is_some())
            .count()
    }

    /// The path error for `repo`, if any. A git entry reports a missing or
    /// invalid repository live; a file/folder entry only reports a missing path
    /// once the on-demand existence check (`r` on the Files tab) has flagged it.
    fn path_error(&self, repo: &Repo) -> Option<String> {
        if self.config.example_mode {
            // Example mode shows curated demo data, so a git entry's error comes
            // from its example info; path entries are never checked on disk.
            return match repo.kind {
                RepoKind::Git => repo.example_error(),
                RepoKind::Path => None,
            };
        }
        match repo.kind {
            RepoKind::Git => repo.entry_error(),
            RepoKind::Path => self
                .files_missing
                .contains(&repo.path)
                .then(|| "path not found".to_string()),
        }
    }

    /// Clears the error state of a just-repaired entry, whichever tab it lives
    /// on. The error list spans every tab, so a tab-wide refresh would miss an
    /// entry repaired from another tab: re-stat the file/folder entries (clears
    /// the missing marker) and refresh just the repaired path's git status.
    fn clear_repaired_error(&mut self, path: PathBuf) {
        self.recheck_files();
        self.refresh_paths(vec![path], false, false);
    }

    /// Re-stats every file/folder entry, recording the ones whose path is
    /// missing so the marker and error count reflect the current filesystem.
    fn recheck_files(&mut self) {
        self.files_missing = self
            .service
            .repos()
            .iter()
            .filter(|repo| repo.kind == RepoKind::Path && !repo.path.exists())
            .map(|repo| repo.path.clone())
            .collect();
    }

    /// Checks on disk which file/folder entries are missing, recording them so
    /// the marker and the error count reflect the result. Triggered by `r` on
    /// the Files tab; never on start. Reports a transient summary.
    fn check_files_existence(&mut self) {
        self.recheck_files();
        let missing = self.files_missing.len();
        self.set_status(if missing == 0 {
            "checked paths: all exist".to_string()
        } else if missing == 1 {
            "checked paths: 1 missing".to_string()
        } else {
            format!("checked paths: {missing} missing")
        });
    }

    /// Renders the entry table, or an empty hint.
    fn render_body(&self, frame: &mut Frame, area: Rect) {
        // Remember the page size for page-wise navigation.
        self.list_height
            .set(area.height.saturating_sub(1).max(1) as usize);
        let view = self.ordered_view();
        if view.is_empty() {
            render_empty_hint(frame, area, empty_hint(self.tab));
            return;
        }
        if self.is_sectioned() {
            self.render_sections(frame, area, view.len());
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
        let query = self.filter.value();
        let table_view = table::TableView {
            tab: self.tab,
            config: &self.config,
            icons: &self.icons,
            example_mode: self.config.example_mode,
            spinner,
            selected: &selected,
            has_selection: !self.selected.is_empty(),
            missing: &self.files_missing,
            show_slugs: self.show_slugs,
            query: self.filtering_active().then_some(query.as_str()),
        };
        table::render_table(frame, area, &visible, cursor, &table_view);
    }

    /// Renders the Files tab as a sectioned list (`view_len` entries total).
    fn render_sections(&self, frame: &mut Frame, area: Rect, view_len: usize) {
        let groups = self.section_groups();
        let cursor = self.cursor.min(view_len.saturating_sub(1));
        let view = sections_view::SectionedView {
            groups: &groups,
            repos: self.service.repos(),
            icons: &self.icons,
            selected: &self.selected,
            has_selection: !self.selected.is_empty(),
            missing: &self.files_missing,
            show_slugs: self.show_slugs,
            offset: &self.list_offset,
        };
        sections_view::render(frame, area, cursor, &view);
    }

    /// The footer height in rows: the filter and status states stay at two
    /// lines, the key hints take however many rows they wrap into at `width`.
    fn footer_height(&self, width: u16) -> u16 {
        if self.filtering || self.status_msg.is_some() {
            return 2;
        }
        let rows = footer_lines(&hints(self.tab), width).len();
        rows.max(1) as u16
    }

    /// Renders the footer: the filter line, a status message, or key hints.
    fn render_footer(&self, frame: &mut Frame, area: Rect) {
        if self.filtering {
            let mut spans =
                vec![Span::styled("filter: ", Style::default().fg(ACCENT))];
            spans.extend(self.filter.render_line(Style::default(), true).spans);
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
            Overlay::SectionJump(modal, _) => modal.render(frame, area),
            Overlay::Sections(modal) => modal.render(frame, area),
            Overlay::SectionPrompt(prompt, _) => prompt.render(frame, area),
            Overlay::SectionDelete(confirm, _) => confirm.render(frame, area),
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
    let mut hints: Vec<(&str, &str)> = vec![("Enter", "open")];
    // The git tool overlay only applies to git repositories.
    if tab == Tab::GitRepos {
        hints.push(("l", "lazygit"));
    }
    hints.extend([
        ("o", "cd"),
        ("O", "open in app"),
        ("Space", "select"),
        ("f", "filter"),
        ("F", "changes"),
    ]);
    // The Files tab groups into sections: s jumps, M manages them.
    if tab == Tab::FilesAndFolders {
        hints.push(("s", "section"));
        hints.push(("M", "sections"));
    } else {
        hints.push(("s", "sort"));
    }
    hints.push(("v", "preview"));
    hints.extend([("n", "add"), ("e", "edit"), ("d/Del", "del")]);
    hints.push(("u", "undo"));
    hints.push(("z", "fav"));
    // Archive tab restores; the others archive.
    hints.push(match tab {
        Tab::Archive => ("A", "restore"),
        _ => ("A", "archive"),
    });
    hints.push(("S", "slug"));
    hints.push(("i", "slugs"));
    hints.push(("y", "copy path"));
    hints.push(("b", "github"));
    hints.push(("p", "fix path"));
    // Git status refresh only makes sense where entries are git repositories;
    // the Files tab uses `r` to check that paths still exist.
    match tab {
        Tab::GitRepos => hints.push(("x/r", "refresh")),
        Tab::FilesAndFolders => hints.push(("r", "check paths")),
        Tab::Archive => {}
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

/// Whether `repo` passes the changes-only filter: non-git entries always pass;
/// a git entry passes only when its (live or example) status is not clean.
fn repo_has_change(repo: &Repo, example_mode: bool) -> bool {
    if repo.kind != RepoKind::Git {
        return true;
    }
    let info = if example_mode {
        repo.example_git_info.as_ref()
    } else {
        repo.git_info.as_ref()
    };
    info.is_some_and(|info| !info.is_clean())
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
        fn log(&self, _path: &Path, _max: usize) -> Vec<String> {
            Vec::new()
        }
    }

    fn sample_app() -> App {
        let mut git = Repo::new(PathBuf::from("/code/hop"));
        git.name = Some("hop".to_string());
        git.fav = true;
        let mut missing = Repo::new(PathBuf::from("/code/gone"));
        missing.slug = Some("gone".to_string());
        let mut folder = Repo::new(PathBuf::from("/notes"));
        folder.kind = RepoKind::Path;
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
    fn footer_grows_to_fit_all_hints() {
        // At a narrow width the hints wrap past two rows; the footer area must
        // size to hold them all rather than clipping at a fixed height.
        let app = sample_app();
        let width = 60;
        let expected = footer_lines(&hints(app.tab), width).len() as u16;
        assert!(expected > 2, "test width should force wrapping");
        assert_eq!(app.footer_height(width), expected);

        let mut terminal = Terminal::new(TestBackend::new(width, 40)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        let rendered = terminal.backend().buffer().clone();
        let text: String = rendered
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        // The last hint must be present, i.e. nothing was clipped away.
        assert!(text.contains("quit"), "last footer hint was clipped");
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
    fn pressing_l_on_a_git_repo_opens_the_tool_inline() {
        let mut app = sample_app();
        // The first git-tab entry is the git repo "hop".
        let outcome = app
            .handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
        assert!(matches!(outcome, Some(RunOutcome::LaunchGitToolInline(_))));
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

    #[test]
    fn relative_age_picks_the_largest_unit() {
        assert_eq!(relative_age(chrono::Duration::days(3)), "3d");
        assert_eq!(relative_age(chrono::Duration::hours(5)), "5h");
        assert_eq!(relative_age(chrono::Duration::minutes(12)), "12m");
        assert_eq!(relative_age(chrono::Duration::seconds(-30)), "0m");
    }

    #[test]
    fn changes_filter_keeps_non_git_and_dirty_git() {
        use crate::domain::repo::GitInfo;
        // A non-git entry always passes.
        let mut folder = Repo::new(PathBuf::from("/notes"));
        folder.kind = RepoKind::Path;
        assert!(repo_has_change(&folder, false));

        // A git entry with no info or a clean tree is filtered out.
        let mut clean = Repo::new(PathBuf::from("/clean"));
        assert!(!repo_has_change(&clean, false));
        clean.git_info = Some(GitInfo {
            valid: true,
            ..GitInfo::default()
        });
        assert!(!repo_has_change(&clean, false));

        // A git entry with changes passes.
        let mut dirty = Repo::new(PathBuf::from("/dirty"));
        dirty.git_info = Some(GitInfo {
            valid: true,
            changes: Some(2),
            ..GitInfo::default()
        });
        assert!(repo_has_change(&dirty, false));
    }
}
