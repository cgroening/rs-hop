//! The application screen: the [`App`] state, its construction and the run
//! loop.
//!
//! The struct and the loop live here; the behaviours are split by
//! responsibility into sibling modules, each with its own `impl App` block:
//! [`keys`] (routing a key to a use case), [`view`] (what is currently visible
//! and the per-kind view settings behind it), [`navigate`] (cursor, tab and
//! selection movement), [`lenses`] (the view toggles and the sort picker),
//! [`open`] (leaving the TUI for something else), [`edit`] (mutating entries),
//! [`section_manager`] (the section overlays), [`refresh`] (the git-status
//! workers and the path-error state they feed), [`stats`] (the statistics
//! workers), [`zip`] (the ZIP-backup worker), [`panel`] (the detail panel),
//! [`render`] (state to frame), [`hints`] (footer/help metadata) and
//! [`progress`] (the progress bar).
//!
//! Opening an entry returns a [`RunOutcome`] the composition root acts on after
//! the terminal is restored.

mod edit;
mod hints;
mod keys;
mod lenses;
mod navigate;
mod open;
mod panel;
mod progress;
mod refresh;
mod render;
mod section_manager;
mod stats;
mod view;
mod zip;

use super::app::progress::REFRESH_LABEL;
use crate::tui::terminal::{Tui, TuiEvent};
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};
use ratatui::Frame;

use ratada::input::InputField;
use ratada::quit::{self, QuitConfirm, QuitKind};
use ratada::shortcut_hints;
use ratada::spinner::Spinner;

use crate::config::Config;
use crate::domain::filter::{Tab, TabKind};
use crate::domain::sort::{SortDir, SortMode};
use crate::keymap::Keymap;
use crate::service::preview_service::PreviewLog;
use crate::service::repo_service::RepoService;
use crate::service::stats_service::{
    self, CodeUpdate, GitStatsUpdate, StatsCache,
};
use crate::service::status_service::{self, StatusUpdate};
use crate::service::ui_state_service::{self, TabView};
use crate::service::zip_service::ZipUpdate;
use crate::storage::git_client::GitClient;
use crate::theme::Skin;
use crate::tui::columns::ColumnSet;
use crate::tui::form::RepoForm;
use crate::tui::path_picker::PathPicker;
use crate::tui::presentation::IconSet;
use crate::tui::preview::PreviewLayout;
use crate::tui::section_picker::SectionPicker;
use crate::tui::sections_modal::SectionsModal;
use crate::tui::skin::Colors;
use crate::tui::widgets::{ConfirmModal, SelectModal, TextPrompt};

/// How long a transient status message stays visible.
const STATUS_TTL: Duration = Duration::from_secs(4);

/// How long one refresh-spinner frame is shown.
const SPINNER_INTERVAL: Duration = Duration::from_millis(120);

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
    /// Boxed: the form's three text fields make it far larger than any other
    /// overlay, and `Overlay` is held by value in `App`. The target says whether
    /// it adds, edits one entry, or bulk-edits several.
    Form(Box<RepoForm>, EditTarget),
    Picker(PathPicker, PickerIntent),
    /// The fuzzy section picker over a form in progress (the form is stashed so
    /// it resumes with the chosen section).
    SectionPicker(Box<SectionPicker>, Box<RepoForm>, EditTarget),
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
    FormPath(Box<RepoForm>, EditTarget),
}

/// What a form is editing: a new entry, one existing entry, or several at once.
#[derive(Clone)]
enum EditTarget {
    /// A new entry (the add form).
    Add,
    /// The single entry at this service index.
    One(usize),
    /// Several entries (bulk edit), by service index.
    Bulk(Vec<usize>),
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
    /// Per-kind view settings (sort, columns, grouping, fav-float), indexed by
    /// `tab.kind_index()`; a kind's active and archive views share its slot.
    tab_state: [TabState; 2],
    filtering: bool,
    filter: InputField,
    overlay: Overlay,
    /// The help overlay's scroll position, kept across frames.
    help_scroll: crate::tui::scroll::Scroll,
    /// The help overlay's fuzzy filter, kept while the overlay is open.
    help_query: InputField,
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
    /// All in-flight background status refreshes, drained each loop. Several may
    /// run at once, so starting one never cancels another.
    status_jobs: Vec<RefreshJob>,
    /// Paths across all active refreshes that have not been updated yet (drive
    /// the per-row spinner). Empty when no refresh is running.
    refreshing: HashSet<PathBuf>,
    /// The per-row refresh spinner, stepped by [`App::tick`].
    spinner: Spinner,
    /// When the spinner last stepped, so it animates at a steady rate rather
    /// than at whatever rate the run loop happens to wake up.
    spinner_at: Instant,
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
    /// Whether slugs are shown in their own dim-italic column after the name.
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
    preview_scroll: crate::tui::scroll::Scroll,
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

/// The view settings a kind remembers, shared between its active and archive
/// views (persisted per kind in the UI state).
#[derive(Debug, Clone, Copy)]
struct TabState {
    /// The list sort mode.
    sort: SortMode,
    /// Which way the sort runs.
    sort_dir: SortDir,
    /// Which columns the table shows.
    columns: ColumnSet,
    /// Whether entries are grouped into sections (off = flat global sort).
    grouped: bool,
    /// Whether favourites float to the top of each group.
    fav_float: bool,
}

/// Builds a [`TabState`] from a persisted per-kind [`TabView`], resolving the
/// column-set key and clamping it to what `kind` supports.
fn tab_state_from(view: &TabView, kind: TabKind) -> TabState {
    TabState {
        sort: view.sort,
        sort_dir: view.sort_dir,
        columns: ColumnSet::from_key(&view.columns)
            .available_on(kind.active_tab()),
        grouped: view.grouped,
        fav_float: view.fav_float,
    }
}

/// One in-flight background status refresh. Several may run concurrently, so a
/// new refresh never cancels one already running.
struct RefreshJob {
    /// Receives this worker's `StatusUpdate`s.
    rx: Receiver<StatusUpdate>,
    /// Whether this job's completion updates the global `fetched_at` time (a
    /// full refresh with `fetch`; a subset refresh never does).
    fetched: bool,
    /// Whether this job feeds the full-width progress bar.
    bar: bool,
    /// Paths of this job not yet reported, so its spinners can be cleared if the
    /// job ends before every path is done.
    remaining: HashSet<PathBuf>,
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

/// Runs the TUI event loop until the user opens an entry or quits.
///
/// # Errors
///
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
            Some(TuiEvent::Paste(text)) => {
                app.handle_paste(&text);
                None
            }
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
        let cached = status_service::load_cache(&cache_path);
        let ui = ui_state_service::load(&ui_state_path);
        shortcut_hints::set_visible(ui.hints_visible);
        install_quit_confirmation(config.confirm_quit, skin);
        // The ZIP cache lives next to the git-info cache in the state directory.
        let state_dir = cache_path.parent().unwrap_or_else(|| Path::new("."));
        let zip_cache_path = state_dir.join("zip-manifests.toml");
        let stats_path = state_dir.join("stats-cache.toml");
        let stats = stats_service::load_cache(&stats_path);
        let mut service = service;
        service.apply_git_infos(&cached.infos);
        let (preview_tx, preview_rx) = mpsc::channel();
        let tab_state = [
            tab_state_from(&ui.git, TabKind::Git),
            tab_state_from(&ui.files, TabKind::Files),
        ];
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
            tab_state,
            filtering: false,
            filter: InputField::new(""),
            overlay: Overlay::None,
            help_scroll: crate::tui::scroll::Scroll::default(),
            help_query: InputField::new(""),
            status_msg: None,
            loading: None,
            loading_label: REFRESH_LABEL,
            loading_detail: None,
            loading_name_width: 0,
            zip_rx: None,
            zip_backups: HashMap::new(),
            cache_generated_at: cached.generated_at,
            last_fetched: cached.fetched_at,
            status_jobs: Vec::new(),
            refreshing: HashSet::new(),
            spinner: Spinner::new(),
            spinner_at: Instant::now(),
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
            preview_scroll: crate::tui::scroll::Scroll::default(),
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
            if app.tab.kind() == TabKind::Git {
                app.start_refresh(fetch);
            }
        }
        // The active tab counts as its first visit (e.g. the Files existence
        // check, or a git refresh that was skipped above).
        app.refresh_tab_on_first_visit();
        app
    }

    /// Sets a transient status message.
    fn set_status(&mut self, message: impl Into<String>) {
        self.status_msg = Some((message.into(), Instant::now()));
    }

    /// Expires the status message once its TTL passes and steps the spinner.
    fn tick(&mut self) {
        if let Some((_, at)) = &self.status_msg
            && at.elapsed() > STATUS_TTL
        {
            self.status_msg = None;
        }
        if !self.refreshing.is_empty()
            && self.spinner_at.elapsed() >= SPINNER_INTERVAL
        {
            self.spinner.advance();
            self.spinner_at = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests;
