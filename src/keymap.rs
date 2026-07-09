//! Configurable key bindings for hop's own shortcuts.
//!
//! Maps key chords to [`Action`]s, built from compiled-in defaults plus
//! per-action overrides from `[keys]` in config. Kept free of UI-framework
//! widgets so both the TUI (dispatch + footer/help hints) and the CLI use it.
//! Widget-internal shortcuts (modals, pickers, forms) are not covered here.
//!
//! Some keys are context-dependent (`s` sorts on the git tabs and jumps to a
//! section on the Files tab; `r`/`R` reload on the git tabs and re-check paths
//! on the Files tab); the keymap resolves the key to an [`Action`] and the app
//! interprets it per tab. `Shift`+arrow (extend selection) and `Shift`+`Tab`
//! (cycle back) are handled inline in dispatch, because [`KeyChord`] ignores the
//! shift modifier (it is encoded only in a character's case).

use std::collections::BTreeMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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
    /// Switch to the Git Repos tab.
    TabGit,
    /// Switch to the Files and Folders tab.
    TabFiles,
    /// Switch to the Archive tab.
    TabArchive,
    /// Toggle the current row's multi-selection.
    ToggleSelect,
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
    /// Cycle the detail preview panel.
    Preview,
    /// Cycle the sort mode (git tabs) or jump to a section (Files tab).
    Sort,
    /// Manage sections (Files tab).
    ManageSections,
    /// Reorder the cursor entry up (custom sort / within a section).
    ReorderUp,
    /// Reorder the cursor entry down.
    ReorderDown,
    /// Jump to the previous section (Files tab).
    SectionPrev,
    /// Jump to the next section (Files tab).
    SectionNext,
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
        action: Action::TabArchive,
        config_name: "tab_archive",
        description: "archive",
        default_keys: &["3"],
    },
    ActionSpec {
        action: Action::ToggleSelect,
        config_name: "select",
        description: "select",
        default_keys: &["space"],
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
        description: "preview",
        default_keys: &["v"],
    },
    ActionSpec {
        action: Action::Sort,
        config_name: "sort",
        description: "sort/section",
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
        action: Action::SectionPrev,
        config_name: "section_prev",
        description: "prev section",
        default_keys: &["ctrl+up"],
    },
    ActionSpec {
        action: Action::SectionNext,
        config_name: "section_next",
        description: "next section",
        default_keys: &["ctrl+down"],
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

    fn from_config_name(name: &str) -> Option<Action> {
        Action::all().find(|a| a.config_name() == name)
    }
}

/// A parsed key chord: a key plus the `ctrl`/`alt` modifiers. `shift` is encoded
/// in the character's case and otherwise ignored when matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyChord {
    code: KeyCode,
    ctrl: bool,
    alt: bool,
}

impl KeyChord {
    /// Parses a chord like `"a"`, `"G"`, `"ctrl+u"`, `"alt+up"`, `"pgup"` or
    /// `"enter"`. Returns `None` for an unrecognised string.
    pub fn parse(text: &str) -> Option<KeyChord> {
        let parts: Vec<&str> = text
            .split('+')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .collect();
        // The final token is the key; anything before it is a modifier.
        let (code_token, modifiers) = parts.split_last()?;
        let mut ctrl = false;
        let mut alt = false;
        for modifier in modifiers {
            match modifier.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => ctrl = true,
                "alt" | "option" => alt = true,
                "shift" => {}
                _ => return None,
            }
        }
        let code = code_from_token(code_token)?;
        Some(KeyChord { code, ctrl, alt })
    }

    /// Whether `key` triggers this chord (code plus ctrl/alt; shift ignored).
    pub fn matches(&self, key: &KeyEvent) -> bool {
        self.code == key.code
            && self.ctrl == key.modifiers.contains(KeyModifiers::CONTROL)
            && self.alt == key.modifiers.contains(KeyModifiers::ALT)
    }

    /// A display string for hints, e.g. `ctrl+u`, `alt+up`, `pgup`, `G`.
    pub fn display(&self) -> String {
        let mut text = String::new();
        if self.ctrl {
            text.push_str("ctrl+");
        }
        if self.alt {
            text.push_str("alt+");
        }
        text.push_str(&token_for_code(self.code));
        text
    }
}

/// Parses a single key token (no modifiers) into a [`KeyCode`].
fn code_from_token(token: &str) -> Option<KeyCode> {
    let lower = token.to_ascii_lowercase();
    let code = match lower.as_str() {
        "enter" | "return" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "space" => KeyCode::Char(' '),
        "backspace" => KeyCode::Backspace,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "pgup" | "pageup" => KeyCode::PageUp,
        "pgdn" | "pgdown" | "pagedown" => KeyCode::PageDown,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "del" | "delete" => KeyCode::Delete,
        _ => return function_or_char(token, &lower),
    };
    Some(code)
}

/// Resolves an `fn` function key or a single character (preserving case).
fn function_or_char(token: &str, lower: &str) -> Option<KeyCode> {
    if let Some(digits) = lower.strip_prefix('f')
        && let Ok(number) = digits.parse::<u8>()
        && (1..=12).contains(&number)
    {
        return Some(KeyCode::F(number));
    }
    let mut chars = token.chars();
    let first = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    Some(KeyCode::Char(first))
}

/// The display token for a key code (inverse of [`code_from_token`]).
fn token_for_code(code: KeyCode) -> String {
    match code {
        KeyCode::Char(' ') => "space".to_string(),
        KeyCode::Char(ch) => ch.to_string(),
        KeyCode::Enter => "enter".to_string(),
        KeyCode::Esc => "esc".to_string(),
        KeyCode::Tab => "tab".to_string(),
        KeyCode::Backspace => "backspace".to_string(),
        KeyCode::Up => "up".to_string(),
        KeyCode::Down => "down".to_string(),
        KeyCode::Left => "left".to_string(),
        KeyCode::Right => "right".to_string(),
        KeyCode::PageUp => "pgup".to_string(),
        KeyCode::PageDown => "pgdn".to_string(),
        KeyCode::Home => "home".to_string(),
        KeyCode::End => "end".to_string(),
        KeyCode::Delete => "del".to_string(),
        KeyCode::F(number) => format!("f{number}"),
        _ => "?".to_string(),
    }
}

/// A configured key that was dropped because an earlier action already claimed
/// it. Surfaced so a silently shadowed binding is visible.
#[derive(Debug, Clone)]
pub struct Conflict {
    /// The contested key, as a display string (e.g. `"d"`).
    pub key: String,
    /// The action whose binding was dropped.
    pub action: Action,
    /// The action that already owns the key.
    pub claimed_by: Action,
}

/// The resolved key map: chords paired with the action they trigger.
#[derive(Debug, Clone)]
pub struct Keymap {
    entries: Vec<(KeyChord, Action)>,
    conflicts: Vec<Conflict>,
}

impl Default for Keymap {
    /// The compiled-in default key map.
    fn default() -> Self {
        Self::from_overrides(&BTreeMap::new())
    }
}

impl Keymap {
    /// Builds the map from defaults, replacing an action's keys if `overrides`
    /// names it. Unknown action names and unparseable keys are logged and
    /// skipped; a key already bound to an earlier action keeps that binding.
    pub fn from_overrides(overrides: &BTreeMap<String, Vec<String>>) -> Self {
        for name in overrides.keys() {
            if Action::from_config_name(name).is_none() {
                log::warn!("unknown key action '{name}' in config, ignoring");
            }
        }
        let mut entries: Vec<(KeyChord, Action)> = Vec::new();
        let mut conflicts: Vec<Conflict> = Vec::new();
        for action in Action::all() {
            for key in override_keys(overrides, action) {
                bind_key(&mut entries, &mut conflicts, action, &key);
            }
        }
        Self { entries, conflicts }
    }

    /// The bindings that were dropped because an earlier action owned the key.
    pub fn conflicts(&self) -> &[Conflict] {
        &self.conflicts
    }

    /// The action bound to `key`, if any.
    pub fn action_for(&self, key: &KeyEvent) -> Option<Action> {
        self.entries
            .iter()
            .find(|(chord, _)| chord.matches(key))
            .map(|(_, action)| *action)
    }

    /// The display strings of the keys bound to `action`.
    pub fn keys_for(&self, action: Action) -> Vec<String> {
        self.entries
            .iter()
            .filter(|(_, bound)| *bound == action)
            .map(|(chord, _)| chord.display())
            .collect()
    }

    /// Builds `(keys, description)` hint pairs for `actions`, skipping any with
    /// no bound key. The single source for footer and help content.
    pub fn hints(&self, actions: &[Action]) -> Vec<(String, String)> {
        actions
            .iter()
            .filter_map(|&action| {
                let keys = self.keys_for(action).join("/");
                if keys.is_empty() {
                    None
                } else {
                    Some((keys, action.description().to_string()))
                }
            })
            .collect()
    }
}

/// Binds `key` to `action` in `entries`, or records the reason it was dropped:
/// an unparseable key is logged, and a key already claimed by an earlier action
/// is logged and pushed to `conflicts` (the earlier binding wins).
fn bind_key(
    entries: &mut Vec<(KeyChord, Action)>,
    conflicts: &mut Vec<Conflict>,
    action: Action,
    key: &str,
) {
    let Some(chord) = KeyChord::parse(key) else {
        log::warn!("invalid key '{key}' for '{}'", action.config_name());
        return;
    };
    if let Some((_, owner)) =
        entries.iter().find(|(existing, _)| *existing == chord)
    {
        log::warn!(
            "key '{key}' already bound to '{}', ignoring for '{}'",
            owner.config_name(),
            action.config_name()
        );
        conflicts.push(Conflict {
            key: chord.display(),
            action,
            claimed_by: *owner,
        });
        return;
    }
    entries.push((chord, action));
}

/// The key strings to use for `action`: the override if present, else defaults.
fn override_keys(
    overrides: &BTreeMap<String, Vec<String>>,
    action: Action,
) -> Vec<String> {
    overrides
        .get(action.config_name())
        .cloned()
        .unwrap_or_else(|| {
            action
                .default_keys()
                .iter()
                .map(|s| (*s).to_string())
                .collect()
        })
}

#[cfg(test)]
mod tests {
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
                let mut modifiers = KeyModifiers::NONE;
                if chord.ctrl {
                    modifiers |= KeyModifiers::CONTROL;
                }
                if chord.alt {
                    modifiers |= KeyModifiers::ALT;
                }
                assert_eq!(
                    map.action_for(&key(chord.code, modifiers)),
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
                | Action::TabArchive
                | Action::ToggleSelect
                | Action::Jump
                | Action::Open
                | Action::GitTool
                | Action::JumpCd
                | Action::OpenApp
                | Action::Filter
                | Action::ChangesFilter
                | Action::Github
                | Action::Preview
                | Action::Sort
                | Action::ManageSections
                | Action::ReorderUp
                | Action::ReorderDown
                | Action::SectionPrev
                | Action::SectionNext
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
