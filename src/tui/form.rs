//! The add/edit form for an entry: path, name, section, slug, kind, favourite.
//!
//! Which fields show depends on the current kind: a git entry shows the
//! Favourite toggle, a folder/file shows the Section dropdown; changing the
//! Kind field updates this live. `Tab`/`BackTab` (or `Up`/`Down`) step between
//! the visible fields, the text fields edit inline, `Left`/`Right` cycle the
//! section and the kind, `Space` toggles the favourite, `^O` opens the path
//! picker, `Enter`/`Ctrl+S` save and `Esc` cancels. When the name is still
//! blank, it is auto-filled from the path's basename (after picking a path, or
//! when the name field is focused).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratada::input::InputField;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::domain::repo::{Repo, RepoKind};
use crate::domain::sections::UNGROUPED;
use crate::domain::slug::slugify;
use crate::theme::Skin;
use crate::tui::presentation::{FieldView, field_spans};
use crate::tui::skin::Colors;
use crate::tui::widgets::centered_rect;

/// Display columns reserved for a field's label, before its value.
const LABEL_WIDTH: usize = 8;

/// What a field line needs to draw itself, bundled to keep the parameter count
/// low.
struct LineCtx<'a> {
    /// The colour roles resolved from the active theme.
    colors: &'a Colors,
    /// The active theme, whose palette colours the caret of a text field.
    skin: &'a Skin,
    /// The display columns a field's value may occupy, after its label.
    value_width: usize,
}

/// One editable field of the form.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Path,
    Name,
    Section,
    Slug,
    Kind,
    Fav,
    Backup,
}

/// The values captured by the form on save.
pub struct RepoDraft {
    /// Explicit name, or `None` to fall back to the path's basename.
    pub name: Option<String>,
    /// The entry path as typed.
    pub path: String,
    /// Slug, or `None` when blank.
    pub slug: Option<String>,
    /// Section name, or `None` for Ungrouped.
    pub section: Option<String>,
    /// The entry kind.
    pub kind: RepoKind,
    /// Whether the entry is a favourite.
    pub fav: bool,
    /// Whether the "backup all" (`Z`) run includes this entry.
    pub include_in_backup: bool,
}

/// Outcome of feeding a key to the form.
pub enum FormResult {
    /// Still editing.
    Pending,
    /// The user saved these values.
    Save(RepoDraft),
    /// The user asked to pick the path with the filesystem picker.
    PickPath,
    /// The user cancelled.
    Cancel,
}

/// Add/edit form state.
pub struct RepoForm {
    title: String,
    name: InputField,
    path: InputField,
    slug: InputField,
    kind: RepoKind,
    fav: bool,
    include_in_backup: bool,
    /// The git-namespace section options: `None` (Ungrouped) first, then each
    /// known section. Shown when the current kind is git.
    git_section_options: Vec<Option<String>>,
    /// The files-namespace section options, shown when the kind is a path.
    path_section_options: Vec<Option<String>>,
    /// The selected index into the current kind's section options.
    section_choice: usize,
    /// The original section, kept for entries whose Section field is hidden.
    seed_section: Option<String>,
    /// The focused field's position in the currently visible fields.
    focus: usize,
}

impl RepoForm {
    /// A blank add form, seeded with a `path` and guessed `kind`. The backup
    /// toggle defaults per kind: on for git repos, off for file/folder entries.
    pub fn for_add(
        path: &str,
        kind: RepoKind,
        git_sections: &[String],
        path_sections: &[String],
    ) -> Self {
        Self::build(
            "Add entry",
            "",
            path,
            "",
            kind,
            false,
            kind == RepoKind::Git,
            None,
            git_sections,
            path_sections,
        )
    }

    /// An edit form seeded from an existing entry's fields.
    pub fn for_edit(
        repo: &Repo,
        git_sections: &[String],
        path_sections: &[String],
    ) -> Self {
        Self::build(
            "Edit entry",
            repo.name.as_deref().unwrap_or(""),
            &repo.path.to_string_lossy(),
            repo.slug.as_deref().unwrap_or(""),
            repo.kind,
            repo.fav,
            repo.include_in_backup,
            repo.section.clone(),
            git_sections,
            path_sections,
        )
    }

    /// Builds a form from explicit seed values and both kinds' section names.
    #[allow(clippy::too_many_arguments)]
    fn build(
        title: &str,
        name: &str,
        path: &str,
        slug: &str,
        kind: RepoKind,
        fav: bool,
        include_in_backup: bool,
        section: Option<String>,
        git_sections: &[String],
        path_sections: &[String],
    ) -> Self {
        let git_section_options = section_options(git_sections);
        let path_section_options = section_options(path_sections);
        let current = match kind {
            RepoKind::Git => &git_section_options,
            RepoKind::Path => &path_section_options,
        };
        let section_choice = section_choice(current, section.as_deref());
        RepoForm {
            title: title.to_string(),
            name: InputField::new(name),
            path: InputField::new(path),
            slug: InputField::new(slug),
            kind,
            fav,
            include_in_backup,
            git_section_options,
            path_section_options,
            section_choice,
            seed_section: section,
            focus: 0,
        }
    }

    /// The fields visible for the current kind, in focus order. Both kinds show
    /// the Section dropdown (each namespace is per kind); only git shows Fav.
    fn fields(&self) -> Vec<Field> {
        let mut fields =
            vec![Field::Path, Field::Name, Field::Section, Field::Slug];
        if self.kind == RepoKind::Git {
            fields.push(Field::Fav);
        }
        // The backup toggle shows for both kinds; Kind sits at the very bottom.
        fields.push(Field::Backup);
        fields.push(Field::Kind);
        fields
    }

    /// The section dropdown options for the current kind.
    fn section_options(&self) -> &[Option<String>] {
        match self.kind {
            RepoKind::Git => &self.git_section_options,
            RepoKind::Path => &self.path_section_options,
        }
    }

    /// Handles a key, returning a save draft, a cancel, or pending.
    pub fn handle_key(&mut self, key: KeyEvent) -> FormResult {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => return FormResult::Cancel,
            KeyCode::Enter => return FormResult::Save(self.draft()),
            KeyCode::Char('s') if ctrl => {
                return FormResult::Save(self.draft());
            }
            KeyCode::Char('o') if ctrl => return FormResult::PickPath,
            KeyCode::Tab | KeyCode::Down => self.move_focus(1),
            KeyCode::BackTab | KeyCode::Up => {
                self.move_focus(self.fields().len() - 1)
            }
            _ => self.edit_focused(key),
        }
        FormResult::Pending
    }

    /// Moves focus by `delta` (wrapping), auto-filling the name on arrival.
    fn move_focus(&mut self, delta: usize) {
        let fields = self.fields();
        self.focus = (self.focus + delta) % fields.len();
        if fields[self.focus] == Field::Name {
            self.autofill_name();
        }
    }

    /// Routes an editing key to the focused field.
    fn edit_focused(&mut self, key: KeyEvent) {
        match self.focused_field() {
            Field::Path => {
                self.path.handle_key(key);
            }
            Field::Name => {
                self.name.handle_key(key);
            }
            Field::Slug => {
                self.slug.handle_key(key);
            }
            Field::Section => match key.code {
                KeyCode::Left => self.cycle_section(-1),
                KeyCode::Right => self.cycle_section(1),
                _ => {}
            },
            Field::Kind => self.edit_kind(key),
            Field::Fav => {
                if key.code == KeyCode::Char(' ') {
                    self.fav = !self.fav;
                }
            }
            Field::Backup => {
                if key.code == KeyCode::Char(' ') {
                    self.include_in_backup = !self.include_in_backup;
                }
            }
        }
    }

    /// Cycles the kind on `Left`/`Right`, keeping focus on the Kind field even
    /// though the visible field set changes with the kind. The section choice is
    /// re-resolved against the new kind's namespace, preserving it by name.
    fn edit_kind(&mut self, key: KeyEvent) {
        let changed = match key.code {
            KeyCode::Left => {
                self.kind = prev_kind(self.kind);
                true
            }
            KeyCode::Right => {
                self.kind = next_kind(self.kind);
                true
            }
            _ => false,
        };
        if changed {
            let selected = self.selected_section().map(str::to_string);
            self.section_choice =
                section_choice(self.section_options(), selected.as_deref());
            self.focus = self.field_index(Field::Kind);
        }
    }

    /// The currently selected section name, or `None` for Ungrouped.
    fn selected_section(&self) -> Option<&str> {
        match self.section_options().get(self.section_choice) {
            Some(Some(name)) => Some(name),
            _ => None,
        }
    }

    /// The field currently focused.
    fn focused_field(&self) -> Field {
        self.fields()[self.focus]
    }

    /// The position of `field` in the current layout, or 0 when absent.
    fn field_index(&self, field: Field) -> usize {
        self.fields()
            .iter()
            .position(|candidate| *candidate == field)
            .unwrap_or(0)
    }

    /// Cycles the section choice by `delta` (wrapping).
    fn cycle_section(&mut self, delta: isize) {
        let len = self.section_options().len();
        if len == 0 {
            return;
        }
        let next =
            (self.section_choice as isize + delta).rem_euclid(len as isize);
        self.section_choice = next as usize;
    }

    /// The path field's current value (used to seed the path picker).
    pub fn path_value(&self) -> String {
        self.path.value().to_string()
    }

    /// Replaces the path field (after picking a path) and auto-fills the name
    /// from its basename when the name is still blank.
    pub fn set_path(&mut self, path: &str) {
        self.path = InputField::new(path);
        self.autofill_name();
        self.focus = 0;
    }

    /// Fills the name from the path's basename when the name is still blank.
    fn autofill_name(&mut self) {
        if !self.name.value().trim().is_empty() {
            return;
        }
        if let Some(base) = basename(self.path.value()) {
            self.name = InputField::new(&base);
        }
    }

    /// Builds the draft from the current field values.
    fn draft(&self) -> RepoDraft {
        let name = non_empty(self.name.value().to_string());
        let slug = non_empty(slugify(self.slug.value()));
        let section = if self.fields().contains(&Field::Section) {
            self.section_options()
                .get(self.section_choice)
                .cloned()
                .flatten()
        } else {
            self.seed_section.clone()
        };
        RepoDraft {
            name,
            path: self.path.value().to_string(),
            slug,
            section,
            kind: self.kind,
            fav: self.fav,
            include_in_backup: self.include_in_backup,
        }
    }

    /// Renders the form centred in `area`.
    pub fn render(&self, frame: &mut Frame, area: Rect, skin: &Skin) {
        let colors = Colors::from_palette(&skin.palette);
        let fields = self.fields();
        let height = fields.len() as u16 + 4;
        let rect = centered_rect(70, height, area);
        frame.render_widget(Clear, rect);
        let block = ratada::chrome::modal_block(skin, &self.title);
        let inner_width = block.inner(rect).width as usize;
        let ctx = LineCtx {
            colors: &colors,
            skin,
            value_width: inner_width.saturating_sub(LABEL_WIDTH),
        };
        let mut lines: Vec<Line> = fields
            .iter()
            .enumerate()
            .map(|(index, field)| self.field_line(*field, index, &ctx))
            .collect();
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            "Tab field · \u{2190}\u{2192} change · Space fav · ^O pick path · \
             Enter save · Esc cancel",
            Style::default().fg(colors.dim),
        )));
        frame.render_widget(Paragraph::new(lines).block(block), rect);
    }

    /// Builds the rendered line for `field` at focus position `index`.
    fn field_line(
        &self,
        field: Field,
        index: usize,
        ctx: &LineCtx,
    ) -> Line<'static> {
        let colors = ctx.colors;
        match field {
            Field::Path => self.text_line("Path", &self.path, index, ctx),
            Field::Name => self.text_line("Name", &self.name, index, ctx),
            Field::Slug => self.text_line("Slug", &self.slug, index, ctx),
            Field::Section => {
                let label = self.section_label();
                self.choice_line("Section", label, index, colors)
            }
            Field::Kind => {
                self.choice_line("Kind", kind_label(self.kind), index, colors)
            }
            Field::Fav => self.fav_line(index, colors),
            Field::Backup => self.backup_line(index, colors),
        }
    }

    /// A labelled text field line, highlighting it when focused. The value is
    /// drawn by the toolkit, so a long path scrolls under the caret instead of
    /// spilling out of the box.
    fn text_line(
        &self,
        label: &str,
        input: &InputField,
        index: usize,
        ctx: &LineCtx,
    ) -> Line<'static> {
        let mut spans = vec![self.label_span(label, index, ctx.colors)];
        spans.extend(field_spans(FieldView {
            field: input,
            palette: &ctx.skin.palette,
            width: ctx.value_width,
            focused: index == self.focus,
        }));
        self.styled(spans, index, ctx.colors)
    }

    /// A `< value >` selector line (section, kind).
    fn choice_line(
        &self,
        label: &str,
        value: &str,
        index: usize,
        colors: &Colors,
    ) -> Line<'static> {
        let spans = vec![
            self.label_span(label, index, colors),
            Span::styled(
                format!("< {value} >"),
                Style::default().fg(colors.accent),
            ),
        ];
        self.styled(spans, index, colors)
    }

    /// The favourite toggle line.
    fn fav_line(&self, index: usize, colors: &Colors) -> Line<'static> {
        let mark = if self.fav { "[x]" } else { "[ ]" };
        let spans = vec![
            self.label_span("Fav", index, colors),
            Span::raw(mark.to_string()),
        ];
        self.styled(spans, index, colors)
    }

    /// The "include in backup all (`Z`)" toggle line.
    fn backup_line(&self, index: usize, colors: &Colors) -> Line<'static> {
        let mark = if self.include_in_backup { "[x]" } else { "[ ]" };
        let spans = vec![
            self.label_span("Backup", index, colors),
            Span::raw(mark.to_string()),
        ];
        self.styled(spans, index, colors)
    }

    /// The label of the currently selected section option.
    fn section_label(&self) -> &str {
        self.selected_section().unwrap_or(UNGROUPED)
    }

    /// The fixed-width field label, accented when focused.
    fn label_span(
        &self,
        label: &str,
        index: usize,
        colors: &Colors,
    ) -> Span<'static> {
        let style = if index == self.focus {
            Style::default()
                .fg(colors.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(colors.dim)
        };
        Span::styled(format!("{label:<LABEL_WIDTH$}"), style)
    }

    /// Applies the focus background tint to a field line.
    fn styled(
        &self,
        spans: Vec<Span<'static>>,
        index: usize,
        colors: &Colors,
    ) -> Line<'static> {
        let line = Line::from(spans);
        if index == self.focus {
            line.style(Style::default().bg(colors.selection_bg))
        } else {
            line
        }
    }
}

/// The Section dropdown options: Ungrouped (`None`) first, then each section.
fn section_options(sections: &[String]) -> Vec<Option<String>> {
    let mut options = vec![None];
    options.extend(sections.iter().cloned().map(Some));
    options
}

/// The option index matching `section` (case-insensitive), or 0 (Ungrouped).
fn section_choice(options: &[Option<String>], section: Option<&str>) -> usize {
    let Some(name) = section else {
        return 0;
    };
    options
        .iter()
        .position(|option| {
            option
                .as_deref()
                .is_some_and(|o| o.eq_ignore_ascii_case(name))
        })
        .unwrap_or(0)
}

/// The final path component of `path`, or `None` when it has none.
fn basename(path: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    std::path::Path::new(trimmed)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

/// Returns `Some(value)` when non-empty after trimming, else `None`.
fn non_empty(value: String) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

/// The next kind in the cycle (git <-> file/folder).
fn next_kind(kind: RepoKind) -> RepoKind {
    toggle_kind(kind)
}

/// The previous kind in the cycle (git <-> file/folder).
fn prev_kind(kind: RepoKind) -> RepoKind {
    toggle_kind(kind)
}

/// Toggles between the two kinds.
fn toggle_kind(kind: RepoKind) -> RepoKind {
    match kind {
        RepoKind::Git => RepoKind::Path,
        RepoKind::Path => RepoKind::Git,
    }
}

/// The display label for a kind.
fn kind_label(kind: RepoKind) -> &'static str {
    match kind {
        RepoKind::Git => "git",
        RepoKind::Path => "file/folder",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basename_takes_final_component() {
        assert_eq!(basename("/code/hop").as_deref(), Some("hop"));
        assert_eq!(basename("/code/hop/").as_deref(), Some("hop"));
        assert_eq!(basename("  ").as_deref(), None);
    }

    #[test]
    fn section_choice_matches_case_insensitively() {
        let options = section_options(&["Work".to_string()]);
        assert_eq!(section_choice(&options, Some("work")), 1);
        assert_eq!(section_choice(&options, Some("Misc")), 0);
        assert_eq!(section_choice(&options, None), 0);
    }

    #[test]
    fn both_kinds_show_section_and_only_git_shows_fav() {
        let git = RepoForm::for_add("/p", RepoKind::Git, &[], &[]);
        assert!(git.fields().contains(&Field::Fav));
        assert!(git.fields().contains(&Field::Section));

        let folder = RepoForm::for_add("/p", RepoKind::Path, &[], &[]);
        assert!(folder.fields().contains(&Field::Section));
        assert!(!folder.fields().contains(&Field::Fav));

        // The backup toggle shows for both kinds.
        assert!(git.fields().contains(&Field::Backup));
        assert!(folder.fields().contains(&Field::Backup));
    }

    #[test]
    fn switching_kind_shows_that_kinds_sections() {
        let git_sections = ["Backend".to_string()];
        let path_sections = ["Notes".to_string()];
        let mut form = RepoForm::for_add(
            "/p",
            RepoKind::Git,
            &git_sections,
            &path_sections,
        );
        assert_eq!(form.section_options().len(), 2); // Ungrouped + Backend
        assert_eq!(form.section_options()[1].as_deref(), Some("Backend"));
        // Switch to a path: the files namespace's sections show instead.
        form.edit_kind(KeyEvent::from(KeyCode::Right));
        assert_eq!(form.kind, RepoKind::Path);
        assert_eq!(form.section_options()[1].as_deref(), Some("Notes"));
    }

    #[test]
    fn for_add_seeds_backup_toggle_per_kind() {
        // Git repos default to included, file/folder entries to excluded.
        let git = RepoForm::for_add("/p", RepoKind::Git, &[], &[]);
        assert!(git.draft().include_in_backup);
        let folder = RepoForm::for_add("/p", RepoKind::Path, &[], &[]);
        assert!(!folder.draft().include_in_backup);
    }
}
