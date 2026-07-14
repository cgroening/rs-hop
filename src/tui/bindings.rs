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
