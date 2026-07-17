//! Configurable key bindings for hop's own shortcuts.
//!
//! Maps key chords to [`Action`]s, built from compiled-in defaults plus
//! per-action overrides from `[keys]` in config. Kept free of UI-framework
//! widgets so both the TUI (dispatch + footer/help hints) and the CLI use it.
//! Widget-internal shortcuts (modals, pickers, forms) are not covered here.
//!
//! Some keys are context-dependent (`r`/`R` reload on the git tabs and re-check
//! paths on the Files tab; `s` only jumps to a section where the list is
//! sectioned); the keymap resolves the key to an [`Action`] and the app
//! interprets it per tab. `Shift`+`Tab` (cycle back) is still handled inline in
//! dispatch, because tab cycling is structural rather than a user-facing action.
//!
//! The chord grammar, the defaults-vs-overrides merge and the display form live
//! in [`ratada::keymap`]; this module owns only the action catalog.

pub use ratada::keymap::{Conflict as ChordConflict, KeyChord};

/// The resolved key map for hop's actions.
pub type Keymap = ratada::keymap::Keymap<Action>;

/// A configured key dropped because an earlier action already claimed it.
pub type Conflict = ChordConflict<Action>;

/// An app-level action a key can trigger. The single source of truth for the
/// action's config name, human description and default keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Move the cursor up one row.
    Up,
    /// Move the cursor down one row.
    Down,
    /// Jump to the first row.
    Top,
    /// Jump to the last row.
    Bottom,
    /// Move up one page.
    PageUp,
    /// Move down one page.
    PageDown,
    /// Move up half a page.
    HalfPageUp,
    /// Move down half a page.
    HalfPageDown,
    /// Switch to Git Repos (a second press toggles its archive).
    TabGit,
    /// Switch to Files and Folders (a second press toggles its archive).
    TabFiles,
    /// Toggle the current row's multi-selection.
    ToggleSelect,
    /// Extend the multi-selection to the row above.
    ExtendUp,
    /// Extend the multi-selection to the row below.
    ExtendDown,
    /// Jump: write the cd path and exit.
    Jump,
    /// Open the entry (git tool / cd / editor / default app).
    Open,
    /// Launch the git tool inline as an overlay.
    GitTool,
    /// Jump only (cd path), like [`Action::Jump`], via `o`.
    JumpCd,
    /// Force open with the default application.
    OpenApp,
    /// Start the live fuzzy filter.
    Filter,
    /// Toggle showing only repos with a status change.
    ChangesFilter,
    /// Open the entry on GitHub.
    Github,
    /// Show or hide the detail panel.
    Preview,
    /// Move the detail panel to the other side.
    PreviewPosition,
    /// Scroll the detail panel up.
    PreviewScrollUp,
    /// Scroll the detail panel down.
    PreviewScrollDown,
    /// Make the detail panel smaller.
    PreviewShrink,
    /// Make the detail panel bigger.
    PreviewGrow,
    /// Cycle the table's column set.
    Columns,
    /// Open the sort picker.
    Sort,
    /// Toggle grouping entries into sections (off = flat global sort).
    ToggleGrouping,
    /// Toggle floating favourites to the top.
    ToggleFavFloat,
    /// Jump to a section.
    SectionJump,
    /// Manage sections.
    ManageSections,
    /// Reorder the cursor entry up (custom sort / within a section).
    ReorderUp,
    /// Reorder the cursor entry down.
    ReorderDown,
    /// Add a new entry.
    Add,
    /// Edit the selected entry.
    Edit,
    /// Delete the selection or cursor entry.
    Delete,
    /// Undo the last change.
    Undo,
    /// Toggle favourite on the selection or cursor.
    ToggleFav,
    /// ZIP-backup the selection or cursor entry.
    Zip,
    /// ZIP-backup every entry opted into the backup.
    ZipAll,
    /// Archive or restore the selection or cursor.
    Archive,
    /// Set or change the slug.
    Slug,
    /// Toggle showing slugs inline after the name.
    ToggleSlugs,
    /// Copy the entry's path to the clipboard.
    CopyPath,
    /// Repair a missing path.
    RepairPath,
    /// List entries with path errors.
    Errors,
    /// Reload status (git tabs) or re-check paths (Files tab).
    Reload,
    /// Reload and `git fetch` (git tabs).
    ReloadFetch,
    /// Refresh only the selection or cursor entry.
    RefreshOne,
    /// Refresh only the selection or cursor, with `git fetch`.
    RefreshOneFetch,
    /// Toggle the help overlay.
    Help,
    /// Quit the application.
    Quit,
}

/// One catalog row: an [`Action`] with its config name, human description and
/// default keys. [`ACTIONS`] is the single source of truth these derive from.
struct ActionSpec {
    action: Action,
    config_name: &'static str,
    description: &'static str,
    default_keys: &'static [&'static str],
}

/// Every action with its config name, description and default keys, in a stable
/// order. This order is also used for hint output and, in `from_overrides`, for
/// conflict precedence (an earlier action claims a contested key first).
const ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        action: Action::Up,
        config_name: "up",
        description: "up",
        default_keys: &["up"],
    },
    ActionSpec {
        action: Action::Down,
        config_name: "down",
        description: "down",
        default_keys: &["down"],
    },
    ActionSpec {
        action: Action::Top,
        config_name: "top",
        description: "top",
        default_keys: &["g"],
    },
    ActionSpec {
        action: Action::Bottom,
        config_name: "bottom",
        description: "bottom",
        default_keys: &["G"],
    },
    ActionSpec {
        action: Action::PageUp,
        config_name: "page_up",
        description: "page up",
        default_keys: &["pgup"],
    },
    ActionSpec {
        action: Action::PageDown,
        config_name: "page_down",
        description: "page down",
        default_keys: &["pgdn"],
    },
    ActionSpec {
        action: Action::HalfPageUp,
        config_name: "half_page_up",
        description: "half page up",
        default_keys: &["ctrl+u"],
    },
    ActionSpec {
        action: Action::HalfPageDown,
        config_name: "half_page_down",
        description: "half page down",
        default_keys: &["ctrl+d"],
    },
    ActionSpec {
        action: Action::TabGit,
        config_name: "tab_git",
        description: "git repos",
        default_keys: &["1"],
    },
    ActionSpec {
        action: Action::TabFiles,
        config_name: "tab_files",
        description: "files",
        default_keys: &["2"],
    },
    ActionSpec {
        action: Action::ToggleSelect,
        config_name: "select",
        description: "select",
        default_keys: &["space"],
    },
    ActionSpec {
        action: Action::ExtendUp,
        config_name: "extend_up",
        description: "extend",
        default_keys: &["shift+up"],
    },
    ActionSpec {
        action: Action::ExtendDown,
        config_name: "extend_down",
        description: "extend",
        default_keys: &["shift+down"],
    },
    ActionSpec {
        action: Action::Jump,
        config_name: "jump",
        description: "cd",
        default_keys: &["enter"],
    },
    ActionSpec {
        action: Action::Open,
        config_name: "open",
        description: "open",
        default_keys: &["L"],
    },
    ActionSpec {
        action: Action::GitTool,
        config_name: "git_tool",
        description: "lazygit",
        default_keys: &["l"],
    },
    ActionSpec {
        action: Action::JumpCd,
        config_name: "jump_cd",
        description: "cd",
        default_keys: &["o"],
    },
    ActionSpec {
        action: Action::OpenApp,
        config_name: "open_app",
        description: "open in app",
        default_keys: &["O"],
    },
    ActionSpec {
        action: Action::Filter,
        config_name: "filter",
        description: "filter",
        default_keys: &["f"],
    },
    ActionSpec {
        action: Action::ChangesFilter,
        config_name: "changes_filter",
        description: "changes",
        default_keys: &["F"],
    },
    ActionSpec {
        action: Action::Github,
        config_name: "github",
        description: "github",
        default_keys: &["b"],
    },
    ActionSpec {
        action: Action::Preview,
        config_name: "preview",
        description: "detail panel",
        default_keys: &["v"],
    },
    ActionSpec {
        action: Action::PreviewPosition,
        config_name: "preview_position",
        description: "right/bottom",
        default_keys: &["V"],
    },
    ActionSpec {
        action: Action::PreviewScrollUp,
        config_name: "preview_scroll_up",
        description: "scroll panel",
        default_keys: &["ctrl+up"],
    },
    ActionSpec {
        action: Action::PreviewScrollDown,
        config_name: "preview_scroll_down",
        description: "scroll panel",
        default_keys: &["ctrl+down"],
    },
    ActionSpec {
        action: Action::PreviewShrink,
        config_name: "preview_shrink",
        description: "smaller",
        default_keys: &["ctrl+left"],
    },
    ActionSpec {
        action: Action::PreviewGrow,
        config_name: "preview_grow",
        description: "bigger",
        default_keys: &["ctrl+right"],
    },
    ActionSpec {
        action: Action::Columns,
        config_name: "columns",
        description: "columns",
        default_keys: &["c"],
    },
    ActionSpec {
        action: Action::Sort,
        config_name: "sort",
        description: "sort",
        default_keys: &["t"],
    },
    ActionSpec {
        action: Action::ToggleGrouping,
        config_name: "grouping",
        description: "group",
        default_keys: &["."],
    },
    ActionSpec {
        action: Action::ToggleFavFloat,
        config_name: "fav_float",
        description: "favs on top",
        default_keys: &[","],
    },
    ActionSpec {
        action: Action::SectionJump,
        config_name: "section_jump",
        description: "jump to section",
        default_keys: &["s"],
    },
    ActionSpec {
        action: Action::ManageSections,
        config_name: "manage_sections",
        description: "sections",
        default_keys: &["M"],
    },
    ActionSpec {
        action: Action::ReorderUp,
        config_name: "reorder_up",
        description: "reorder up",
        default_keys: &["alt+up"],
    },
    ActionSpec {
        action: Action::ReorderDown,
        config_name: "reorder_down",
        description: "reorder down",
        default_keys: &["alt+down"],
    },
    ActionSpec {
        action: Action::Add,
        config_name: "add",
        description: "add",
        default_keys: &["n"],
    },
    ActionSpec {
        action: Action::Edit,
        config_name: "edit",
        description: "edit",
        default_keys: &["e"],
    },
    ActionSpec {
        action: Action::Delete,
        config_name: "delete",
        description: "del",
        default_keys: &["d", "del", "backspace"],
    },
    ActionSpec {
        action: Action::Undo,
        config_name: "undo",
        description: "undo",
        default_keys: &["u"],
    },
    ActionSpec {
        action: Action::ToggleFav,
        config_name: "fav",
        description: "fav",
        default_keys: &["*"],
    },
    ActionSpec {
        action: Action::Zip,
        config_name: "zip",
        description: "zip",
        default_keys: &["z"],
    },
    ActionSpec {
        action: Action::ZipAll,
        config_name: "zip_all",
        description: "zip all",
        default_keys: &["Z"],
    },
    ActionSpec {
        action: Action::Archive,
        config_name: "archive",
        description: "archive/restore",
        default_keys: &["A"],
    },
    ActionSpec {
        action: Action::Slug,
        config_name: "slug",
        description: "slug",
        default_keys: &["S"],
    },
    ActionSpec {
        action: Action::ToggleSlugs,
        config_name: "toggle_slugs",
        description: "slugs",
        default_keys: &["i"],
    },
    ActionSpec {
        action: Action::CopyPath,
        config_name: "copy_path",
        description: "copy path",
        default_keys: &["y"],
    },
    ActionSpec {
        action: Action::RepairPath,
        config_name: "repair_path",
        description: "fix path",
        default_keys: &["p"],
    },
    ActionSpec {
        action: Action::Errors,
        config_name: "errors",
        description: "errors",
        default_keys: &["!"],
    },
    ActionSpec {
        action: Action::Reload,
        config_name: "reload",
        description: "reload/check",
        default_keys: &["r"],
    },
    ActionSpec {
        action: Action::ReloadFetch,
        config_name: "reload_fetch",
        description: "reload+fetch",
        default_keys: &["R"],
    },
    ActionSpec {
        action: Action::RefreshOne,
        config_name: "refresh_one",
        description: "refresh one",
        default_keys: &["x"],
    },
    ActionSpec {
        action: Action::RefreshOneFetch,
        config_name: "refresh_one_fetch",
        description: "refresh+fetch",
        default_keys: &["X"],
    },
    ActionSpec {
        action: Action::Help,
        config_name: "help",
        description: "help",
        default_keys: &["?"],
    },
    ActionSpec {
        action: Action::Quit,
        config_name: "quit",
        description: "quit",
        default_keys: &["q"],
    },
];

impl Action {
    /// Every action, in catalog order.
    pub fn all() -> impl Iterator<Item = Action> + Clone {
        ACTIONS.iter().map(|spec| spec.action)
    }

    /// The catalog row for this action.
    fn spec(self) -> &'static ActionSpec {
        // Every variant has exactly one `ACTIONS` row (guarded by the
        // `actions_table_matches_the_enum` test), so this never fails.
        ACTIONS
            .iter()
            .find(|spec| spec.action == self)
            .expect("every action has an ACTIONS entry")
    }

    /// The `[keys]` config key for this action.
    pub fn config_name(self) -> &'static str {
        self.spec().config_name
    }

    /// A short human description for footer/help hints.
    pub fn description(self) -> &'static str {
        self.spec().description
    }

    /// The default key strings for this action.
    pub fn default_keys(self) -> &'static [&'static str] {
        self.spec().default_keys
    }
}

/// Hands the catalog to the toolkit, which owns the chords.
///
/// Each method delegates to its inherent counterpart above, so the catalog
/// stays the single source and existing call sites need no
/// `use ratada::keymap::Action`.
impl ratada::keymap::Action for Action {
    fn all() -> impl Iterator<Item = Self> + Clone {
        Action::all()
    }

    fn config_name(&self) -> &'static str {
        (*self).config_name()
    }

    fn description(&self) -> &'static str {
        (*self).description()
    }

    fn default_keys(&self) -> &'static [&'static str] {
        (*self).default_keys()
    }
}

#[cfg(test)]
mod tests {
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
                let chord =
                    KeyChord::parse(text).expect("default key must parse");
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
                | Action::PreviewShrink
                | Action::PreviewGrow
                | Action::Columns
                | Action::Sort
                | Action::ToggleGrouping
                | Action::ToggleFavFloat
                | Action::SectionJump
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
}
