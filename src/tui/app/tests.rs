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
fn c_cycles_the_column_sets_and_only_they_show_the_bar() {
    let mut app = sample_app();
    // Flat view (grouping off) shows the classic table with column headers.
    press(&mut app, KeyCode::Char('.'));
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
    for frame in spinner_frames(GlyphVariant::Unicode) {
        assert!(
            !code.contains(frame),
            "example mode must never spin: found {frame:?}"
        );
    }
}

/// Every distinct frame of the toolkit spinner, gathered by stepping it
/// until it wraps back to where it started.
fn spinner_frames(variant: GlyphVariant) -> Vec<&'static str> {
    let mut spinner = Spinner::new();
    let first = spinner.frame(variant);
    let mut frames = vec![first];
    loop {
        spinner.advance();
        let frame = spinner.frame(variant);
        if frame == first {
            return frames;
        }
        frames.push(frame);
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
    assert_eq!(
        app.preview.position,
        crate::tui::preview::PreviewPosition::Right
    );
    press(&mut app, KeyCode::Char('V'));
    assert_eq!(
        app.preview.position,
        crate::tui::preview::PreviewPosition::Bottom
    );
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
fn every_overlay_renders_at_a_normal_and_a_cramped_size() {
    // The form, the slug prompt and the path picker size their fields from
    // the box's inner width; on a terminal too small to hold the box that
    // width goes to zero, which must clamp rather than underflow.
    // Each case is the keys that open the overlay; `M` is Files-tab only.
    // The error list is left out: it needs a failing entry, and it is the
    // same `SelectModal` the sort picker already covers.
    let openers = [
        ("n", "add form"),
        ("e", "edit form"),
        ("d", "delete confirm"),
        ("S", "slug prompt"),
        ("p", "path picker"),
        ("2M", "manage sections"),
        ("t", "sort picker"),
        ("?", "help"),
    ];
    for (keys, what) in openers {
        let mut app = sample_app();
        for key in keys.chars() {
            press(&mut app, KeyCode::Char(key));
        }
        assert!(
            !matches!(app.overlay, Overlay::None),
            "{what} did not open on {keys:?}"
        );
        for (width, height) in [(100, 30), (20, 6)] {
            let _ = screen(&app, width, height);
        }
    }
}

#[test]
fn a_long_filter_query_scrolls_inside_the_status_band() {
    // The status band is one row wide; the toolkit's field scrolls the query
    // under the caret and marks the hidden head, rather than overrunning the
    // trailing hint. A hand-drawn caret used to just overflow.
    let mut app = sample_app();
    press(&mut app, KeyCode::Char('f'));
    for ch in "abcdefghijklmnopqrstuvwxyz0123456789".chars() {
        press(&mut app, KeyCode::Char(ch));
    }
    let narrow = screen(&app, 60, 20);
    assert!(narrow.contains('\u{2026}'), "the clipped head is marked");
    assert!(narrow.contains("Esc clear"), "the trailing hint survives");
    assert!(narrow.contains('9'), "the caret's end stays visible");
    // Given room, the whole query fits and nothing is marked as clipped.
    let wide = screen(&app, 120, 20);
    assert!(wide.contains("abcdefghijklmnopqrstuvwxyz0123456789"));
}

#[test]
fn the_progress_label_keeps_its_width_as_the_name_changes() {
    // The gauge centres the label, so a constant label width is what pins
    // the `XX %` column while entry names of different lengths come and go.
    let label_for = |name: &str| {
        ProgressText {
            prefix: " 50 %",
            name,
            ratio: 0.5,
            name_width: 6,
        }
        .label()
    };
    let widths: Vec<usize> = ["", "hop", "mdtask"]
        .iter()
        .map(|n| label_for(n).len())
        .collect();
    assert_eq!(widths, vec![widths[0]; 3]);
    assert!(label_for("hop").starts_with(" 50 % - hop"));
    // Without a name the separator is blanked out rather than dangling.
    assert!(!label_for("").contains('-'));
}

#[test]
fn progress_bar_paints_accent_fill_and_label() {
    // A half-filled bar: the left cells carry the accent background and the
    // centred percentage label is present.
    let config = Config::default();
    let colors = Colors::from_palette(&config.palette());
    let skin = config.skin();
    let mut terminal = Terminal::new(TestBackend::new(40, 2)).unwrap();
    terminal
        .draw(|frame| {
            render_progress(
                frame,
                frame.area(),
                &skin,
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
    // A cell early in the bar (inside the filled region) is accent.
    assert_eq!(buf.cell((2, 0)).unwrap().style().bg, Some(colors.accent));
    // A cell near the right end (past the half fill) is the track colour.
    assert_eq!(
        buf.cell((38, 0)).unwrap().style().bg,
        Some(colors.selection_bg)
    );
    // The bar runs edge to edge: the first cell is filled and the last one
    // carries the track, so neither side is left blank.
    assert_eq!(buf.cell((0, 0)).unwrap().style().bg, Some(colors.accent));
    assert_eq!(
        buf.cell((39, 0)).unwrap().style().bg,
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
        Some("shift+up/shift+down".to_string())
    );

    let overrides =
        BTreeMap::from([("extend_up".to_string(), vec!["K".to_string()])]);
    let custom = navigation_group(&Keymap::from_overrides(&overrides));
    assert_eq!(keys_for(&custom, extend), Some("K/shift+down".to_string()));
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
    let add = "add an entry (fill the form; ^O picks the path)";

    let app = sample_app();
    assert_eq!(row(&app, add), Some("n".to_string()));

    let overrides =
        BTreeMap::from([("add".to_string(), vec!["N".to_string()])]);
    let custom = app_with_keys(overrides);
    assert_eq!(row(&custom, add), Some("N".to_string()));
}

/// Home and End are what §1.7 requires; g and G are the optional vim addition,
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
