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
//! interprets it per tab. `Shift`+`Tab` (cycle back) is still handled inline
//! in dispatch, because tab cycling is structural rather than a user-facing
//! action.
//!
//! The chord grammar, the defaults-vs-overrides merge and the display form live
//! in [`ratada::keymap`]; this module owns only the action catalog.

pub use ratada::keymap::{Conflict as ChordConflict, KeyChord};

/// The resolved key map for hop's actions.
pub type Keymap = ratada::keymap::Keymap<Action>;

/// A configured key dropped because an earlier action already claimed it.
pub type Conflict = ChordConflict<Action>;

mod catalog;
use catalog::{ACTIONS, ActionSpec};

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
    /// Extend the selection up by a page.
    ExtendPageUp,
    /// Extend the selection down by a page.
    ExtendPageDown,
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
    /// Scroll the detail panel up by a page.
    PreviewPageUp,
    /// Scroll the detail panel down by a page.
    PreviewPageDown,
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
    /// Jump to the start of the current (then the previous) section.
    SectionPrev,
    /// Jump to the start of the next section.
    SectionNext,
    /// Collapse the cursor's section onto its header, or expand it again.
    ToggleCollapse,
    /// Collapse every section, or expand them all when none is open.
    ToggleCollapseAll,
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
    /// `git pull --ff-only` the selection or cursor entry (git tabs).
    Pull,
    /// `git push` the selection or cursor entry (git tabs).
    Push,
    /// `git pull --ff-only` every git entry of the current tab, after asking.
    PullAll,
    /// `git push` every git entry of the current tab, after asking.
    PushAll,
    /// Toggle the help overlay.
    Help,
    /// Quit the application.
    Quit,
}

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
mod tests;
