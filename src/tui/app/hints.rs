//! Where the footer hints and the help overlay get their rows from.
//!
//! No key is spelled out here: every one is resolved from the keymap at render
//! time, so a `[keys]` override shows up in the footer, the overlay and the
//! `Navigation` group at once. The section catalogue itself lives in
//! [`bindings`](crate::tui::bindings).

use super::*;

use ratada::shortcut_hints;

use crate::keymap::{Action, Keymap};
use crate::tui::bindings::{GLOBAL_ACTIONS, HELP_SECTIONS};
use crate::tui::help::Section;

/// The label of the group and help section listing the app-wide chords.
pub(super) const GLOBAL_GROUP: &str = "Global";
/// The label of the help section listing the cursor and selection keys.
pub(super) const NAVIGATION_GROUP: &str = "Navigation";

/// The `Navigation` rows, each merging several actions into one legible row
/// (`↑ ↓` reads better than two separate lines).
///
/// The keys come from the keymap; only the wording is fixed here, because the
/// overlay has room for a fuller description than the footer's terse one.
const NAVIGATION_ROWS: &[(&[Action], &str)] = &[
    (
        &[Action::TabGit, Action::TabFiles],
        "Git Repos / Files (press again for that kind's archive)",
    ),
    (&[Action::Up, Action::Down], "move cursor (wraps)"),
    (&[Action::Top, Action::Bottom], "top / bottom"),
    (&[Action::PageUp, Action::PageDown], "page up / down"),
    (
        &[Action::HalfPageUp, Action::HalfPageDown],
        "half page up / down",
    ),
    (&[Action::ToggleSelect], "toggle the selection"),
    (
        &[Action::ExtendUp, Action::ExtendDown],
        "extend the selection by a row",
    ),
    (
        &[Action::ExtendPageUp, Action::ExtendPageDown],
        "extend the selection by a page",
    ),
];

/// The structural keys the keymap deliberately does not own, so they can never
/// be rebound and are therefore safe to spell out.
const STRUCTURAL_ROWS: &[(&str, &str)] = &[
    ("Tab / Shift+Tab", "cycle the two active tabs"),
    ("Esc", "clear the selection"),
];

/// The app-wide chords: this app's own keys, resolved through the keymap,
/// followed by the ones the toolkit intercepts itself (the hints toggle and the
/// hard quit).
pub(super) fn global_group(keymap: &Keymap) -> Section {
    let mut hints = keymap.hints(GLOBAL_ACTIONS);
    hints.extend(shortcut_hints::global_bindings());
    (GLOBAL_GROUP.to_string(), hints)
}

/// The help overlay's leading section: the merged movement rows, then the
/// structural keys.
pub(super) fn navigation_group(keymap: &Keymap) -> Section {
    let mut hints: Vec<(String, String)> = Vec::new();
    for (actions, description) in NAVIGATION_ROWS {
        let keys = keys_of(keymap, actions);
        // An unbound action gets no row, as `Keymap::hints` does it.
        if !keys.is_empty() {
            hints.push((keys, (*description).to_string()));
        }
    }
    hints.extend(
        STRUCTURAL_ROWS
            .iter()
            .map(|(key, text)| ((*key).to_string(), (*text).to_string())),
    );
    (NAVIGATION_GROUP.to_string(), hints)
}

/// The keys bound to `actions`, joined into one help row.
///
/// Not `Keymap::hints`: that pairs the keys with the terse footer wording
/// ("extend"), while the overlay keeps its own fuller description.
pub(super) fn keys_of(keymap: &Keymap, actions: &[Action]) -> String {
    actions
        .iter()
        .flat_map(|&action| keymap.keys_for(action))
        .collect::<Vec<String>>()
        .join("/")
}

impl App {
    /// The per-tab footer hints as labelled groups (clibase-style). A compact
    /// Navigation group leads, then each `bindings` group is turned into
    /// `(key, description)` pairs via the keymap, so the shown keys reflect any
    /// `[keys]` overrides. Empty groups (no bound key) are dropped.
    pub(super) fn hint_groups(&self) -> Vec<(String, Vec<(String, String)>)> {
        let keymap = &self.keymap;
        let compact = [
            (&[Action::Up, Action::Down][..], "move"),
            (&[Action::Top, Action::Bottom][..], "top/bottom"),
            (&[Action::PageUp, Action::PageDown][..], "page"),
            (&[Action::HalfPageUp, Action::HalfPageDown][..], "half"),
        ];
        let navigation = (
            NAVIGATION_GROUP.to_string(),
            compact
                .into_iter()
                .map(|(actions, text)| {
                    (keys_of(keymap, actions), text.to_string())
                })
                .filter(|(keys, _)| !keys.is_empty())
                .collect(),
        );
        let mut groups = vec![navigation];
        for (label, actions) in crate::tui::bindings::hint_groups(self.tab) {
            let hints = keymap.hints(actions);
            if !hints.is_empty() {
                groups.push(((*label).to_string(), hints));
            }
        }
        groups.push(global_group(keymap));
        groups
    }

    /// Every help-overlay section, in display order, with the keys resolved
    /// from the keymap.
    pub(super) fn help_sections(&self) -> Vec<Section> {
        let keymap = &self.keymap;
        let mut sections = vec![navigation_group(keymap)];
        for (title, rows) in HELP_SECTIONS {
            let hints: Vec<(String, String)> = rows
                .iter()
                .filter_map(|(action, description)| {
                    let keys = keymap.keys_for(*action).join("/");
                    // An action nobody can reach gets no row.
                    (!keys.is_empty())
                        .then(|| (keys, (*description).to_string()))
                })
                .collect();
            if !hints.is_empty() {
                sections.push(((*title).to_string(), hints));
            }
        }
        sections.push(global_group(keymap));
        sections
    }
}
