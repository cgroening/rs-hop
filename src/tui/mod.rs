//! TUI front end: the three-tab list view, its key handling and overlays.
//!
//! [`App`] owns the resolved config, the [`RepoService`] and the view state
//! (active tab, cursor, sort, live filter and the current overlay). [`run`]
//! draws, polls events on a short timeout and drains background git-status
//! updates so the list shows immediately and fills in as `git` reports. Opening
//! an entry returns a [`RunOutcome`] the composition root acts on after the
//! terminal is restored.

pub mod appframe;
pub mod bindings;
pub mod columns;
pub mod detail;
pub mod form;
pub mod help;
pub mod navigation;
pub mod path_picker;
pub mod presentation;
pub mod preview;
pub mod scan_picker;
pub mod scroll;
pub mod sections_modal;
pub mod sections_view;
pub mod skin;
pub mod table;
pub mod terminal;
pub mod text_input;
pub mod widgets;

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

pub use terminal::{Tui, TuiEvent};

use ratada::quit::{self, QuitConfirm, QuitKind};
use ratada::shortcut_hints;

use crate::config::Config;
use crate::domain::backup;
use crate::domain::filter::{Tab, belongs_to_tab, fuzzy_indices};
use crate::domain::path_repair::nearest_existing_on_disk;
use crate::domain::repo::{self, Repo, RepoKind};
use crate::domain::sections;
use crate::domain::sort::{
    SortContext, SortDir, SortMode, StatsLookup, sort_indices,
};
use crate::keymap::{Action, Keymap};
use crate::service::preview_service::{self, PreviewLog};
use crate::service::repo_service::RepoService;
use crate::service::stats_service::{
    CodeUpdate, GitStatsUpdate, spawn_code_stats, spawn_git_stats,
};
use crate::service::status_service::{StatusUpdate, spawn_refresh};
use crate::service::zip_service::{ZipJob, ZipUpdate, spawn_zip};
use crate::storage::cache;
use crate::storage::git_client::GitClient;
use crate::storage::stats_cache::{self, StatsCache};
use crate::storage::ui_state;
use crate::theme::Skin;
use crate::tui::columns::ColumnSet;
use crate::tui::form::{FormResult, RepoDraft, RepoForm};
use crate::tui::path_picker::{PathPicker, PickerResult};
use crate::tui::presentation::{IconSet, github_url, render_empty_hint};
use crate::tui::preview::PreviewLayout;
use crate::tui::sections_modal::{SectionsAction, SectionsModal};
use crate::tui::skin::Colors;
use crate::tui::widgets::{
    ConfirmModal, ConfirmResult, PromptResult, SelectModal, SelectResult,
    TextPrompt,
};
use crate::util::opener::launch_git_tool;
use crate::util::paths::expand_tilde;

/// How long a transient status message stays visible.
const STATUS_TTL: Duration = Duration::from_secs(4);

/// Progress-bar label while a background status refresh runs.
const REFRESH_LABEL: &str = "refreshing";

/// Progress-bar label while a background ZIP backup runs.
const ZIP_LABEL: &str = "zipping";

/// Padding width of the percentage in the progress text, so `XX %` keeps a
/// constant width from `0` through `100`.
const PERCENT_WIDTH: usize = 3;

/// Separator between the percentage/counts prefix and the entry name.
const PROGRESS_SEPARATOR: &str = " - ";

/// How long the cursor must rest on an entry before its preview `git log` is
/// fetched, so quick scrolling does not spawn a fetch per row.
const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(120);

/// How many `git log` lines the preview shows.
const PREVIEW_LOG_LINES: usize = 5;

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
    /// The sort picker; the vec maps rows to modes.
    Sort(SelectModal, Vec<SortMode>),
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
    /// The resolved skin (config-driven palette + glyphs) for the panel frame.
    skin: Skin,
    /// The colour roles resolved once from the active theme.
    colors: Colors,
    /// The resolved key bindings, built once so `[keys]` warnings are logged
    /// once and dispatch never rebuilds the map per key press.
    keymap: Keymap,
    git_client: Arc<dyn GitClient>,
    cache_path: PathBuf,
    /// Path of the ZIP-backup fingerprint cache (in the state directory).
    zip_cache_path: PathBuf,
    ui_state_path: PathBuf,
    tab: Tab,
    cursor: usize,
    sort: SortMode,
    sort_dir: SortDir,
    filtering: bool,
    filter: text_input::TextInput,
    overlay: Overlay,
    /// The help overlay's scroll position, kept across frames.
    help_scroll: scroll::Scroll,
    status_msg: Option<(String, Instant)>,
    loading: Option<(usize, usize)>,
    /// The progress-bar label for the current `loading` operation (a refresh or
    /// a ZIP backup).
    loading_label: &'static str,
    /// The entry currently being processed, shown in the progress-bar text.
    loading_detail: Option<String>,
    /// Display width of the longest entry name in the current `loading` run, so
    /// the progress text can reserve a stable block and pin the `XX %` column.
    loading_name_width: usize,
    /// Receives background ZIP-backup progress, drained each loop.
    zip_rx: Option<Receiver<ZipUpdate>>,
    /// Last ZIP-backup time per repo path, shown in the "ZIP Backup" column.
    /// Loaded once at start and after each backup (no per-frame filesystem I/O).
    zip_backups: HashMap<PathBuf, DateTime<Local>>,
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
    /// cursor pages within the viewport.
    list_offset: std::cell::Cell<usize>,
    /// Scroll offset of the table view (git tabs, Archive, filtered Files),
    /// kept across frames so the cursor only scrolls once it reaches an edge.
    table_offset: std::cell::Cell<usize>,
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
    /// Where the detail panel sits and how big it is (persisted).
    preview: PreviewLayout,
    /// The detail panel's scroll position, kept across frames.
    preview_scroll: scroll::Scroll,
    /// Which columns the table shows (persisted).
    columns: ColumnSet,
    /// Path of the statistics cache in the state directory.
    stats_path: PathBuf,
    /// Cached code and history statistics, seeded from disk at start.
    stats: StatsCache,
    /// Receives background code statistics, drained each loop.
    code_rx: Option<Receiver<CodeUpdate>>,
    /// Receives background history statistics, drained each loop.
    git_stats_rx: Option<Receiver<GitStatsUpdate>>,
    /// Paths a statistics worker has not reported yet (drive the cell spinner).
    computing: HashSet<PathBuf>,
    /// Cached `git log` excerpts for the preview, keyed by entry path.
    preview_log: HashMap<PathBuf, Vec<String>>,
    /// Sender handed to the background log workers; kept alive so `preview_rx`
    /// never disconnects.
    preview_tx: Sender<PreviewLog>,
    /// Receives background preview-log results, drained each loop.
    preview_rx: Receiver<PreviewLog>,
    /// Paths whose preview log is being fetched in the background (dedupe).
    preview_pending: HashSet<PathBuf>,
    /// The cursor path the preview is debouncing, with when it became current.
    preview_target: Option<PathBuf>,
    /// When `preview_target` last changed, for the debounce window.
    preview_target_at: Instant,
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
        let icons = IconSet::new(config.appearance.glyphs);
        let skin = config.skin();
        let colors = Colors::from_palette(&skin.palette);
        let keymap = config.keymap();
        let cached = cache::load(&cache_path);
        let ui = ui_state::load(&ui_state_path);
        shortcut_hints::set_visible(ui.hints_visible);
        install_quit_confirmation(config.confirm_quit, skin);
        // The ZIP cache lives next to the git-info cache in the state directory.
        let state_dir = cache_path.parent().unwrap_or_else(|| Path::new("."));
        let zip_cache_path = state_dir.join("zip-manifests.toml");
        let stats_path = state_dir.join("stats-cache.toml");
        let stats = stats_cache::load(&stats_path);
        let mut service = service;
        service.apply_git_infos(&cached.infos);
        let (preview_tx, preview_rx) = mpsc::channel();
        let mut app = App {
            config,
            service,
            icons,
            skin,
            colors,
            keymap,
            git_client,
            cache_path,
            zip_cache_path,
            ui_state_path,
            tab: ui.tab,
            cursor: 0,
            sort: ui.sort,
            sort_dir: ui.sort_dir,
            filtering: false,
            filter: text_input::TextInput::new(""),
            overlay: Overlay::None,
            help_scroll: scroll::Scroll::default(),
            status_msg: None,
            loading: None,
            loading_label: REFRESH_LABEL,
            loading_detail: None,
            loading_name_width: 0,
            zip_rx: None,
            zip_backups: HashMap::new(),
            cache_generated_at: cached.generated_at,
            last_fetched: cached.fetched_at,
            refresh_fetched: false,
            status_rx: None,
            refreshing: HashSet::new(),
            refresh_started: Instant::now(),
            selected: HashSet::new(),
            anchor: None,
            list_offset: std::cell::Cell::new(0),
            table_offset: std::cell::Cell::new(0),
            auto_refresh: false,
            refreshed_tabs: HashSet::new(),
            files_missing: HashSet::new(),
            show_slugs: ui.show_slugs,
            changes_only: false,
            tab_focus: HashMap::new(),
            list_height: std::cell::Cell::new(1),
            preview: PreviewLayout::from_state(
                &ui.preview,
                ui.preview_width_pct,
                ui.preview_height_rows,
            ),
            preview_scroll: scroll::Scroll::default(),
            columns: ColumnSet::from_key(&ui.columns).available_on(ui.tab),
            stats_path,
            stats,
            code_rx: None,
            git_stats_rx: None,
            computing: HashSet::new(),
            preview_log: HashMap::new(),
            preview_tx,
            preview_rx,
            preview_pending: HashSet::new(),
            preview_target: None,
            preview_target_at: Instant::now(),
        };
        app.reload_zip_backups();
        app.start_stats();
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
            sort_indices(repos, &mut indices, &self.sort_context());
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
        self.loading_detail = None;
        self.loading_name_width = self.max_name_width(&paths);
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
                Ok(StatusUpdate::Started { path }) => {
                    self.loading_detail = Some(self.name_for_path(&path));
                }
                Ok(StatusUpdate::Done { path, info }) => {
                    self.service.set_git_info(&path, info);
                    self.refreshing.remove(&path);
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
            self.loading_detail = None;
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

    /// The display name of the entry at `path`, or its basename as a fallback.
    fn name_for_path(&self, path: &Path) -> String {
        self.service
            .repos()
            .iter()
            .find(|repo| repo.path == path)
            .map_or_else(|| repo::basename(path), Repo::display_name)
    }

    /// The widest display name among `paths`, used to reserve a stable block in
    /// the progress text so the `XX %` column does not move as names change.
    fn max_name_width(&self, paths: &[PathBuf]) -> usize {
        paths
            .iter()
            .map(|path| {
                UnicodeWidthStr::width(self.name_for_path(path).as_str())
            })
            .max()
            .unwrap_or(0)
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
        app.drain_status();
        app.drain_zip();
        app.drain_preview();
        app.drain_code_stats();
        app.drain_git_stats();
        app.request_preview_log();
        tui.draw(|frame| app.render(frame))?;
        // Poll faster while a refresh or backup runs (progress bar) or while a
        // preview log is being debounced/fetched, so updates land promptly.
        let timeout =
            if app.is_refreshing() || app.is_zipping() || app.is_computing() {
                80
            } else if app.preview_busy() {
                60
            } else {
                150
            };
        let outcome = match tui.poll_event(Duration::from_millis(timeout))? {
            // The hard `Ctrl+Q` is the escape hatch and never asks.
            Some(TuiEvent::Quit) => Some(RunOutcome::Quit),
            Some(TuiEvent::Key(key)) => match app.handle_key(key) {
                // A soft quit (`q`) goes through the confirmation policy;
                // `handle_key` has no `Tui`, so the loop asks.
                Some(RunOutcome::Quit) => {
                    let repaint = |frame: &mut Frame| app.render(frame);
                    quit::request(tui, QuitKind::Soft, &repaint)
                        .then_some(RunOutcome::Quit)
                }
                other => other,
            },
            Some(TuiEvent::Resize) | None => None,
        };
        if let Some(outcome) = outcome {
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

/// Wires the toolkit's quit confirmation: `q` asks when `confirm_quit` is set,
/// `Ctrl+Q` never does. `skin` is `Copy` and hop never re-themes at runtime, so
/// the guard can hold it.
fn install_quit_confirmation(confirm_quit: bool, skin: Skin) {
    quit::set_confirm(if confirm_quit {
        QuitConfirm::Soft
    } else {
        QuitConfirm::Never
    });
    quit::set_guard(move |tui, _kind, bg| {
        ratada::modal::confirm(tui, &skin, " Quit hop? ", bg)
    });
}

/// The blank cell (or row) between the list and the detail panel.
const PANEL_GUTTER: u16 = 1;
/// The narrowest list a side panel may squeeze the body down to, in columns.
const MIN_LIST_COLS: u16 = 20;
/// The shortest list a bottom panel may squeeze the body down to, in rows.
/// A column count would leave no room for the panel at all on a normal screen.
const MIN_LIST_ROWS: u16 = 3;

/// The label of the group and help section listing the app-wide chords.
const GLOBAL_GROUP: &str = "Global";

/// The app-wide chords: this app's own keys, resolved through the keymap so a
/// `[keys]` override shows up here too, followed by the ones the toolkit
/// intercepts itself (the hints toggle and the hard quit).
fn global_group(keymap: &Keymap) -> (String, Vec<(String, String)>) {
    let mut hints = keymap.hints(&[Action::Help, Action::Quit]);
    hints.extend(shortcut_hints::global_bindings());
    (GLOBAL_GROUP.to_string(), hints)
}

impl App {
    /// Handles a key, returning an outcome when the loop should end.
    ///
    /// The toolkit's hints toggle is consumed first, so it works in every
    /// state; `Ctrl+Q` never reaches here (the `Tui` turns it into
    /// [`TuiEvent::Quit`]).
    fn handle_key(&mut self, key: KeyEvent) -> Option<RunOutcome> {
        if shortcut_hints::consume_toggle(key) {
            self.save_ui_state();
            return None;
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
                // `?`/`Esc` close it; everything else keeps it open, and the
                // movement keys scroll the (taller than the screen) list.
                if !matches!(key.code, KeyCode::Char('?') | KeyCode::Esc) {
                    self.help_scroll.handle_key(key);
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
            Overlay::Sort(mut modal, modes) => match modal.handle_key(key) {
                SelectResult::Selected(row) => {
                    if let Some(&mode) = modes.get(row) {
                        self.apply_sort(mode);
                    }
                }
                SelectResult::Cancel => {}
                SelectResult::Pending => {
                    self.overlay = Overlay::Sort(modal, modes);
                }
            },
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
    ///
    /// Keys the keymap cannot express are consumed first (see
    /// [`App::handle_untracked_key`]); everything else resolves to an
    /// [`Action`], so a `[keys]` override actually rebinds it.
    fn handle_list_key(&mut self, key: KeyEvent) -> Option<RunOutcome> {
        if self.filtering {
            return self.handle_filter_key(key);
        }
        if self.handle_untracked_key(key) {
            return None;
        }
        let action = self.keymap.action_for(&key)?;
        self.run_action(action)
    }

    /// Handles the keys that have no [`Action`], returning whether one matched.
    ///
    /// `Shift`+arrow cannot be a binding because [`KeyChord`](crate::keymap::
    /// KeyChord) ignores the shift modifier, so it has to be caught before the
    /// keymap turns a shifted arrow into a plain cursor move. Tab cycling and
    /// `Esc` are structural, not user-facing actions.
    fn handle_untracked_key(&mut self, key: KeyEvent) -> bool {
        // Only a bare `Shift`+arrow extends: `Ctrl`/`Alt`+arrow are real chords
        // the keymap owns, and they keep winning as they did before.
        let shift = key.modifiers == KeyModifiers::SHIFT;
        match key.code {
            KeyCode::Up if shift => self.extend_selection(-1),
            KeyCode::Down if shift => self.extend_selection(1),
            KeyCode::Tab => self.cycle_tab(1),
            KeyCode::BackTab => self.cycle_tab(-1),
            KeyCode::Esc => self.clear_selection(),
            _ => return false,
        }
        true
    }

    /// Runs `action` on the list view, returning an outcome when the loop should
    /// end.
    ///
    /// The context-dependent actions branch on the active tab here rather than
    /// on the key, so a rebound key keeps its meaning: `Sort` jumps to a section
    /// where the list is sectioned, `Reload`/`ReloadFetch` re-check paths on the
    /// Files tab, and `ManageSections` only applies there.
    fn run_action(&mut self, action: Action) -> Option<RunOutcome> {
        match action {
            Action::Up => self.move_cursor(-1),
            Action::Down => self.move_cursor(1),
            Action::Top => self.cursor_to_edge(false),
            Action::Bottom => self.cursor_to_edge(true),
            Action::PageUp => self.page(-1, false),
            Action::PageDown => self.page(1, false),
            Action::HalfPageUp => self.page(-1, true),
            Action::HalfPageDown => self.page(1, true),
            Action::TabGit => self.switch_tab(Tab::GitRepos),
            Action::TabFiles => self.switch_tab(Tab::FilesAndFolders),
            Action::TabArchive => self.switch_tab(Tab::Archive),
            Action::ToggleSelect => self.toggle_select(),
            Action::Jump | Action::JumpCd => return self.open_selected(false),
            Action::Open => return self.open_selected(true),
            Action::GitTool => return self.open_git_inline(),
            Action::OpenApp => return self.force_open_with(),
            Action::Filter => self.filtering = true,
            Action::ChangesFilter => self.toggle_changes_only(),
            Action::Github => self.open_on_github(),
            Action::Preview => self.toggle_preview(),
            Action::PreviewPosition => self.flip_preview_position(),
            Action::PreviewScrollUp => self.scroll_preview(-1),
            Action::PreviewScrollDown => self.scroll_preview(1),
            Action::PreviewShrink => self.resize_preview(-1),
            Action::PreviewGrow => self.resize_preview(1),
            Action::Columns => self.cycle_columns(),
            Action::Sort => self.open_sort_picker(),
            Action::SectionJump if self.is_sectioned() => {
                self.open_section_jump();
            }
            Action::SectionJump => {}
            Action::ManageSections if self.tab == Tab::FilesAndFolders => {
                self.open_sections_manager();
            }
            Action::ManageSections => {}
            Action::ReorderUp => self.move_entry(-1),
            Action::ReorderDown => self.move_entry(1),
            Action::Add => self.open_add(),
            Action::Edit => self.open_edit_form(),
            Action::Delete => self.open_delete_confirm(),
            Action::Undo => self.undo(),
            Action::ToggleFav => self.toggle_fav(),
            Action::Zip => self.zip_targets(),
            Action::ZipAll => self.zip_all(),
            Action::Archive => self.toggle_archive(),
            Action::Slug => self.open_slug_prompt(),
            Action::ToggleSlugs => self.toggle_slugs(),
            Action::CopyPath => self.copy_path(),
            Action::RepairPath => self.open_repair_picker(),
            Action::Errors => self.open_error_list(),
            Action::Reload | Action::ReloadFetch
                if self.tab == Tab::FilesAndFolders =>
            {
                self.check_files_existence();
            }
            Action::Reload => self.reload_status(false),
            Action::ReloadFetch => self.reload_status(true),
            Action::RefreshOne => self.refresh_targets(false),
            Action::RefreshOneFetch => self.refresh_targets(true),
            Action::Help => {
                self.help_scroll.reset();
                self.overlay = Overlay::Help;
            }
            Action::Quit => return Some(RunOutcome::Quit),
        }
        None
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
            KeyCode::Enter => return self.open_selected(false),
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
        // Each tab is a distinct list; drop the previous tab's scroll offset.
        self.list_offset.set(0);
        self.table_offset.set(0);
        self.restore_focus();
        self.columns = self.columns.available_on(tab);
        self.preview_scroll.reset();
        self.save_ui_state();
        self.start_stats();
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

    /// Everything a sort needs, borrowed from the statistics caches.
    fn sort_context(&self) -> SortContext<'_> {
        SortContext {
            mode: self.sort,
            dir: self.sort_dir,
            now: Local::now().timestamp(),
            stats: StatsLookup {
                code: &self.stats.code,
                git: &self.stats.git,
            },
        }
    }

    /// The modes the sort picker offers: the four general ones, then the
    /// columns of the active set - so a user only sorts by what is on screen.
    fn sort_modes(&self) -> Vec<SortMode> {
        let mut modes = vec![
            SortMode::Name,
            SortMode::Recent,
            SortMode::Frecency,
            SortMode::Custom,
        ];
        modes.extend_from_slice(self.columns.sort_modes());
        modes
    }

    /// Opens the sort picker, with the cursor on the active mode.
    fn open_sort_picker(&mut self) {
        let modes = self.sort_modes();
        let cursor = modes.iter().position(|m| *m == self.sort).unwrap_or(0);
        let items: Vec<String> = modes
            .iter()
            .map(|mode| {
                if *mode == self.sort {
                    format!("{}  {}", mode.title(), self.sort_dir.arrow())
                } else {
                    mode.title().to_string()
                }
            })
            .collect();
        self.overlay =
            Overlay::Sort(SelectModal::new(" Sort by ", items, cursor), modes);
    }

    /// Applies a picked sort mode. Re-picking the active column flips the
    /// direction; a fresh statistics column starts descending, because "which
    /// is the biggest" is the question it answers.
    fn apply_sort(&mut self, mode: SortMode) {
        if mode == self.sort {
            self.sort_dir = self.sort_dir.flip();
        } else if mode.is_statistic() {
            self.sort_dir = SortDir::Desc;
        } else {
            self.sort_dir = SortDir::Asc;
        }
        self.sort = mode;
        self.save_ui_state();
    }

    /// Cycles the table's column set and starts the worker the new set needs.
    fn cycle_columns(&mut self) {
        self.columns = self.columns.next(self.tab);
        self.save_ui_state();
        self.start_stats();
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

    /// Shows or hides the detail panel and persists the choice.
    fn toggle_preview(&mut self) {
        self.preview.toggle();
        self.preview_scroll.reset();
        self.save_ui_state();
    }

    /// Moves the detail panel to the other side.
    fn flip_preview_position(&mut self) {
        self.preview.flip_position();
        self.save_ui_state();
    }

    /// Grows or shrinks the detail panel along its current axis.
    fn resize_preview(&mut self, step: i16) {
        if !self.preview.visible {
            return;
        }
        self.preview.resize(step);
        self.save_ui_state();
    }

    /// Scrolls the detail panel.
    fn scroll_preview(&mut self, delta: i32) {
        if self.preview.visible {
            self.preview_scroll.scroll_by(delta);
        }
    }

    /// Persists the sort mode, active tab, slug display, preview mode and
    /// whether the hint footer is shown.
    fn save_ui_state(&self) {
        let _ = ui_state::save(
            &self.ui_state_path,
            &ui_state::UiState {
                sort: self.sort,
                sort_dir: self.sort_dir,
                tab: self.tab,
                show_slugs: self.show_slugs,
                preview: self.preview.as_key().to_string(),
                preview_width_pct: self.preview.width_pct,
                preview_height_rows: self.preview.height_rows,
                columns: self.columns.as_key().to_string(),
                hints_visible: shortcut_hints::visible(),
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

    /// The paths of the target entries (the selection, or the cursor entry when
    /// nothing is selected), in list order.
    fn target_paths(&self) -> Vec<String> {
        self.targets()
            .into_iter()
            .filter_map(|index| self.service.get(index))
            .map(|repo| repo.path.to_string_lossy().into_owned())
            .collect()
    }

    /// Opens the selected git entry's GitHub page in the browser (a non-blocking
    /// GUI handoff, so the TUI stays up).
    fn open_on_github(&mut self) {
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
    fn github_url_for(&self, index: usize) -> Option<String> {
        let repo = self.service.get(index)?;
        let info = if self.config.example_mode {
            repo.example_git_info.as_ref()
        } else {
            repo.git_info.as_ref()
        };
        let name = info.and_then(|info| info.github_repo_name.clone())?;
        github_url(&name, self.config.github_username.as_deref())
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

    /// Zips the target git entries (selection or cursor) into the backup folder.
    /// Non-git entries are ignored; a lone non-git cursor reports a hint.
    fn zip_targets(&mut self) {
        // An explicit single/selection backup ignores the include flag: the
        // user is targeting these entries on purpose. Git repos and folders
        // both qualify; missing/non-folder paths are dropped in `start_zip`.
        let targets = self.targets();
        if targets.is_empty() {
            self.set_status("nothing to zip");
            return;
        }
        self.start_zip(&targets);
        self.clear_selection();
    }

    /// Zips every entry (across all tabs) that opts into the "backup all" run
    /// into the backup folder: all git repos plus the file/folder entries whose
    /// backup toggle is on.
    fn zip_all(&mut self) {
        let indices: Vec<usize> = self
            .service
            .repos()
            .iter()
            .enumerate()
            .filter(|(_, repo)| repo.include_in_backup)
            .map(|(index, _)| index)
            .collect();
        if indices.is_empty() {
            self.set_status("nothing to zip");
            return;
        }
        self.start_zip(&indices);
    }

    /// Starts a background ZIP backup of the repos at `indices`, showing the
    /// progress bar. Refuses to start while another refresh or backup runs.
    fn start_zip(&mut self, indices: &[usize]) {
        if self.loading.is_some() {
            self.set_status("busy: a refresh or backup is running");
            return;
        }
        let Some(folder) = self.config.zip_backup_folder.as_deref() else {
            self.set_status("no zip_backup_folder configured");
            return;
        };
        let folder = expand_tilde(folder);
        if let Err(error) = std::fs::create_dir_all(&folder) {
            self.set_status(format!("could not create backup folder: {error}"));
            return;
        }
        let repos = self.service.repos();
        let jobs: Vec<ZipJob> = indices
            .iter()
            .filter_map(|&i| repos.get(i))
            // Skip entries whose directory is gone (broken or offline drives),
            // so no empty archive is written for them.
            .filter(|repo| repo.path.is_dir())
            .map(|repo| ZipJob {
                src: repo.path.clone(),
                dest: backup_dest(&folder, repo, repos),
                name: repo.display_name(),
            })
            .collect();
        if jobs.is_empty() {
            self.set_status("nothing to zip (paths missing?)");
            return;
        }
        let count = jobs.len();
        self.set_status(if count == 1 {
            "creating backup…".to_string()
        } else {
            format!("creating {count} backups…")
        });
        self.loading_name_width = jobs
            .iter()
            .map(|job| UnicodeWidthStr::width(job.name.as_str()))
            .max()
            .unwrap_or(0);
        self.loading = Some((0, 0));
        self.loading_label = ZIP_LABEL;
        self.loading_detail = None;
        self.zip_rx = Some(spawn_zip(
            jobs,
            self.config.zip_exclude_dirs.clone(),
            self.zip_cache_path.clone(),
        ));
    }

    /// Applies any pending background ZIP-backup progress without blocking.
    /// Starts the worker the active column set needs, over the paths currently
    /// shown. `Standard` starts nothing at all, so a user who never opens the
    /// statistics never pays for a source walk.
    ///
    /// In example mode no worker ever runs; the cells fall back to a dash
    /// rather than spinning forever.
    fn start_stats(&mut self) {
        self.code_rx = None;
        self.git_stats_rx = None;
        self.computing.clear();
        if !self.columns.is_statistics() || self.config.example_mode {
            return;
        }
        let paths: Vec<PathBuf> = self
            .ordered_view()
            .iter()
            .filter_map(|&i| self.service.get(i))
            .map(|repo| repo.path.clone())
            .collect();
        if paths.is_empty() {
            return;
        }
        self.computing = paths.iter().cloned().collect();
        if self.columns.needs_code_stats() {
            self.code_rx = Some(spawn_code_stats(
                paths,
                self.config.zip_exclude_dirs.clone(),
            ));
        } else if self.columns.needs_git_stats() {
            self.git_stats_rx =
                Some(spawn_git_stats(Arc::clone(&self.git_client), paths));
        }
    }

    /// Drains the background code statistics into the cache.
    fn drain_code_stats(&mut self) {
        let Some(rx) = self.code_rx.take() else {
            return;
        };
        loop {
            match rx.try_recv() {
                Ok(CodeUpdate::Started { .. }) => {}
                Ok(CodeUpdate::Done { path, stats }) => {
                    self.computing.remove(&path);
                    self.stats.code.insert(path, *stats);
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.code_rx = Some(rx);
                    return;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.finish_stats();
                    return;
                }
            }
        }
    }

    /// Drains the background history statistics into the cache.
    fn drain_git_stats(&mut self) {
        let Some(rx) = self.git_stats_rx.take() else {
            return;
        };
        loop {
            match rx.try_recv() {
                Ok(GitStatsUpdate::Started { .. }) => {}
                Ok(GitStatsUpdate::Done { path, stats }) => {
                    self.computing.remove(&path);
                    self.stats.git.insert(path, stats);
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.git_stats_rx = Some(rx);
                    return;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.finish_stats();
                    return;
                }
            }
        }
    }

    /// Persists the statistics cache once a worker has finished.
    fn finish_stats(&mut self) {
        self.computing.clear();
        if let Err(error) = stats_cache::save(&self.stats_path, &self.stats) {
            log::warn!("could not write the stats cache: {error}");
        }
    }

    /// Whether a statistics worker is still running.
    fn is_computing(&self) -> bool {
        self.code_rx.is_some() || self.git_stats_rx.is_some()
    }

    fn drain_zip(&mut self) {
        let Some(rx) = self.zip_rx.take() else {
            return;
        };
        let mut summary = None;
        let mut disconnected = false;
        loop {
            match rx.try_recv() {
                Ok(update) => {
                    self.loading = Some((update.done, update.total));
                    if update.label.is_some() {
                        self.loading_detail = update.label;
                    }
                    if update.finished {
                        summary = Some((
                            update.archives,
                            update.unchanged,
                            update.errors,
                        ));
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if disconnected || summary.is_some() {
            self.loading = None;
            self.loading_label = REFRESH_LABEL;
            self.loading_detail = None;
            self.reload_zip_backups();
            if let Some((archives, unchanged, errors)) = summary {
                self.report_zip_done(archives, unchanged, errors);
            }
        } else {
            self.zip_rx = Some(rx);
        }
    }

    /// Reports the outcome of a finished ZIP-backup run.
    fn report_zip_done(
        &mut self,
        archives: usize,
        unchanged: usize,
        errors: usize,
    ) {
        // Nothing written and nothing failed: every target was already current.
        if archives == 0 && errors == 0 && unchanged > 0 {
            self.set_status("backup up to date (no changes)");
            return;
        }
        let folder =
            self.config.zip_backup_folder.as_deref().unwrap_or_default();
        let mut message = if archives == 1 {
            format!("backed up 1 archive to {folder}")
        } else {
            format!("backed up {archives} archives to {folder}")
        };
        if unchanged > 0 {
            message.push_str(&format!(" ({unchanged} unchanged)"));
        }
        if errors > 0 {
            message.push_str(&format!(" ({errors} failed)"));
        }
        self.set_status(message);
    }

    /// Whether a background ZIP backup is currently running.
    fn is_zipping(&self) -> bool {
        self.zip_rx.is_some()
    }

    /// Rebuilds the per-repo last-backup times from the backup folder (once at
    /// start and after each backup; never per frame).
    fn reload_zip_backups(&mut self) {
        self.zip_backups.clear();
        let Some(folder) = self.config.zip_backup_folder.as_deref() else {
            return;
        };
        let folder = expand_tilde(folder);
        let repos = self.service.repos();
        // Both git repos and file/folder entries can have a backup archive.
        for repo in repos.iter() {
            let dest = backup_dest(&folder, repo, repos);
            if let Ok(meta) = std::fs::metadata(&dest)
                && let Ok(modified) = meta.modified()
            {
                self.zip_backups.insert(
                    repo.path.clone(),
                    DateTime::<Local>::from(modified),
                );
            }
        }
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

    /// Restarts the full background refresh, optionally fetching first. When the
    /// preview is visible, the current tab's git logs are reloaded too.
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
        if self.preview.visible {
            let paths = self.tab_git_paths();
            self.invalidate_logs(&paths);
            self.fetch_logs(paths);
        }
    }

    /// The current tab's git entry paths (for reloading preview logs).
    fn tab_git_paths(&self) -> Vec<PathBuf> {
        self.tab_indices()
            .iter()
            .filter_map(|&i| self.service.get(i))
            .filter(|repo| repo.kind == RepoKind::Git)
            .map(|repo| repo.path.clone())
            .collect()
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
        if self.preview.visible {
            let git_paths: Vec<PathBuf> = targets
                .iter()
                .filter_map(|&i| self.service.get(i))
                .filter(|repo| repo.kind == RepoKind::Git)
                .map(|repo| repo.path.clone())
                .collect();
            self.invalidate_logs(&git_paths);
            self.fetch_logs(git_paths);
        }
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
    repo.include_in_backup = draft.include_in_backup;
}

impl App {
    /// Renders the whole screen: the panel app-frame (tinted header/content/
    /// status bands plus backgroundless hints), the entry list (and preview) in
    /// the content surface, and any overlay on top.
    fn render(&self, frame: &mut Frame) {
        let area = frame.area();
        let active = Tab::ALL
            .iter()
            .position(|tab| *tab == self.tab)
            .unwrap_or(0);
        let areas = appframe::render_frame(
            frame,
            &self.skin,
            active,
            self.status_lines(),
            &self.hint_groups(),
            self.loading.is_some(),
        );
        let (body, footer) = self.split_columns_footer(areas.content);
        let (list_area, preview_area) = self.split_preview(body);
        self.render_body(frame, list_area);
        if let Some(footer) = footer {
            self::columns::render_footer(
                frame,
                footer,
                (self.columns, self.tab),
                (self.visible_totals(), &self.colors),
            );
        }
        if let Some(preview_area) = preview_area {
            self.render_preview(frame, preview_area);
        }
        if let Some(progress_area) = areas.progress {
            self.render_progress_bar(frame, progress_area);
        }
        // Snapshot the finished view so an overlay can dim it as its backdrop.
        appframe::snapshot_frame(frame);
        self.render_overlay(frame, area);
    }

    /// Paints the refresh/backup progress bar (pre-migration style) into the
    /// panel reserved above the status band, when a run is in flight.
    fn render_progress_bar(&self, frame: &mut Frame, area: Rect) {
        let Some((done, total)) = self.loading else {
            return;
        };
        let ratio = progress_ratio(done, total);
        let prefix = self.progress_prefix(ratio, done, total);
        render_progress(
            frame,
            area,
            &self.colors,
            ProgressText {
                prefix: &prefix,
                name: self.loading_detail.as_deref().unwrap_or(""),
                ratio,
                name_width: self.loading_name_width,
            },
        );
    }

    /// The status-band lines: the info line (or the progress line while a
    /// refresh/backup runs), plus the live-filter input or a transient status
    /// message when either is active.
    fn status_lines(&self) -> Vec<Line<'_>> {
        let mut lines = vec![self.info_line()];
        if self.filtering {
            let mut spans = vec![Span::styled(
                "filter: ",
                Style::default().fg(self.colors.accent),
            )];
            spans.extend(
                self.filter
                    .render_line(Style::default(), self.colors.cursor, true)
                    .spans,
            );
            spans.push(Span::styled(
                "   Enter open · Esc clear",
                Style::default().fg(self.colors.dim),
            ));
            lines.push(Line::from(spans));
        } else if let Some((message, _)) = &self.status_msg {
            lines.push(Line::from(Span::styled(
                format!(" {message}"),
                Style::default().fg(self.colors.accent),
            )));
        }
        lines
    }

    /// The per-tab footer hints as labelled groups (clibase-style). A compact
    /// Navigation group leads, then each `bindings` group is turned into
    /// `(key, description)` pairs via the keymap, so the shown keys reflect any
    /// `[keys]` overrides. Empty groups (no bound key) are dropped.
    fn hint_groups(&self) -> Vec<(String, Vec<(String, String)>)> {
        let keymap = &self.keymap;
        let navigation = (
            "Navigation".to_string(),
            [
                ("\u{2191}\u{2193}", "move"),
                ("g/G", "top/bottom"),
                ("PgUp/PgDn", "page"),
                ("Ctrl+u/d", "half"),
            ]
            .into_iter()
            .map(|(key, desc)| (key.to_string(), desc.to_string()))
            .collect(),
        );
        let mut groups = vec![navigation];
        for (label, actions) in bindings::hint_groups(self.tab) {
            let hints = keymap.hints(actions);
            if !hints.is_empty() {
                groups.push(((*label).to_string(), hints));
            }
        }
        groups.push(global_group(keymap));
        groups
    }

    /// The app-wide chords, shared by the footer and the help overlay.
    fn global_hints(&self) -> (String, Vec<(String, String)>) {
        global_group(&self.keymap)
    }

    /// Splits the content band into the body and, outside the standard column
    /// set, the totals-and-bar footer below it. A short terminal keeps every
    /// row for the list and gets no footer.
    fn split_columns_footer(&self, content: Rect) -> (Rect, Option<Rect>) {
        let rows = self::columns::footer_rows(self.columns, content.height);
        if rows == 0 {
            return (content, None);
        }
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(rows)])
            .split(content);
        (parts[0], Some(parts[1]))
    }

    /// The summed statistics of the entries currently shown, so the totals row
    /// follows the filter.
    fn visible_totals(&self) -> crate::domain::stats::Totals {
        let entries = self
            .ordered_view()
            .into_iter()
            .filter_map(|i| self.service.get(i))
            .filter_map(|repo| self.stats.code.get(&repo.path));
        crate::domain::stats::totals(entries)
    }

    /// Splits `body` into the list area and an optional panel area, per the
    /// active [`PreviewLayout`]. A one-cell gutter separates the two, so the
    /// panel's border never touches the list.
    fn split_preview(&self, body: Rect) -> (Rect, Option<Rect>) {
        if !self.preview.visible {
            return (body, None);
        }
        let (direction, minimum, panel) = match self.preview.position {
            preview::PreviewPosition::Right => (
                Direction::Horizontal,
                MIN_LIST_COLS,
                Constraint::Percentage(self.preview.width_pct),
            ),
            preview::PreviewPosition::Bottom => (
                Direction::Vertical,
                MIN_LIST_ROWS,
                Constraint::Length(self.preview.height_rows),
            ),
        };
        let parts = Layout::default()
            .direction(direction)
            .constraints([
                Constraint::Min(minimum),
                Constraint::Length(PANEL_GUTTER),
                panel,
            ])
            .split(body);
        (parts[0], Some(parts[2]))
    }

    /// The cursor entry's path when it is a git repo whose log the preview
    /// would show (preview visible, not example mode); otherwise `None`.
    fn preview_log_path(&self) -> Option<PathBuf> {
        if !self.preview.visible || self.config.example_mode {
            return None;
        }
        let repo = self.selected_index().and_then(|i| self.service.get(i))?;
        if repo.kind != RepoKind::Git {
            return None;
        }
        Some(repo.path.clone())
    }

    /// Requests the cursor entry's preview `git log` once the cursor has rested
    /// on it for [`PREVIEW_DEBOUNCE`], so quick scrolling never blocks. The
    /// fetch itself runs on a background worker (see [`fetch_logs`]).
    fn request_preview_log(&mut self) {
        let Some(path) = self.preview_log_path() else {
            self.preview_target = None;
            return;
        };
        if self.preview_log.contains_key(&path)
            || self.preview_pending.contains(&path)
        {
            return;
        }
        if self.preview_target.as_deref() != Some(&path) {
            self.preview_target = Some(path);
            self.preview_target_at = Instant::now();
            return;
        }
        if self.preview_target_at.elapsed() < PREVIEW_DEBOUNCE {
            return;
        }
        self.fetch_logs(vec![path]);
    }

    /// Spawns a background worker to fetch the preview logs for `paths`,
    /// skipping any already cached and marking the rest as pending.
    fn fetch_logs(&mut self, paths: Vec<PathBuf>) {
        let wanted: Vec<PathBuf> = paths
            .into_iter()
            .filter(|path| !self.preview_log.contains_key(path))
            .collect();
        if wanted.is_empty() {
            return;
        }
        for path in &wanted {
            self.preview_pending.insert(path.clone());
        }
        preview_service::spawn_logs(
            Arc::clone(&self.git_client),
            wanted,
            PREVIEW_LOG_LINES,
            self.preview_tx.clone(),
        );
    }

    /// Applies any background preview-log results without blocking.
    fn drain_preview(&mut self) {
        while let Ok(log) = self.preview_rx.try_recv() {
            self.preview_pending.remove(&log.path);
            self.preview_log.insert(log.path, log.lines);
        }
    }

    /// Whether the preview is waiting on a log (debouncing or fetching), so the
    /// loop should poll faster to pick the result up promptly.
    fn preview_busy(&self) -> bool {
        if !self.preview_pending.is_empty() {
            return true;
        }
        self.preview_log_path()
            .is_some_and(|path| !self.preview_log.contains_key(&path))
    }

    /// Drops the cached and pending preview logs for `paths`, so they are
    /// re-fetched on demand. Does nothing for paths not in the cache.
    fn invalidate_logs(&mut self, paths: &[PathBuf]) {
        for path in paths {
            self.preview_log.remove(path);
            self.preview_pending.remove(path);
        }
    }

    /// Renders the detail/preview panel for the cursor entry.
    fn render_preview(&self, frame: &mut Frame, area: Rect) {
        let repo = self.selected_index().and_then(|i| self.service.get(i));
        let log = repo
            .map(|r| self.preview_log.get(&r.path).map(Vec::as_slice))
            .unwrap_or(None);
        let log_loading = self
            .preview_log_path()
            .is_some_and(|path| !self.preview_log.contains_key(&path));
        preview::render(
            frame,
            area,
            &self.skin,
            preview::PreviewContext {
                repo,
                icons: &self.icons,
                colors: &self.colors,
                example_mode: self.config.example_mode,
                log: log.unwrap_or(&[]),
                log_loading,
                code: repo.and_then(|r| self.stats.code.get(&r.path)),
                git: repo.and_then(|r| self.stats.git.get(&r.path)),
                now: Local::now().timestamp(),
                scroll: &self.preview_scroll,
            },
        );
    }

    /// The fixed-width leading part of the progress text: the percentage, plus
    /// the file counts while zipping. Widths are padded so the part keeps a
    /// constant width, pinning the `XX %` column for the whole run.
    fn progress_prefix(&self, ratio: f64, done: usize, total: usize) -> String {
        let pct = (ratio * 100.0).round() as u16;
        let pw = PERCENT_WIDTH;
        if self.loading_label == ZIP_LABEL {
            let cw = digit_count(total);
            format!("{pct:>pw$} % ({done:>cw$}/{total})")
        } else {
            format!("{pct:>pw$} %")
        }
    }

    /// The info line for the status band - error count, entry count, sort, the
    /// active lenses, local status and remote fetch time, each behind its icon.
    /// The refresh/backup progress is shown by the bar above the status band
    /// (see [`App::render_progress_bar`]), so this stays the normal info line.
    fn info_line(&self) -> Line<'_> {
        let icons = self.icons;
        let muted = Style::default().fg(self.colors.muted);
        let sep = || Span::styled("   ", Style::default().fg(self.colors.dim));
        let mut spans = vec![Span::raw(" ")];

        let errors = self.error_count();
        if errors > 0 {
            spans.push(Span::styled(
                format!("{}{errors}", icons.missing),
                Style::default()
                    .fg(self.colors.danger)
                    .add_modifier(Modifier::BOLD),
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
                Style::default().fg(self.colors.accent),
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
        Line::from(spans)
    }

    /// The remote-fetch segment of the info line: amber when over a day old or
    /// never fetched, muted otherwise.
    fn remote_span(&self, icon: &str, muted: Style) -> Span<'static> {
        match self.last_fetched {
            None => Span::styled(
                format!("{icon} never fetched"),
                Style::default().fg(self.colors.changes),
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
                    Style::default().fg(self.colors.changes)
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
    /// invalid repository from its gathered git info (set by the background
    /// refresh, so no filesystem stat happens here); a file/folder entry only
    /// reports a missing path once the on-demand existence check (`r` on the
    /// Files tab) has flagged it.
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
                .then(|| repo::PATH_NOT_FOUND.to_string()),
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
            render_empty_hint(frame, area, empty_hint(self.tab), &self.colors);
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
            skin: &self.skin,
            colors: &self.colors,
            columns: self.columns,
            code: &self.stats.code,
            git: &self.stats.git,
            computing: &self.computing,
            now: Local::now().timestamp(),
            icons: &self.icons,
            example_mode: self.config.example_mode,
            spinner,
            selected: &selected,
            has_selection: !self.selected.is_empty(),
            missing: &self.files_missing,
            show_slugs: self.show_slugs,
            query: self.filtering_active().then_some(query.as_str()),
            zip_backups: &self.zip_backups,
            offset: &self.table_offset,
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
            skin: &self.skin,
            colors: &self.colors,
            columns: self.columns,
            code: &self.stats.code,
            git: &self.stats.git,
            computing: &self.computing,
            spinner: self.spinner_frame(),
            now: Local::now().timestamp(),
            selected: &self.selected,
            has_selection: !self.selected.is_empty(),
            missing: &self.files_missing,
            show_slugs: self.show_slugs,
            zip_backups: &self.zip_backups,
            offset: &self.list_offset,
        };
        sections_view::render(frame, area, cursor, &view);
    }

    /// Renders the active overlay, if any, over the dimmed live view (the
    /// snapshot taken at the end of [`App::render`]), clibase-style.
    fn render_overlay(&self, frame: &mut Frame, area: Rect) {
        if matches!(self.overlay, Overlay::None) {
            return;
        }
        appframe::dim_backdrop(frame);
        let skin = &self.skin;
        match &self.overlay {
            Overlay::None => {}
            Overlay::Help => help::render(
                frame,
                area,
                skin,
                &self.global_hints(),
                &self.help_scroll,
            ),
            Overlay::Confirm(modal, _) => modal.render(frame, area, skin),
            Overlay::Prompt(prompt, _) => prompt.render(frame, area, skin),
            Overlay::Form(form, _) => form.render(frame, area, skin),
            Overlay::Picker(picker, _) => picker.render(frame, area, skin),
            Overlay::Errors(modal, _) => modal.render(frame, area, skin),
            Overlay::ErrorAction(modal, _) => modal.render(frame, area, skin),
            Overlay::SectionJump(modal, _) => modal.render(frame, area, skin),
            Overlay::Sort(modal, _) => modal.render(frame, area, skin),
            Overlay::Sections(modal) => modal.render(frame, area, skin),
            Overlay::SectionPrompt(prompt, _) => {
                prompt.render(frame, area, skin)
            }
            Overlay::SectionDelete(confirm, _) => {
                confirm.render(frame, area, skin)
            }
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

/// The destination archive path for `repo` in `folder`: a unique, slugified
/// file name (see [`crate::domain::backup::backup_filename`]).
fn backup_dest(folder: &Path, repo: &Repo, repos: &[Repo]) -> PathBuf {
    folder.join(backup::backup_filename(repo, repos))
}

/// The fill ratio for `done` of `total`, clamped to `0.0..=1.0` (0 when empty).
fn progress_ratio(done: usize, total: usize) -> f64 {
    if total == 0 {
        return 0.0;
    }
    (done as f64 / total as f64).clamp(0.0, 1.0)
}

/// The number of decimal digits in `n` (at least 1), for padding counts.
fn digit_count(n: usize) -> usize {
    n.to_string().len()
}

/// The composed text and fill ratio for one frame of the progress bar.
struct ProgressText<'a> {
    /// The fixed-width leading part (percentage, plus file counts when zipping).
    prefix: &'a str,
    /// The entry name shown after the prefix; empty when none is known yet.
    name: &'a str,
    /// Fill ratio in `0.0..=1.0`.
    ratio: f64,
    /// Display width reserved for the name, so the prefix column stays put as
    /// names of different lengths come and go.
    name_width: usize,
}

/// Renders a solid progress bar for an in-flight operation (status refresh or
/// ZIP backup), filling the whole `area`. The text is `prefix - name`, drawn
/// over a fixed-width block (prefix + separator + the widest name) that is
/// centred and pinned, so the percentage never shifts column as the name
/// changes. The text colour is chosen per cell from whether it sits over the
/// filled or unfilled part, so it never ends up dark text on the dark
/// (unfilled) background.
fn render_progress(
    frame: &mut Frame,
    area: Rect,
    colors: &Colors,
    text: ProgressText,
) {
    // Leave one blank cell of padding on each side of the bar.
    let area = Rect {
        x: area.x.saturating_add(1),
        width: area.width.saturating_sub(2),
        ..area
    };
    if area.width == 0 || area.height == 0 {
        return;
    }
    let filled = (f64::from(area.width) * text.ratio).round() as u16;
    let line: Vec<char> = if text.name.is_empty() {
        text.prefix.chars().collect()
    } else {
        format!("{}{PROGRESS_SEPARATOR}{}", text.prefix, text.name)
            .chars()
            .collect()
    };
    let label_width = line.len() as u16;
    // Reserve room for the widest possible line and centre that block, so the
    // left edge - and thus the `XX %` column - is fixed for the whole run.
    let block_width = (UnicodeWidthStr::width(text.prefix)
        + UnicodeWidthStr::width(PROGRESS_SEPARATOR)
        + text.name_width) as u16;
    let start = area.x + area.width.saturating_sub(block_width) / 2;
    let label_row = area.y + area.height / 2;

    let buf = frame.buffer_mut();
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let over_filled = (x - area.x) < filled;
            let bg = if over_filled {
                colors.accent
            } else {
                colors.selection_bg
            };
            let is_label =
                y == label_row && x >= start && x < start + label_width;
            let (symbol, fg) = if is_label {
                let ch = line[(x - start) as usize];
                // Dark text on the light filled bar, light text on the rest.
                let fg = if over_filled {
                    Color::Black
                } else {
                    colors.accent
                };
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
    use std::collections::BTreeMap;
    use std::path::Path;

    use crossterm::event::KeyModifiers;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::config::{Appearance, Config};
    use crate::domain::repo::GitInfo;
    use crate::storage::in_memory_repository::InMemoryRepoRepository;
    use crate::theme::GlyphVariant;

    use crate::domain::stats::GitStats;

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
        fn stats(&self, _path: &Path) -> GitStats {
            GitStats::default()
        }
    }

    fn sample_app() -> App {
        app_with_keys(BTreeMap::new())
    }

    /// A sample app whose `[keys]` section holds `overrides`.
    fn app_with_keys(overrides: BTreeMap<String, Vec<String>>) -> App {
        let mut config = sample_config();
        config.keys = overrides;
        app_with(config)
    }

    /// The demo settings every sample app shares: example mode, ASCII glyphs.
    fn sample_config() -> Config {
        Config {
            example_mode: true,
            appearance: Appearance {
                glyphs: GlyphVariant::Ascii,
                ..Appearance::default()
            },
            ..Config::default()
        }
    }

    /// A sample app over four demo entries, using `config`.
    fn app_with(config: Config) -> App {
        let mut git = Repo::new(PathBuf::from("/code/hop"));
        git.name = Some("hop".to_string());
        git.fav = true;
        let mut missing = Repo::new(PathBuf::from("/code/gone"));
        missing.slug = Some("gone".to_string());
        let mut folder = Repo::new(PathBuf::from("/notes"));
        folder.kind = RepoKind::Path;
        let mut archived = Repo::new(PathBuf::from("/old"));
        archived.archived = true;
        // Each app needs its own state files: the tests run in parallel and
        // would otherwise read each other's persisted sort and column set.
        static NEXT: std::sync::atomic::AtomicUsize =
            std::sync::atomic::AtomicUsize::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir()
            .join(format!("hop-tui-test-{}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let service = RepoService::new(
            Box::new(InMemoryRepoRepository::new(vec![
                git, missing, folder, archived,
            ])),
            dir.join("usage.toml"),
            dir.join("selected.txt"),
        )
        .unwrap();
        App::new(
            config,
            service,
            Arc::new(NoGit),
            dir.join("cache.toml"),
            dir.join("ui-state.toml"),
            StartupStatus::Refresh { fetch: false },
        )
    }

    #[test]
    fn the_active_theme_colours_the_content_cells() {
        // The whole point of dropping `tui::colors`: a re-theme must reach the
        // table cells, not just the frame and the modals.
        let rose = Colors::from_palette(&sample_config().palette());
        let mut config = sample_config();
        config.appearance.theme = "monochrome".to_string();
        let mono = Colors::from_palette(&config.palette());
        assert_ne!(rose.accent, mono.accent, "the themes must differ");

        let app = app_with(config);
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let foregrounds: Vec<_> =
            buffer.content().iter().map(|c| c.style().fg).collect();
        assert!(
            foregrounds.contains(&Some(mono.accent)),
            "the theme accent must appear in the rendered content"
        );
        assert!(
            !foregrounds.contains(&Some(rose.accent)),
            "no cell may keep the compiled-in rose accent"
        );
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

    /// The whole rendered buffer as one string.
    fn screen(app: &App, width: u16, height: u16) -> String {
        let mut terminal =
            Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    #[test]
    fn c_cycles_the_column_sets_and_only_they_show_the_bar() {
        let mut app = sample_app();
        // Standard looks exactly as it always did: no bar, no totals row.
        let standard = screen(&app, 120, 30);
        assert!(standard.contains("Branch"));
        assert!(!standard.contains("Columns"), "no bar in the standard set");

        press(&mut app, KeyCode::Char('c'));
        let code = screen(&app, 120, 30);
        assert!(code.contains("LOC") && code.contains("Language"));
        assert!(
            !code.contains("Branch"),
            "the standard columns are replaced"
        );
        assert!(code.contains("Columns"), "the bar names the sets");
        assert!(code.contains("projects"), "the totals row is shown");

        press(&mut app, KeyCode::Char('c'));
        let activity = screen(&app, 120, 30);
        assert!(activity.contains("Commits") && activity.contains("Authors"));

        press(&mut app, KeyCode::Char('c'));
        assert!(screen(&app, 120, 30).contains("Branch"), "back to standard");
    }

    #[test]
    fn example_mode_shows_dashes_rather_than_spinning_forever() {
        // No worker ever runs in example mode, so a spinner would never stop.
        // Unicode glyphs, because the ASCII spinner uses `-` itself - which is
        // also the text for a value that will never arrive.
        let mut config = sample_config();
        config.appearance.glyphs = GlyphVariant::Unicode;
        let mut app = app_with(config);
        assert!(app.config.example_mode);
        press(&mut app, KeyCode::Char('c'));
        let code = screen(&app, 120, 30);
        assert!(code.contains('-'), "an uncomputable cell reads as a dash");
        for frame in app.icons.spinner {
            assert!(
                !code.contains(frame),
                "example mode must never spin: found {frame:?}"
            );
        }
    }

    #[test]
    fn the_bar_and_totals_vanish_on_a_short_terminal() {
        let mut app = sample_app();
        press(&mut app, KeyCode::Char('c'));
        assert!(screen(&app, 120, 30).contains("Columns"));
        // Too short to spare five rows: the list keeps them.
        assert!(!screen(&app, 120, 14).contains("Columns"));
    }

    #[test]
    fn v_toggles_the_panel_and_capital_v_moves_it() {
        let mut app = sample_app();
        assert!(!screen(&app, 120, 30).contains("Details"));
        press(&mut app, KeyCode::Char('v'));
        // The border title names the cursor entry.
        assert!(screen(&app, 120, 30).contains("Details - hop"));
        assert_eq!(app.preview.position, preview::PreviewPosition::Right);
        press(&mut app, KeyCode::Char('V'));
        assert_eq!(app.preview.position, preview::PreviewPosition::Bottom);
        assert!(screen(&app, 120, 30).contains("Details - hop"));
        press(&mut app, KeyCode::Char('v'));
        assert!(!screen(&app, 120, 30).contains("Details"));
    }

    #[test]
    fn ctrl_arrows_resize_the_panel_only_while_it_is_open() {
        let mut app = sample_app();
        let before = app.preview.width_pct;
        // Closed: the chord does nothing.
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL));
        assert_eq!(app.preview.width_pct, before);

        press(&mut app, KeyCode::Char('v'));
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL));
        assert!(app.preview.width_pct > before, "ctrl+right widens it");
        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL));
        assert_eq!(app.preview.width_pct, before, "ctrl+left narrows it back");
    }

    #[test]
    fn t_opens_the_sort_picker_and_re_picking_flips_the_direction() {
        let mut app = sample_app();
        press(&mut app, KeyCode::Char('t'));
        assert!(matches!(app.overlay, Overlay::Sort(_, _)));
        let listed = screen(&app, 120, 30);
        assert!(listed.contains("Sort by") && listed.contains("Frecency"));
        // Standard offers no column modes.
        assert!(!listed.contains("Lines of code"));

        // Name is active and first; Enter re-picks it and flips the direction.
        assert_eq!((app.sort, app.sort_dir), (SortMode::Name, SortDir::Asc));
        press(&mut app, KeyCode::Enter);
        assert_eq!((app.sort, app.sort_dir), (SortMode::Name, SortDir::Desc));
    }

    #[test]
    fn the_sort_picker_offers_the_active_column_sets_modes() {
        let mut app = sample_app();
        press(&mut app, KeyCode::Char('c'));
        press(&mut app, KeyCode::Char('t'));
        let listed = screen(&app, 120, 30);
        assert!(listed.contains("Lines of code") && listed.contains("Size"));
        assert!(!listed.contains("Commits"), "those belong to Activity");
    }

    #[test]
    fn the_files_tab_never_offers_the_activity_columns() {
        let mut app = sample_app();
        press(&mut app, KeyCode::Char('2'));
        press(&mut app, KeyCode::Char('c'));
        assert_eq!(app.columns, ColumnSet::Code);
        press(&mut app, KeyCode::Char('c'));
        assert_eq!(app.columns, ColumnSet::Standard, "Activity is skipped");
    }

    #[test]
    fn a_column_set_the_tab_lacks_falls_back_when_switching_to_it() {
        let mut app = sample_app();
        press(&mut app, KeyCode::Char('c'));
        press(&mut app, KeyCode::Char('c'));
        assert_eq!(app.columns, ColumnSet::Activity);
        press(&mut app, KeyCode::Char('2'));
        assert_eq!(app.columns, ColumnSet::Standard);
    }

    #[test]
    fn progress_bar_paints_accent_fill_and_label() {
        // A half-filled bar: the left cells carry the accent background and the
        // centred percentage label is present.
        let colors = Colors::from_palette(&Config::default().palette());
        let mut terminal = Terminal::new(TestBackend::new(40, 2)).unwrap();
        terminal
            .draw(|frame| {
                render_progress(
                    frame,
                    frame.area(),
                    &colors,
                    ProgressText {
                        prefix: " 50 %",
                        name: "repo",
                        ratio: 0.5,
                        name_width: 4,
                    },
                );
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        // A cell early in the bar (inside the padded, filled region) is accent.
        assert_eq!(buf.cell((2, 0)).unwrap().style().bg, Some(colors.accent));
        // A cell near the right end (past the half fill) is the track colour.
        assert_eq!(
            buf.cell((38, 0)).unwrap().style().bg,
            Some(colors.selection_bg)
        );
        let text: String = buf
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        assert!(text.contains('%') && text.contains("repo"));
    }

    #[test]
    fn copy_targets_collects_selected_paths() {
        let mut app = sample_app();
        // No selection: just the cursor entry (the first git repo).
        assert_eq!(app.target_paths(), vec!["/code/hop".to_string()]);
        // Selecting two entries copies both paths, in index order.
        app.selected.insert(0);
        app.selected.insert(2);
        assert_eq!(
            app.target_paths(),
            vec!["/code/hop".to_string(), "/notes".to_string()],
        );
    }

    #[test]
    fn hint_groups_are_labelled_and_tab_specific() {
        let labels = |app: &App| {
            app.hint_groups()
                .into_iter()
                .map(|(label, _)| label)
                .collect::<Vec<_>>()
        };
        let mut app = sample_app();
        app.tab = Tab::GitRepos;
        let git = labels(&app);
        assert_eq!(git.first().map(String::as_str), Some("Navigation"));
        assert!(git.contains(&"Git".to_string()));
        assert!(!git.contains(&"Sections".to_string()));

        app.tab = Tab::FilesAndFolders;
        let files = labels(&app);
        assert!(files.contains(&"Sections".to_string()));
        assert!(files.contains(&"Paths".to_string()));
        assert!(!files.contains(&"Git".to_string()));

        // Keys come from the keymap (per-action), e.g. add -> "n".
        let add = app
            .hint_groups()
            .into_iter()
            .flat_map(|(_, pairs)| pairs)
            .find(|(_, desc)| desc == "add");
        assert_eq!(add, Some(("n".to_string(), "add".to_string())));
    }

    #[test]
    fn hint_band_grows_to_fit_all_hints() {
        // At a narrow width the hints wrap past two rows; the panel's hint band
        // must size to hold them all rather than clipping at a fixed height.
        let app = sample_app();
        let width = 60;
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
    fn pressing_shift_l_on_a_git_repo_returns_launch_outcome() {
        let mut app = sample_app();
        // The first git-tab entry is the git repo "hop".
        let outcome = app
            .handle_key(KeyEvent::new(KeyCode::Char('L'), KeyModifiers::NONE));
        assert!(matches!(outcome, Some(RunOutcome::LaunchGitTool(_))));
    }

    #[test]
    fn pressing_enter_on_a_git_repo_only_jumps() {
        let mut app = sample_app();
        // The first git-tab entry is the git repo "hop".
        let outcome =
            app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(outcome, Some(RunOutcome::Jumped)));
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
    fn q_quits() {
        let mut app = sample_app();
        let outcome = app
            .handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(matches!(outcome, Some(RunOutcome::Quit)));
    }

    #[test]
    fn ctrl_q_is_left_to_the_terminal_guard() {
        // The `Tui` turns `Ctrl+Q` into `TuiEvent::Quit` before dispatch ever
        // sees it, so no list binding may claim the chord. Dispatch used to
        // quit on it only because it matched on `KeyCode` and ignored the
        // modifier.
        let mut app = sample_app();
        let outcome = app.handle_key(KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::CONTROL,
        ));
        assert!(outcome.is_none());
    }

    #[test]
    fn a_keys_override_rebinds_the_action() {
        let overrides =
            BTreeMap::from([("quit".to_string(), vec!["w".to_string()])]);
        let mut app = app_with_keys(overrides);
        let rebound = app
            .handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));
        assert!(matches!(rebound, Some(RunOutcome::Quit)));
        let default = app
            .handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(
            default.is_none(),
            "the replaced default must not still quit"
        );
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
