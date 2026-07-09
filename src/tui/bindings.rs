//! Which actions appear in each tab's footer shortcut hints, grouped and
//! ordered.
//!
//! The keys and descriptions are not stored here: [`App`](crate::tui::App) turns
//! these action lists into `(key, description)` hints via
//! [`Keymap::hints`](crate::keymap::Keymap::hints), so the shown keys always
//! reflect the configured key map. Edit the group tables to reorder or slim the
//! hint bar; keep them in sync with the help overlay (`tui::help`) and the
//! README.

use crate::domain::filter::Tab;
use crate::keymap::Action;

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
    (
        "View",
        &[
            Action::ToggleSelect,
            Action::Filter,
            Action::ChangesFilter,
            Action::Sort,
            Action::Preview,
            Action::ToggleSlugs,
            Action::CopyPath,
            Action::Github,
        ],
    ),
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
    ("App", &[Action::Help, Action::Quit]),
];

/// The Archive tab's footer hints. Archived entries are git repos, so the tab
/// shares the Git Repos groups (the neutral "archive/restore" description
/// covers the restore action).
const ARCHIVE_HINT_GROUPS: &[(&str, &[Action])] = GIT_HINT_GROUPS;

/// The Files and Folders tab's footer hints. Like the git tabs but without the
/// git tool, and with section management and path checks instead of the git
/// refresh group.
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
    (
        "View",
        &[
            Action::ToggleSelect,
            Action::Filter,
            Action::ChangesFilter,
            Action::Sort,
            Action::Preview,
            Action::ToggleSlugs,
            Action::CopyPath,
            Action::Github,
        ],
    ),
    (
        "Sections",
        &[
            Action::ManageSections,
            Action::SectionPrev,
            Action::SectionNext,
            Action::ReorderUp,
            Action::ReorderDown,
        ],
    ),
    (
        "Paths",
        &[Action::Reload, Action::RepairPath, Action::Errors],
    ),
    ("App", &[Action::Help, Action::Quit]),
];

/// The footer hint groups for `tab`, as `(label, actions)` pairs in display
/// order.
pub fn hint_groups(tab: Tab) -> &'static [(&'static str, &'static [Action])] {
    match tab {
        Tab::GitRepos => GIT_HINT_GROUPS,
        Tab::FilesAndFolders => FILES_HINT_GROUPS,
        Tab::Archive => ARCHIVE_HINT_GROUPS,
    }
}
