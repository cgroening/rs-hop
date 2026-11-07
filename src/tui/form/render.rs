//! How the add/edit form draws itself, from the toolkit's form building
//! blocks: the modal frame and padding, the label column as wide as the
//! longest label, the `*` of a changed field in its own column, the focused
//! row tinted from border to border, the pointer glyph on the two picker
//! fields (Path, Section), the stepper on the kind, a problem in red under its
//! field, and two fixed hint rows - the focused field's keys above the form's
//! - so the box keeps its height while the focus moves.

use ratada::form::field::{self, FieldStyles, LabelMarks};
use ratada::modal::{hint_block, hints_width};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use super::{Field, MIXED, Mode, RepoForm, kind_label};
use crate::domain::sections::UNGROUPED;
use crate::theme::Skin;
use crate::tui::presentation::{FieldView, field_spans};
use crate::tui::widgets::box_rect;

/// The fewest columns a value gets, however short the opening values are.
const MIN_VALUE_WIDTH: usize = 36;
/// Rows of the hint block: a spacer, the field's keys, the form's keys.
const HINT_ROWS: u16 = 3;

/// The form's own keys in normal mode.
const NORMAL_HINTS: [(&str, &str); 4] = [
    ("tab", "next"),
    ("s", "save"),
    ("ctrl+o", "reset"),
    ("esc", "leave (asks to save)"),
];
/// The form's own keys while a field is typed into.
const EDITING_HINTS: [(&str, &str); 3] =
    [("tab", "next"), ("ctrl+s", "save"), ("ctrl+o", "reset")];
/// The keys of the "Save changes?" question.
const QUESTION_HINTS: [(&str, &str); 3] =
    [("y", "save"), ("n", "discard"), ("b", "back")];
/// The keys of the focused field, by kind and mode.
const TEXT_HINTS: [(&str, &str); 1] = [("enter", "edit")];
const TEXT_EDITING_HINTS: [(&str, &str); 1] = [("enter/esc", "done")];
const PATH_HINTS: [(&str, &str); 2] = [("enter", "pick"), ("e", "type")];
const SECTION_HINTS: [(&str, &str); 1] = [("enter", "pick")];
const KIND_HINTS: [(&str, &str); 1] = [("\u{2190}\u{2192}", "change")];
const TOGGLE_HINTS: [(&str, &str); 1] = [("space", "toggle")];

/// Every hint row the form can show, for its minimum width.
const ALL_HINTS: [&[(&str, &str)]; 9] = [
    &NORMAL_HINTS,
    &EDITING_HINTS,
    &QUESTION_HINTS,
    &TEXT_HINTS,
    &TEXT_EDITING_HINTS,
    &PATH_HINTS,
    &SECTION_HINTS,
    &KIND_HINTS,
    &TOGGLE_HINTS,
];

/// What every row of one form shares.
struct RowCtx<'a> {
    skin: &'a Skin,
    styles: FieldStyles,
    label_width: usize,
    width: usize,
}

impl super::RepoForm {
    /// Renders the form centred in `area`, as big as its fields and keys.
    pub fn render(&self, frame: &mut Frame, area: Rect, skin: &Skin) {
        let fields = self.fields();
        let rows =
            fields.len() as u16 + u16::from(self.error.is_some()) + HINT_ROWS;
        let rect = box_rect(area, (self.content_width(), rows));
        frame.render_widget(Clear, rect);
        let inner = ratada::overlay::framed(frame, rect, skin, &self.title);
        let ctx = RowCtx {
            skin,
            styles: FieldStyles::from_palette(&skin.palette),
            label_width: label_width(&fields),
            width: usize::from(inner.width),
        };
        let mut lines: Vec<Line> = Vec::new();
        let mut focus_row = 0;
        for (index, field) in fields.iter().enumerate() {
            if index == self.focus {
                focus_row = lines.len();
            }
            lines.push(self.field_line(*field, index, &ctx));
            if let Some((faulty, message)) = &self.error
                && faulty == field
            {
                lines.push(field::problem_line(
                    message,
                    field::value_column(ctx.label_width),
                    ctx.width,
                    &ctx.styles,
                ));
            }
        }
        lines.extend(self.hint_lines(skin, ctx.width));

        // On a short terminal the box is capped and the lower rows fall off:
        // scroll to keep the focused field visible, with the shared scrollbar.
        let total = lines.len();
        let viewport = usize::from(inner.height);
        let offset = ratada::nav::keep_visible(
            ratada::nav::ScrollView {
                total,
                offset: 0,
                viewport,
            },
            focus_row,
        );
        if let Some(row) = focus_row.checked_sub(offset) {
            tint_row(frame, inner, row, &ctx);
        }
        frame.render_widget(
            Paragraph::new(lines)
                .scroll((u16::try_from(offset).unwrap_or(0), 0)),
            inner,
        );
        ratada::scroll::render_scrollbar(
            frame,
            inner,
            skin,
            ratada::nav::ScrollView {
                total,
                offset,
                viewport,
            },
        );
    }

    /// The content width: the label column and the widest opening value (at
    /// least [`MIN_VALUE_WIDTH`]), never narrower than a hint row.
    fn content_width(&self) -> usize {
        let fields = self.fields();
        let values = self
            .baseline
            .path
            .width()
            .max(self.baseline.name.width())
            .max(MIN_VALUE_WIDTH);
        let content = field::value_column(label_width(&fields)) + values;
        let hints = ALL_HINTS.iter().map(|row| hints_width(row)).max();
        content.max(hints.unwrap_or(0))
    }

    /// The hint block: a spacer, the focused field's keys (or a notice, or
    /// the "Save changes?" question), the form's keys.
    fn hint_lines(&self, skin: &Skin, width: usize) -> Vec<Line<'static>> {
        let palette = &skin.palette;
        let accent = ratada::style::accent(palette);
        let (field_row, form_hints): (Line, &[(&str, &str)]) =
            if self.asking_save {
                (Line::styled("Save changes?", accent), &QUESTION_HINTS)
            } else {
                let row = match &self.notice {
                    Some(notice) => Line::styled(notice.clone(), accent),
                    None => hint_row(self.field_hints(), skin, width),
                };
                let form: &[(&str, &str)] = match self.mode {
                    Mode::Normal => &NORMAL_HINTS,
                    Mode::Editing => &EDITING_HINTS,
                };
                (row, form)
            };
        vec![Line::raw(""), field_row, hint_row(form_hints, skin, width)]
    }

    /// The keys of the focused field in the current mode.
    fn field_hints(&self) -> &'static [(&'static str, &'static str)] {
        let editing = self.mode == Mode::Editing;
        match self.focused_field() {
            Field::Name | Field::Slug | Field::Path if editing => {
                &TEXT_EDITING_HINTS
            }
            Field::Name | Field::Slug => &TEXT_HINTS,
            Field::Path => &PATH_HINTS,
            Field::Section => &SECTION_HINTS,
            Field::Kind => &KIND_HINTS,
            Field::Fav | Field::Backup => &TOGGLE_HINTS,
        }
    }

    /// One field row: the change marker, the label (red with a problem) and
    /// the value.
    fn field_line(
        &self,
        field: Field,
        index: usize,
        ctx: &RowCtx,
    ) -> Line<'static> {
        let focused = index == self.focus;
        let marks = LabelMarks {
            changed: self.is_changed(field),
            focused,
            problem: self.error.as_ref().is_some_and(|(f, _)| *f == field),
        };
        let mut spans = field::label_cell(
            field_name(field),
            ctx.label_width,
            marks,
            &ctx.styles,
        );
        let value_width = ctx
            .width
            .saturating_sub(field::value_column(ctx.label_width));
        spans.extend(self.value_spans(field, focused, value_width, ctx));
        Line::from(spans)
    }

    /// The value part of a field row, `width` columns wide.
    fn value_spans(
        &self,
        field: Field,
        focused: bool,
        width: usize,
        ctx: &RowCtx,
    ) -> Vec<Span<'static>> {
        let skin = ctx.skin;
        let typing = focused && self.mode == Mode::Editing;
        match field {
            Field::Path if typing => self.typed(&self.path, width, skin),
            Field::Path => field::picker_value_spans(
                self.path.value(),
                width,
                skin.glyphs.pointer,
                &ctx.styles,
            ),
            Field::Name if typing => self.typed(&self.name, width, skin),
            Field::Slug if typing => self.typed(&self.slug, width, skin),
            Field::Name => text_value(self.name.value(), width, &ctx.styles),
            Field::Slug => text_value(self.slug.value(), width, &ctx.styles),
            Field::Section => field::picker_value_spans(
                &self.section_label(),
                width,
                skin.glyphs.pointer,
                &ctx.styles,
            ),
            Field::Kind => {
                vec![Span::raw(skin.glyphs.stepper(kind_label(self.kind)))]
            }
            Field::Fav => {
                let mark = checkbox(self.fav, self.fav_mixed, self.fav_touched);
                vec![Span::raw(mark.to_string())]
            }
            Field::Backup => {
                let mark = checkbox(
                    self.include_in_backup,
                    self.backup_mixed,
                    self.backup_touched,
                );
                vec![Span::raw(mark.to_string())]
            }
        }
    }

    /// A text field being typed into: the toolkit's caret over a value that
    /// scrolls to fit `width`.
    fn typed(
        &self,
        input: &ratada::input::InputField,
        width: usize,
        skin: &Skin,
    ) -> Vec<Span<'static>> {
        field_spans(FieldView {
            field: input,
            palette: &skin.palette,
            width,
            focused: true,
        })
    }

    /// The label shown for the Section field: the chosen section, Ungrouped, or
    /// the mixed placeholder in an untouched bulk form.
    pub(super) fn section_label(&self) -> String {
        if self.bulk && self.section_mixed && !self.section_touched {
            return MIXED.to_string();
        }
        self.section
            .clone()
            .unwrap_or_else(|| UNGROUPED.to_string())
    }
}

/// A plain text value, or the dimmed empty marker.
fn text_value(
    value: &str,
    width: usize,
    styles: &FieldStyles,
) -> Vec<Span<'static>> {
    if value.is_empty() {
        return vec![Span::styled(field::EMPTY_VALUE, styles.muted)];
    }
    vec![Span::raw(ratada::text::truncate(value, width))]
}

/// One hint row drawn like the toolkit's modal keys.
fn hint_row(
    hints: &[(&str, &str)],
    skin: &Skin,
    width: usize,
) -> Line<'static> {
    hint_block(hints, &skin.palette, width)
        .into_iter()
        .nth(1)
        .unwrap_or_default()
}

/// The label of `field`.
fn field_name(field: Field) -> &'static str {
    match field {
        Field::Path => "Path",
        Field::Name => "Name",
        Field::Slug => "Slug",
        Field::Section => "Section",
        Field::Kind => "Kind",
        Field::Fav => "Fav",
        Field::Backup => "Backup",
    }
}

/// The width of the label column over `fields`.
fn label_width(fields: &[Field]) -> usize {
    field::label_column_width(fields.iter().map(|&f| field_name(f)))
}

/// Tints the focused row (`row` lines into `inner`) from border to border,
/// through the modal's side padding.
fn tint_row(frame: &mut Frame, inner: Rect, row: usize, ctx: &RowCtx) {
    let y = inner.y + u16::try_from(row).unwrap_or(u16::MAX);
    if y >= inner.bottom() {
        return;
    }
    let pad = ratada::chrome::MODAL_PADDING_X;
    let rect =
        Rect::new(inner.x.saturating_sub(pad), y, inner.width + 2 * pad, 1)
            .intersection(frame.area());
    frame.render_widget(Block::default().style(ctx.styles.focus_tint), rect);
}

/// The checkbox glyph: mixed (`[-]`) while an untouched bulk field differs, else
/// on (`[x]`) or off (`[ ]`).
pub(super) fn checkbox(on: bool, mixed: bool, touched: bool) -> &'static str {
    if mixed && !touched {
        "[-]"
    } else if on {
        "[x]"
    } else {
        "[ ]"
    }
}

impl RepoForm {
    /// Whether the form currently asks "Save changes?" (for tests).
    #[cfg(test)]
    pub(super) fn is_asking(&self) -> bool {
        self.asking_save
    }
}
