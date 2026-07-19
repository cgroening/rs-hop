//! Which actions appear in each tab's footer shortcut hints, grouped and
//! ordered.
//!
//! The keys and descriptions are not stored here: [`App`](crate::tui::App) turns
//! these action lists into `(key, description)` hints via
//! [`Keymap::hints`](crate::keymap::Keymap::hints), so the shown keys always
//! reflect the configured key map. Edit the group tables to reorder or slim the
//! hint bar; keep them in sync with the help overlay (`tui::help`) and the
//! README.

use crate::domain::filter::{Tab, TabKind};
use crate::keymap::Action;

/// The section-management group, shared by both kinds (sections now work on git
/// and files alike).
const SECTIONS_GROUP: (&str, &[Action]) = (
    "Sections",
    &[
        Action::ManageSections,
        Action::SectionJump,
        Action::ToggleGrouping,
        Action::ReorderUp,
        Action::ReorderDown,
    ],
);

/// The view group, shared by both kinds.
const VIEW_GROUP: (&str, &[Action]) = (
    "View",
    &[
        Action::ToggleSelect,
        Action::Filter,
        Action::ChangesFilter,
        Action::Columns,
        Action::Sort,
        Action::ToggleFavFloat,
        Action::ToggleSlugs,
        Action::CopyPath,
        Action::Github,
    ],
);

/// The detail-panel group, shared by both kinds.
const PANEL_GROUP: (&str, &[Action]) = (
    "Panel",
    &[
        Action::Preview,
        Action::PreviewPosition,
        Action::PreviewScrollUp,
        Action::PreviewGrow,
    ],
);

/// The Git Repos tab's footer hints, grouped and ordered.
const GIT_HINT_GROUPS: &[(&str, &[Action])] = &[
    (
        "Open",
        &[
            Action::Jump,
            Action::Open,
            Action::GitTool,
            Action::JumpCd,
            Action::OpenApp,
        ],
    ),
    (
        "Manage",
        &[
            Action::Add,
            Action::Edit,
            Action::Delete,
            Action::ToggleFav,
            Action::Archive,
            Action::Slug,
            Action::Undo,
        ],
    ),
    ("Backup", &[Action::Zip, Action::ZipAll]),
    VIEW_GROUP,
    PANEL_GROUP,
    SECTIONS_GROUP,
    (
        "Git",
        &[
            Action::Reload,
            Action::ReloadFetch,
            Action::RefreshOne,
            Action::RefreshOneFetch,
            Action::RepairPath,
            Action::Errors,
        ],
    ),
];

/// The Files and Folders tab's footer hints. Like the git tabs but without the
/// git tool, and with path checks instead of the git refresh group.
const FILES_HINT_GROUPS: &[(&str, &[Action])] = &[
    (
        "Open",
        &[Action::Jump, Action::Open, Action::JumpCd, Action::OpenApp],
    ),
    (
        "Manage",
        &[
            Action::Add,
            Action::Edit,
            Action::Delete,
            Action::ToggleFav,
            Action::Archive,
            Action::Slug,
            Action::Undo,
        ],
    ),
    ("Backup", &[Action::Zip, Action::ZipAll]),
    VIEW_GROUP,
    PANEL_GROUP,
    SECTIONS_GROUP,
    (
        "Paths",
        &[Action::Reload, Action::RepairPath, Action::Errors],
    ),
];

/// The footer hint groups for `tab`, as `(label, actions)` pairs in display
/// order. Both a kind's active and archive views share the kind's groups.
pub fn hint_groups(tab: Tab) -> &'static [(&'static str, &'static [Action])] {
    match tab.kind() {
        TabKind::Git => GIT_HINT_GROUPS,
        TabKind::Files => FILES_HINT_GROUPS,
    }
}

/// The help overlay's sections: every action, with the longer description the
/// overlay has room for.
///
/// The keys are deliberately absent. They are resolved from the keymap at
/// render time, so a `[keys]` override moves the key in the overlay as well -
/// which spelling them out here would silently prevent.
pub const HELP_SECTIONS: &[(&str, &[(Action, &str)])] = &[
    (
        "Open",
        &[
            (
                Action::Jump,
                "jump only: write path and exit (folder cd, file -> parent)",
            ),
            (
                Action::Open,
                "open: git -> tool \u{b7} folder -> cd \u{b7} text file -> \
                 editor \u{b7} else app",
            ),
            (
                Action::GitTool,
                "git: open the tool (lazygit) as an overlay, then return here",
            ),
            (Action::JumpCd, "jump only: write path and exit"),
            (
                Action::OpenApp,
                "force open with the default app (e.g. a text file in its GUI \
                 app)",
            ),
            (
                Action::Github,
                "open on GitHub in the browser (a tab per selected git repo, \
                 else the cursor)",
            ),
        ],
    ),
    (
        "Manage",
        &[
            (
                Action::Add,
                "add an entry (fill the form; ^O picks the path)",
            ),
            (
                Action::Edit,
                "edit the entry (or bulk-edit when several are selected)",
            ),
            (
                Action::Delete,
                "delete (acts on the selection, else the cursor)",
            ),
            (
                Action::Undo,
                "undo the last change (delete / archive / favourite / edit)",
            ),
            (Action::ToggleFav, "toggle favourite (selection or cursor)"),
            (Action::Archive, "archive / restore (selection or cursor)"),
            (Action::Slug, "set or change the slug"),
            (
                Action::CopyPath,
                "copy path to the clipboard (all selected, one per line, else \
                 the cursor)",
            ),
        ],
    ),
    (
        "Backup",
        &[
            (
                Action::Zip,
                "zip the selected/cursor repo or folder to the backup folder",
            ),
            (
                Action::ZipAll,
                "zip every entry opted into backup (form 'Backup' toggle)",
            ),
        ],
    ),
    (
        "View",
        &[
            (
                Action::ToggleSelect,
                "add or remove the cursor entry from the \
                                    selection",
            ),
            (
                Action::Filter,
                "live fuzzy filter (Esc clears; matches are highlighted)",
            ),
            (
                Action::ChangesFilter,
                "toggle showing only git repos with a status change",
            ),
            (
                Action::Columns,
                "cycle the columns (Standard / Code / Activity)",
            ),
            (
                Action::Sort,
                "pick the column to sort by (again: flip direction)",
            ),
            (
                Action::ToggleFavFloat,
                "toggle floating favourites to the top",
            ),
            (
                Action::ToggleSlugs,
                "toggle the slug column (dim, italic) after the name",
            ),
        ],
    ),
    (
        "Panel",
        &[
            (Action::Preview, "show or hide the detail panel"),
            (Action::PreviewPosition, "move the panel: right / bottom"),
            (Action::PreviewScrollUp, "scroll the panel up"),
            (Action::PreviewScrollDown, "scroll the panel down"),
            (Action::PreviewPageUp, "scroll the panel up by a page"),
            (Action::PreviewPageDown, "scroll the panel down by a page"),
            (Action::PreviewShrink, "make the panel smaller"),
            (Action::PreviewGrow, "make the panel bigger"),
        ],
    ),
    (
        "Sections",
        &[
            (
                Action::SectionJump,
                "jump to a section (in the grouped view)",
            ),
            (
                Action::ManageSections,
                "manage sections (add / rename / delete / move)",
            ),
            (
                Action::ToggleGrouping,
                "toggle grouping into sections (off: flat, global sort)",
            ),
            (
                Action::ReorderUp,
                "move the entry up within its group (custom sort only)",
            ),
            (
                Action::ReorderDown,
                "move the entry down within its group (custom sort only)",
            ),
        ],
    ),
    (
        "Git and paths",
        &[
            (
                Action::Reload,
                "git tabs: reload status \u{b7} Files: check paths exist",
            ),
            (Action::ReloadFetch, "reload status with a git fetch first"),
            (Action::RefreshOne, "refresh the selection or cursor entry"),
            (Action::RefreshOneFetch, "refresh it with a git fetch first"),
            (Action::RepairPath, "repair a missing path"),
            (Action::Errors, "list entries with path errors and fix them"),
        ],
    ),
];

/// The actions shown in the trailing `Global` section of the help overlay and
/// in the footer's compact global group.
pub const GLOBAL_ACTIONS: &[Action] = &[Action::Help, Action::Quit];

/// The actions shown in the leading `Navigation` section.
pub const NAVIGATION_ACTIONS: &[Action] = &[
    Action::Up,
    Action::Down,
    Action::Top,
    Action::Bottom,
    Action::PageUp,
    Action::PageDown,
    Action::HalfPageUp,
    Action::HalfPageDown,
    Action::ExtendUp,
    Action::ExtendDown,
    Action::ExtendPageUp,
    Action::ExtendPageDown,
    Action::TabGit,
    Action::TabFiles,
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every action must appear in exactly one help section (counting the
    /// leading Navigation and trailing Global groups), so the overlay - and the
    /// README table it mirrors - can never silently omit a new action.
    #[test]
    fn the_help_sections_cover_every_action_exactly_once() {
        let mut covered: Vec<Action> = HELP_SECTIONS
            .iter()
            .flat_map(|(_, rows)| rows.iter().map(|(action, _)| *action))
            .collect();
        covered.extend_from_slice(NAVIGATION_ACTIONS);
        covered.extend_from_slice(GLOBAL_ACTIONS);

        for action in Action::all() {
            assert!(
                covered.contains(&action),
                "{action:?} is in no help section; add it to bindings.rs and \
                 keep the README in sync"
            );
        }
        let total = covered.len();
        covered.sort_by_key(|action| format!("{action:?}"));
        covered.dedup();
        assert_eq!(
            covered.len(),
            total,
            "an action is listed in two help sections"
        );
    }

    /// A footer hint can only be drawn for an action the keymap knows, and each
    /// group should stay free of duplicates.
    #[test]
    fn no_tab_lists_the_same_action_twice_in_its_footer() {
        for groups in [GIT_HINT_GROUPS, FILES_HINT_GROUPS] {
            let mut seen: Vec<Action> = groups
                .iter()
                .flat_map(|(_, actions)| actions.iter().copied())
                .collect();
            let total = seen.len();
            seen.sort_by_key(|action| format!("{action:?}"));
            seen.dedup();
            assert_eq!(seen.len(), total, "a footer lists an action twice");
        }
    }
}
