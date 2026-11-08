//! Tests for the application screen.
//!
//! They drive the whole [`App`] rather than one child module, so
//! they live together here instead of being split alongside the
//! `impl` blocks.
use std::collections::BTreeMap;
use std::path::Path;

use crossterm::event::KeyModifiers;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::*;
// The tests drive the whole `App`, so they reach into the child modules
// that now hold the individual behaviours.
use super::hints::navigation_group;
use super::progress::ProgressText;
use super::progress::render_progress;
use super::render::relative_age;
use super::view::repo_has_change;
use crate::config::{Appearance, Config};
use crate::domain::repo::GitInfo;
use crate::domain::repo::{Repo, RepoKind};
use crate::keymap::Action;
use crate::storage::in_memory_repository::InMemoryRepoRepository;
use crate::theme::GlyphVariant;
use crate::tui::help::Section;
use crossterm::event::{KeyCode, KeyEvent};

use crate::domain::stats::GitStats;
use crate::domain::sync::SyncOutcome;
use crate::storage::git_client::SyncOptions;

mod render;

mod sync;

/// A git client that does nothing (the smoke test runs in example mode).
pub(super) struct NoGit;

impl GitClient for NoGit {
    fn collect(&self, _path: &Path) -> GitInfo {
        GitInfo::default()
    }
    fn fetch(&self, _path: &Path) {}
    fn pull(&self, _path: &Path, _options: SyncOptions) -> SyncOutcome {
        SyncOutcome::UpToDate
    }
    fn push(&self, _path: &Path, _options: SyncOptions) -> SyncOutcome {
        SyncOutcome::UpToDate
    }
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
pub(super) fn sample_config() -> Config {
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

fn press(app: &mut App, code: KeyCode) {
    app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

/// The whole rendered buffer as one string.
fn screen(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
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
fn v_toggles_the_panel_and_capital_v_moves_it() {
    let mut app = sample_app();
    assert!(!screen(&app, 120, 30).contains("Details"));
    press(&mut app, KeyCode::Char('v'));
    // The border title names the cursor entry.
    assert!(screen(&app, 120, 30).contains("Details \u{b7} hop"));
    assert_eq!(
        app.preview.position,
        crate::tui::preview::PreviewPosition::Right
    );
    press(&mut app, KeyCode::Char('V'));
    assert_eq!(
        app.preview.position,
        crate::tui::preview::PreviewPosition::Bottom
    );
    assert!(screen(&app, 120, 30).contains("Details \u{b7} hop"));
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
    assert_eq!((app.sort(), app.sort_dir()), (SortMode::Name, SortDir::Asc));
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        (app.sort(), app.sort_dir()),
        (SortMode::Name, SortDir::Desc)
    );
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
fn the_grouped_view_shows_the_column_header() {
    let mut app = sample_app();
    // Default git tab, grouping on: the column header sits above the list.
    let git = screen(&app, 120, 30);
    assert!(git.contains("Branch"), "git header shows Branch");
    assert!(git.contains("Status") && git.contains("GitHub"));
    assert!(git.contains("ZIP Backup"));

    // The files tab (grouped) shows its own header.
    press(&mut app, KeyCode::Char('2'));
    let files = screen(&app, 120, 30);
    assert!(files.contains("Type") && files.contains("Path"));
}

#[test]
fn i_shows_the_slug_in_its_own_column() {
    let mut app = sample_app();
    // Flat view (grouping off) so the table draws its column headers.
    press(&mut app, KeyCode::Char('.'));
    let before = screen(&app, 120, 30);
    assert!(!before.contains("Slug"), "no Slug header until toggled");

    press(&mut app, KeyCode::Char('i'));
    let after = screen(&app, 120, 30);
    assert!(after.contains("Slug"), "the Slug column has a header");
    // The demo git entry's slug is shown (it lives in its own column).
    assert!(after.contains("gone"), "the slug value is rendered");
}

#[test]
fn the_files_tab_never_offers_the_activity_columns() {
    let mut app = sample_app();
    press(&mut app, KeyCode::Char('2'));
    press(&mut app, KeyCode::Char('c'));
    assert_eq!(app.columns(), ColumnSet::Code);
    press(&mut app, KeyCode::Char('c'));
    assert_eq!(app.columns(), ColumnSet::Standard, "Activity is skipped");
}

#[test]
fn a_column_set_the_tab_lacks_falls_back_when_switching_to_it() {
    let mut app = sample_app();
    press(&mut app, KeyCode::Char('c'));
    press(&mut app, KeyCode::Char('c'));
    assert_eq!(app.columns(), ColumnSet::Activity);
    press(&mut app, KeyCode::Char('2'));
    // Switching to files keeps the files tab's own column set (Standard).
    assert_eq!(app.columns(), ColumnSet::Standard);
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
    app.tab = Tab::GitActive;
    let git = labels(&app);
    assert_eq!(git.first().map(String::as_str), Some("Navigation"));
    assert!(git.contains(&"Git".to_string()));
    // Sections now work on the git tabs too.
    assert!(git.contains(&"Sections".to_string()));

    app.tab = Tab::FilesActive;
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

/// The help overlay used to spell `Shift+↑↓` out as a literal, so a
/// `[keys]` override moved the key but not the text describing it.
#[test]
fn help_navigation_keys_follow_a_keys_override() {
    let keys_for = |section: &Section, description: &str| {
        section
            .1
            .iter()
            .find(|(_, text)| text == description)
            .map(|(keys, _)| keys.clone())
    };
    let extend = "extend the selection by a row";

    let default = navigation_group(&Keymap::default());
    assert_eq!(
        keys_for(&default, extend),
        Some("Shift+\u{2191}/Shift+\u{2193}".to_string())
    );

    let overrides =
        BTreeMap::from([("extend_up".to_string(), vec!["K".to_string()])]);
    let custom = navigation_group(&Keymap::from_overrides(&overrides));
    assert_eq!(
        keys_for(&custom, extend),
        Some("K/Shift+\u{2193}".to_string())
    );
}

/// The help overlay used to spell its keys out as literals, so a `[keys]`
/// override moved the key everywhere except in the overlay describing it.
#[test]
fn help_section_keys_follow_a_keys_override() {
    let row = |app: &App, description: &str| {
        app.help_sections()
            .into_iter()
            .flat_map(|(_, rows)| rows)
            .find(|(_, text)| text == description)
            .map(|(keys, _)| keys)
    };
    let add = "add an entry (fill the form; enter on Path picks it)";

    let app = sample_app();
    assert_eq!(row(&app, add), Some("n".to_string()));

    let overrides =
        BTreeMap::from([("add".to_string(), vec!["N".to_string()])]);
    let custom = app_with_keys(overrides);
    assert_eq!(row(&custom, add), Some("N".to_string()));
}

/// Home and End are required; g and G are the optional vim addition,
/// so both must reach the same action.
#[test]
fn the_list_ends_are_reachable_by_home_end_and_by_g() {
    let keymap = Keymap::default();
    for key in ["home", "g"] {
        assert!(
            keymap.keys_for(Action::Top).iter().any(|k| k == key),
            "{key} must jump to the top"
        );
    }
    for key in ["end", "G"] {
        assert!(
            keymap.keys_for(Action::Bottom).iter().any(|k| k == key),
            "{key} must jump to the bottom"
        );
    }
}

/// The help overlay used to show its filter only as a footer hint, so typing
/// narrowed the list without any visible input.
#[test]
fn typing_in_the_help_overlay_shows_the_query_on_its_search_line() {
    let mut app = sample_app();
    press(&mut app, KeyCode::Char('?'));
    for ch in "quit".chars() {
        press(&mut app, KeyCode::Char(ch));
    }
    assert!(matches!(app.overlay, Overlay::Help), "typing keeps it open");
    let shown = screen(&app, 100, 30);
    assert!(
        shown.contains("search quit"),
        "no visible query in:\n{shown}"
    );

    press(&mut app, KeyCode::Esc);
    assert!(matches!(app.overlay, Overlay::None), "Esc closes the help");
}

#[test]
fn pressing_shift_l_on_a_git_repo_returns_launch_outcome() {
    let mut app = sample_app();
    // The first git-tab entry is the git repo "hop".
    let outcome =
        app.handle_key(KeyEvent::new(KeyCode::Char('L'), KeyModifiers::NONE));
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
    let outcome =
        app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
    assert!(matches!(outcome, Some(RunOutcome::LaunchGitToolInline(_))));
}

#[test]
fn q_quits() {
    let mut app = sample_app();
    let outcome =
        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
    assert!(matches!(outcome, Some(RunOutcome::Quit)));
}

#[test]
fn ctrl_q_is_left_to_the_terminal_guard() {
    // The `Tui` turns `Ctrl+Q` into `TuiEvent::Quit` before dispatch ever
    // sees it, so no list binding may claim the chord. Dispatch used to
    // quit on it only because it matched on `KeyCode` and ignored the
    // modifier.
    let mut app = sample_app();
    let outcome = app
        .handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL));
    assert!(outcome.is_none());
}

#[test]
fn a_keys_override_rebinds_the_action() {
    let overrides =
        BTreeMap::from([("quit".to_string(), vec!["w".to_string()])]);
    let mut app = app_with_keys(overrides);
    let rebound =
        app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));
    assert!(matches!(rebound, Some(RunOutcome::Quit)));
    let default =
        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
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

/// The transient status message, or an empty string when none is shown.
fn status_text(app: &App) -> String {
    app.status_msg
        .as_ref()
        .map(|(message, _)| message.clone())
        .unwrap_or_default()
}
