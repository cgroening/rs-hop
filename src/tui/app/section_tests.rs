//! Tests for the grouped view's section moves and folds.
//!
//! The fixture is a git tab with three shown sections: `Work` (`a`, `b`),
//! `Personal` (`c`) and the implicit Ungrouped (`d`), so the expanded rows are
//! `a b c d` and the sections start at rows 0, 2 and 3.

use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::tests::{NoGit, sample_config};
use super::*;
use crate::domain::repo::{Repo, RepoKind};
use crate::domain::sections::Row;
use crate::storage::in_memory_repository::InMemoryRepoRepository;

/// A fresh, per-test state directory (the tests run in parallel).
fn state_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("hop-section-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// The fixture app, persisting its UI state under `dir`.
fn sectioned_app_in(dir: &Path) -> App {
    let entry = |name: &str, section: Option<&str>| {
        let mut repo = Repo::new(PathBuf::from(format!("/code/{name}")));
        repo.name = Some(name.to_string());
        repo.section = section.map(str::to_string);
        repo
    };
    let mut service = RepoService::new(
        Box::new(InMemoryRepoRepository::new(vec![
            entry("a", Some("Work")),
            entry("b", Some("Work")),
            entry("c", Some("Personal")),
            entry("d", None),
        ])),
        dir.join("usage.toml"),
        dir.join("selected.txt"),
    )
    .unwrap();
    service.add_section(RepoKind::Git, "Work").unwrap();
    service.add_section(RepoKind::Git, "Personal").unwrap();
    App::new(
        sample_config(),
        service,
        Arc::new(NoGit),
        dir.join("cache.toml"),
        dir.join("ui-state.toml"),
        StartupStatus::Cached,
    )
}

/// The fixture app with its own state directory.
fn sectioned_app(tag: &str) -> App {
    sectioned_app_in(&state_dir(tag))
}

/// The display name of the cursor entry, or `None` on a collapsed header.
fn cursor_name(app: &App) -> Option<String> {
    let index = app.selected_index()?;
    Some(app.service.repos()[index].display_name().to_string())
}

fn press(app: &mut App, code: KeyCode) {
    app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn press_shift(app: &mut App, code: KeyCode) {
    app.handle_key(KeyEvent::new(code, KeyModifiers::SHIFT));
}

#[test]
fn the_fixture_shows_three_sections_in_order() {
    let app = sectioned_app("fixture");
    assert!(app.is_sectioned());
    assert_eq!(app.row_count(), 4);
    assert_eq!(cursor_name(&app).as_deref(), Some("a"));
}

#[test]
fn minus_folds_the_section_onto_its_header() {
    let mut app = sectioned_app("fold");
    app.cursor = 1;
    press(&mut app, KeyCode::Char('-'));
    assert_eq!(app.cursor, 0);
    assert_eq!(app.cursor_row(), Some(Row::Collapsed(0)));
    assert_eq!(app.row_count(), 3);
    assert!(app.is_collapsed("Work"));
}

#[test]
fn entry_actions_have_no_target_on_a_collapsed_header() {
    let mut app = sectioned_app("header-target");
    press(&mut app, KeyCode::Char('-'));
    assert_eq!(app.selected_index(), None);
    assert!(app.targets().is_empty());
}

#[test]
fn minus_on_a_collapsed_header_unfolds_it() {
    let mut app = sectioned_app("unfold-minus");
    press(&mut app, KeyCode::Char('-'));
    press(&mut app, KeyCode::Char('-'));
    assert!(!app.is_collapsed("Work"));
    assert_eq!(cursor_name(&app).as_deref(), Some("a"));
}

#[test]
fn enter_on_a_collapsed_header_unfolds_instead_of_jumping() {
    let mut app = sectioned_app("unfold-enter");
    press(&mut app, KeyCode::Char('-'));
    let outcome =
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(outcome.is_none(), "the TUI must stay open");
    assert!(!app.is_collapsed("Work"));
    assert_eq!(cursor_name(&app).as_deref(), Some("a"));
}

#[test]
fn right_moves_to_the_next_section_and_wraps() {
    let mut app = sectioned_app("right");
    press(&mut app, KeyCode::Right);
    assert_eq!(cursor_name(&app).as_deref(), Some("c"));
    press(&mut app, KeyCode::Right);
    assert_eq!(cursor_name(&app).as_deref(), Some("d"));
    press(&mut app, KeyCode::Right);
    assert_eq!(cursor_name(&app).as_deref(), Some("a"));
}

#[test]
fn left_goes_to_the_own_section_start_first() {
    let mut app = sectioned_app("left");
    app.cursor = 1;
    press(&mut app, KeyCode::Left);
    assert_eq!(cursor_name(&app).as_deref(), Some("a"));
    press(&mut app, KeyCode::Left);
    assert_eq!(cursor_name(&app).as_deref(), Some("d"), "wraps to the last");
}

#[test]
fn a_collapsed_header_is_a_section_jump_target() {
    let mut app = sectioned_app("jump-header");
    app.cursor = 2;
    press(&mut app, KeyCode::Char('-'));
    app.cursor = 0;
    press(&mut app, KeyCode::Right);
    assert_eq!(app.cursor_row(), Some(Row::Collapsed(1)));
}

#[test]
fn section_keys_do_nothing_in_the_flat_view() {
    let mut app = sectioned_app("flat");
    press(&mut app, KeyCode::Char('.'));
    assert!(!app.is_sectioned());
    press(&mut app, KeyCode::Right);
    press(&mut app, KeyCode::Char('-'));
    assert_eq!(app.cursor, 0);
    assert!(app.view().collapsed.is_empty());
}

#[test]
fn underscore_folds_all_then_unfolds_all() {
    let mut app = sectioned_app("all");
    app.cursor = 2;
    press(&mut app, KeyCode::Char('_'));
    assert_eq!(app.row_count(), 3);
    assert_eq!(
        app.cursor_row(),
        Some(Row::Collapsed(1)),
        "stays in Personal"
    );
    press(&mut app, KeyCode::Char('_'));
    assert_eq!(app.row_count(), 4);
    assert_eq!(cursor_name(&app).as_deref(), Some("c"));
}

#[test]
fn a_range_selection_skips_collapsed_headers() {
    let mut app = sectioned_app("range");
    press(&mut app, KeyCode::Char('-'));
    press_shift(&mut app, KeyCode::Down);
    press_shift(&mut app, KeyCode::Down);
    let mut selected: Vec<String> = app
        .selected
        .iter()
        .map(|&i| app.service.repos()[i].display_name().to_string())
        .collect();
    selected.sort();
    assert_eq!(selected, ["c", "d"]);
}

#[test]
fn a_hidden_entry_is_refocused_onto_its_header() {
    let mut app = sectioned_app("refocus");
    app.cursor = 1;
    let hidden = app.cursor_path();
    press(&mut app, KeyCode::Char('-'));
    app.cursor = 2;
    app.refocus(hidden);
    assert_eq!(app.cursor_row(), Some(Row::Collapsed(0)));
}

#[test]
fn folds_survive_a_restart() {
    let dir = state_dir("restart");
    let mut app = sectioned_app_in(&dir);
    press(&mut app, KeyCode::Char('-'));
    let reopened = sectioned_app_in(&dir);
    assert!(reopened.is_collapsed("Work"));
    assert_eq!(reopened.row_count(), 3);
}

#[test]
fn renaming_a_section_keeps_its_fold() {
    let mut app = sectioned_app("rename");
    press(&mut app, KeyCode::Char('-'));
    app.submit_section_prompt(
        SectionPromptKind::Rename("Work".to_string()),
        " Job ".to_string(),
    );
    assert!(app.is_collapsed("Job"));
    assert!(!app.is_collapsed("Work"));
}

#[test]
fn deleting_a_section_drops_its_fold() {
    let mut app = sectioned_app("delete");
    press(&mut app, KeyCode::Char('-'));
    app.delete_section("Work");
    assert!(app.view().collapsed.is_empty());
}

#[test]
fn ctrl_arrows_still_scroll_the_panel() {
    let app = sectioned_app("ctrl");
    let key = KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL);
    assert_eq!(
        app.keymap.action_for(&key),
        Some(crate::keymap::Action::PreviewScrollUp),
    );
}

#[test]
fn a_collapsed_header_renders_with_its_count() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let mut app = sectioned_app("render");
    press(&mut app, KeyCode::Char('-'));
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| app.render(frame)).unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(
        screen.contains("> Work (2)"),
        "ASCII closed marker and count"
    );
    assert!(screen.contains("v Personal"), "ASCII open marker");
}
