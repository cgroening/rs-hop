//! How the add/edit form draws itself: the field lines, their labels and
//! the checkbox glyphs.
use ratada::input::InputField;

use super::{Field, LABEL_WIDTH, MIXED, kind_label};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::domain::sections::UNGROUPED;
use crate::theme::Skin;
use crate::tui::presentation::{FieldView, field_spans};
use crate::tui::skin::Colors;
use crate::tui::widgets::centered_rect;
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
impl super::RepoForm {
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
            self.hint(),
            Style::default().fg(colors.dim),
        )));
        // `centered_rect` caps the box against the terminal, so on a short one
        // the lower fields fall off. Scroll to keep the focused field visible
        // and show the shared scrollbar instead of clipping silently.
        let inner = block.inner(rect);
        let total = lines.len();
        let viewport = inner.height as usize;
        let offset = ratada::nav::keep_visible(
            ratada::nav::ScrollView {
                total,
                offset: 0,
                viewport,
            },
            self.focus,
        );
        frame.render_widget(
            Paragraph::new(lines)
                .block(block)
                .scroll((u16::try_from(offset).unwrap_or(0), 0)),
            rect,
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

    /// The footer hint line for this form's mode.
    fn hint(&self) -> &'static str {
        if self.bulk {
            "Tab field · \u{2190}\u{2192} kind · Space toggle · Enter on \
             Section: pick · ^S save · Esc cancel"
        } else {
            "Tab field · \u{2190}\u{2192} kind · Space toggle · Enter on \
             Section: pick · ^O path · ^S save · Esc cancel"
        }
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
                // No `< >` chrome: the section is chosen via the picker
                // (Enter), not cycled like the kind.
                let label = self.section_label();
                self.value_line("Section", &label, index, colors)
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

    /// A `< value >` selector line (the kind, cycled with Left/Right).
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

    /// A labelled value line without the `< >` selector chrome (the section,
    /// which is chosen via the fuzzy picker on Enter, not cycled).
    fn value_line(
        &self,
        label: &str,
        value: &str,
        index: usize,
        colors: &Colors,
    ) -> Line<'static> {
        let spans = vec![
            self.label_span(label, index, colors),
            Span::styled(value.to_string(), Style::default().fg(colors.accent)),
        ];
        self.styled(spans, index, colors)
    }

    /// The favourite toggle line.
    fn fav_line(&self, index: usize, colors: &Colors) -> Line<'static> {
        let mark = checkbox(self.fav, self.fav_mixed, self.fav_touched);
        let spans = vec![
            self.label_span("Fav", index, colors),
            Span::raw(mark.to_string()),
        ];
        self.styled(spans, index, colors)
    }

    /// The "include in backup all (`Z`)" toggle line.
    fn backup_line(&self, index: usize, colors: &Colors) -> Line<'static> {
        let mark = checkbox(
            self.include_in_backup,
            self.backup_mixed,
            self.backup_touched,
        );
        let spans = vec![
            self.label_span("Backup", index, colors),
            Span::raw(mark.to_string()),
        ];
        self.styled(spans, index, colors)
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
