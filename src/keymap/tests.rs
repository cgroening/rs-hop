use std::collections::BTreeMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, mods)
}

#[test]
fn parses_chars_function_keys_and_named_keys() {
    assert_eq!(KeyChord::parse("a").unwrap().display(), "a");
    assert_eq!(KeyChord::parse("G").unwrap().display(), "G");
    assert_eq!(KeyChord::parse("pgup").unwrap().display(), "pgup");
    assert_eq!(KeyChord::parse("enter").unwrap().display(), "enter");
    assert_eq!(KeyChord::parse("ctrl+u").unwrap().display(), "ctrl+u");
    assert_eq!(KeyChord::parse("alt+up").unwrap().display(), "alt+up");
    assert!(KeyChord::parse("nonsense").is_none());
    assert!(KeyChord::parse("").is_none());
}

#[test]
fn matches_distinguishes_ctrl_and_alt_arrows() {
    let plain = KeyChord::parse("up").unwrap();
    let ctrl = KeyChord::parse("ctrl+up").unwrap();
    let alt = KeyChord::parse("alt+up").unwrap();
    assert!(plain.matches(&key(KeyCode::Up, KeyModifiers::NONE)));
    assert!(!plain.matches(&key(KeyCode::Up, KeyModifiers::CONTROL)));
    assert!(ctrl.matches(&key(KeyCode::Up, KeyModifiers::CONTROL)));
    assert!(alt.matches(&key(KeyCode::Up, KeyModifiers::ALT)));
}

#[test]
fn default_map_binds_expected_actions() {
    let map = Keymap::default();
    assert_eq!(
        map.action_for(&key(KeyCode::Char('n'), KeyModifiers::NONE)),
        Some(Action::Add),
    );
    assert_eq!(
        map.action_for(&key(KeyCode::Char('z'), KeyModifiers::NONE)),
        Some(Action::Zip),
    );
    assert_eq!(
        map.action_for(&key(KeyCode::Char('Z'), KeyModifiers::SHIFT)),
        Some(Action::ZipAll),
    );
    assert_eq!(
        map.action_for(&key(KeyCode::Up, KeyModifiers::ALT)),
        Some(Action::ReorderUp),
    );
    assert_eq!(map.keys_for(Action::Delete), vec!["d", "del", "backspace"]);
}

#[test]
fn ctrl_and_plain_letters_do_not_collide() {
    let map = Keymap::default();
    // ctrl+u is half-page up; plain u is undo.
    assert_eq!(
        map.action_for(&key(KeyCode::Char('u'), KeyModifiers::CONTROL)),
        Some(Action::HalfPageUp),
    );
    assert_eq!(
        map.action_for(&key(KeyCode::Char('u'), KeyModifiers::NONE)),
        Some(Action::Undo),
    );
    // ctrl+d is half-page down; plain d is delete.
    assert_eq!(
        map.action_for(&key(KeyCode::Char('d'), KeyModifiers::CONTROL)),
        Some(Action::HalfPageDown),
    );
    assert_eq!(
        map.action_for(&key(KeyCode::Char('d'), KeyModifiers::NONE)),
        Some(Action::Delete),
    );
}

#[test]
fn override_replaces_an_action_and_keeps_others() {
    let mut overrides = BTreeMap::new();
    overrides.insert("add".to_string(), vec!["N".to_string()]);
    let map = Keymap::from_overrides(&overrides);
    assert_eq!(
        map.action_for(&key(KeyCode::Char('N'), KeyModifiers::SHIFT)),
        Some(Action::Add),
    );
    assert_ne!(
        map.action_for(&key(KeyCode::Char('n'), KeyModifiers::NONE)),
        Some(Action::Add),
    );
    assert_eq!(map.keys_for(Action::Edit), vec!["e"]);
}

#[test]
fn default_map_has_no_conflicts() {
    assert!(Keymap::default().conflicts().is_empty());
}

#[test]
fn every_action_has_a_reachable_default_key() {
    // Dispatch resolves keys through the map, so an action whose default
    // chord does not parse or round-trip would be silently unreachable.
    let map = Keymap::default();
    for action in Action::all() {
        assert!(
            !map.keys_for(action).is_empty(),
            "'{}' has no bound key",
            action.config_name()
        );
        for text in action.default_keys() {
            let chord = KeyChord::parse(text).expect("default key must parse");
            assert_eq!(
                map.action_for(&chord.to_key()),
                Some(action),
                "'{text}' does not resolve back to '{}'",
                action.config_name()
            );
        }
    }
}

#[test]
fn hints_pair_keys_with_descriptions() {
    let map = Keymap::default();
    let hints = map.hints(&[Action::Add, Action::Quit]);
    assert_eq!(hints[0], ("n".to_string(), "add".to_string()));
    assert_eq!(hints[1], ("q".to_string(), "quit".to_string()));
}

#[test]
fn actions_table_matches_the_enum() {
    // Compile-time reminder: adding an `Action` variant breaks this match
    // until it is listed, prompting a new `ACTIONS` row.
    fn covered(action: Action) {
        match action {
            Action::Up
            | Action::Down
            | Action::Top
            | Action::Bottom
            | Action::PageUp
            | Action::PageDown
            | Action::HalfPageUp
            | Action::HalfPageDown
            | Action::TabGit
            | Action::TabFiles
            | Action::ToggleSelect
            | Action::ExtendUp
            | Action::ExtendDown
            | Action::ExtendPageUp
            | Action::ExtendPageDown
            | Action::Jump
            | Action::Open
            | Action::GitTool
            | Action::JumpCd
            | Action::OpenApp
            | Action::Filter
            | Action::ChangesFilter
            | Action::Github
            | Action::Preview
            | Action::PreviewPosition
            | Action::PreviewScrollUp
            | Action::PreviewScrollDown
            | Action::PreviewPageUp
            | Action::PreviewPageDown
            | Action::PreviewShrink
            | Action::PreviewGrow
            | Action::Columns
            | Action::Sort
            | Action::ToggleGrouping
            | Action::ToggleFavFloat
            | Action::SectionJump
            | Action::SectionPrev
            | Action::SectionNext
            | Action::ToggleCollapse
            | Action::ToggleCollapseAll
            | Action::ManageSections
            | Action::ReorderUp
            | Action::ReorderDown
            | Action::Add
            | Action::Edit
            | Action::Delete
            | Action::Undo
            | Action::ToggleFav
            | Action::Zip
            | Action::ZipAll
            | Action::Archive
            | Action::Slug
            | Action::ToggleSlugs
            | Action::CopyPath
            | Action::RepairPath
            | Action::Errors
            | Action::Reload
            | Action::ReloadFetch
            | Action::RefreshOne
            | Action::RefreshOneFetch
            | Action::Pull
            | Action::Push
            | Action::PullAll
            | Action::PushAll
            | Action::Help
            | Action::Quit => {}
        }
    }
    let _ = covered;
    // Runtime: catalog has no duplicate config names, and every enum
    // variant resolves to exactly one row.
    let mut names: Vec<&str> =
        ACTIONS.iter().map(|spec| spec.config_name).collect();
    names.sort_unstable();
    let total = names.len();
    names.dedup();
    assert_eq!(names.len(), total, "duplicate config_name in ACTIONS");
    for action in Action::all() {
        let _ = action.spec();
    }
}
