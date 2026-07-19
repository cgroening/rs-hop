//! Turning the application state into a frame: the app-frame bands, the list
//! body, the status lines and the overlays.

use super::*;

use chrono::Local;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::domain::filter::{Tab, TabKind};
use crate::domain::repo::Repo;
use crate::tui::help::HelpView;
use crate::tui::presentation::{FieldView, field_spans, render_empty_hint};

/// The label in front of the live filter in the status band.
pub(super) const FILTER_LABEL: &str = "filter: ";
/// The hint after the live filter in the status band.
pub(super) const FILTER_HINT: &str = "   Enter open · Esc clear";
/// Cells the status band loses to `appframe`'s horizontal padding.
pub(super) const STATUS_PADDING: usize = 2;

/// The columns the live filter's value may occupy on a `width`-wide terminal:
/// the status band minus its padding, its label and its trailing hint. At least
/// one column, so the caret always has somewhere to sit.
pub(super) fn filter_value_width(width: u16) -> usize {
    (width as usize)
        .saturating_sub(STATUS_PADDING)
        .saturating_sub(FILTER_LABEL.chars().count())
        .saturating_sub(FILTER_HINT.chars().count())
        .max(1)
}

/// The placeholder text for an empty tab.
pub(super) fn empty_hint(tab: Tab) -> &'static str {
    match tab {
        Tab::GitActive => "No git repos. Press n to add one.",
        Tab::GitArchive => "No archived git repos.",
        Tab::FilesActive => "No folders or files. Press n to add one.",
        Tab::FilesArchive => "No archived folders or files.",
    }
}

/// A short relative age like `2d`, `5h` or `3m` for the remote line.
pub(super) fn relative_age(age: chrono::Duration) -> String {
    if age.num_days() >= 1 {
        return format!("{}d", age.num_days());
    }
    if age.num_hours() >= 1 {
        return format!("{}h", age.num_hours());
    }
    format!("{}m", age.num_minutes().max(0))
}

impl App {
    /// Renders the whole screen: the panel app-frame (tinted header/content/
    /// status bands plus backgroundless hints), the entry list (and preview) in
    /// the content surface, and any overlay on top.
    pub(super) fn render(&self, frame: &mut Frame) {
        let area = frame.area();
        let areas = crate::tui::appframe::render_frame(
            frame,
            &self.skin,
            crate::tui::appframe::TabBar {
                active: self.tab.kind_index(),
                archived: self.tab.is_archived(),
            },
            self.status_lines(area.width),
            &self.hint_groups(),
            self.loading.is_some(),
        );
        let (body, footer) = self.split_columns_footer(areas.content);
        let (list_area, preview_area) = self.split_preview(body);
        self.render_body(frame, list_area);
        if let Some(footer) = footer {
            crate::tui::columns::render_footer(
                frame,
                footer,
                (self.columns(), self.tab),
                (self.visible_totals(), &self.colors),
            );
        }
        if let Some(preview_area) = preview_area {
            self.render_preview(frame, preview_area);
        }
        if let Some(progress_area) = areas.progress {
            self.render_progress_bar(frame, progress_area);
        }
        // Snapshot the finished view so an overlay can dim it as its backdrop.
        crate::tui::appframe::snapshot_frame(frame);
        self.render_overlay(frame, area);
    }

    /// The status-band lines: the info line (or the progress line while a
    /// refresh/backup runs), plus the live-filter input or a transient status
    /// message when either is active. `width` is the terminal width, from which
    /// the columns left for the filter's value are measured.
    pub(super) fn status_lines(&self, width: u16) -> Vec<Line<'_>> {
        let mut lines = vec![self.info_line()];
        if self.filtering {
            let mut spans = vec![Span::styled(
                FILTER_LABEL,
                Style::default().fg(self.colors.accent),
            )];
            spans.extend(field_spans(FieldView {
                field: &self.filter,
                palette: &self.skin.palette,
                width: filter_value_width(width),
                focused: true,
            }));
            spans.push(Span::styled(
                FILTER_HINT,
                Style::default().fg(self.colors.dim),
            ));
            lines.push(Line::from(spans));
        } else if let Some((message, _)) = &self.status_msg {
            lines.push(Line::from(Span::styled(
                format!(" {message}"),
                Style::default().fg(self.colors.accent),
            )));
        }
        lines
    }

    /// Splits the content band into the body and, outside the standard column
    /// set, the totals-and-bar footer below it. A short terminal keeps every
    /// row for the list and gets no footer.
    pub(super) fn split_columns_footer(
        &self,
        content: Rect,
    ) -> (Rect, Option<Rect>) {
        let rows =
            crate::tui::columns::footer_rows(self.columns(), content.height);
        if rows == 0 {
            return (content, None);
        }
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(rows)])
            .split(content);
        (parts[0], Some(parts[1]))
    }

    /// The info line for the status band - error count, entry count, sort, the
    /// active lenses, local status and remote fetch time, each behind its icon.
    /// The refresh/backup progress is shown by the bar above the status band
    /// (see [`App::render_progress_bar`]), so this stays the normal info line.
    pub(super) fn info_line(&self) -> Line<'_> {
        let icons = self.icons;
        let muted = Style::default().fg(self.colors.muted);
        let sep = || Span::styled("   ", Style::default().fg(self.colors.dim));
        let mut spans = vec![Span::raw(" ")];

        let errors = self.error_count();
        if errors > 0 {
            spans.push(Span::styled(
                format!("{}{errors}", icons.missing),
                Style::default()
                    .fg(self.colors.danger)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(sep());
        }
        // Entry count, as shown/total when a filter narrows the list.
        let shown = self.ordered_view().len();
        let total = self.tab_indices().len();
        let count = if shown == total {
            format!("{} {shown}", icons.count)
        } else {
            format!("{} {shown}/{total}", icons.count)
        };
        spans.push(Span::styled(count, muted));
        spans.push(sep());
        spans.push(Span::styled(
            format!("{} {}", icons.sort, self.sort().label()),
            muted,
        ));
        // Active view lenses (filter / changes-only / slugs) in the accent.
        let mut lenses: Vec<&str> = Vec::new();
        if self.filtering_active() {
            lenses.push("filter");
        }
        if self.changes_only {
            lenses.push("changes");
        }
        if self.show_slugs {
            lenses.push("slugs");
        }
        if !lenses.is_empty() {
            spans.push(sep());
            spans.push(Span::styled(
                lenses.join(" · "),
                Style::default().fg(self.colors.accent),
            ));
        }

        // The status/remote times are git-specific, so skip them on the files
        // tabs.
        if self.config.example_mode {
            spans.push(sep());
            spans.push(Span::styled("example mode", muted));
        } else if self.tab.kind() == TabKind::Git {
            if let Some(at) = self.cache_generated_at {
                let age = Local::now().signed_duration_since(at);
                spans.push(sep());
                spans.push(Span::styled(
                    format!(
                        "{} {} ({} ago)",
                        icons.clock,
                        at.format("%Y-%m-%d %H:%M"),
                        relative_age(age),
                    ),
                    muted,
                ));
            }
            spans.push(sep());
            spans.push(self.remote_span(icons.remote, muted));
        }
        Line::from(spans)
    }

    /// The remote-fetch segment of the info line: amber when over a day old or
    /// never fetched, muted otherwise.
    pub(super) fn remote_span(
        &self,
        icon: &str,
        muted: Style,
    ) -> Span<'static> {
        match self.last_fetched {
            None => Span::styled(
                format!("{icon} never fetched"),
                Style::default().fg(self.colors.changes),
            ),
            Some(at) => {
                let age = Local::now().signed_duration_since(at);
                let stale = age.num_hours() >= 24;
                let suffix = if stale { "  (stale)" } else { "" };
                let text = format!(
                    "{icon} {} ({} ago){suffix}",
                    at.format("%Y-%m-%d %H:%M"),
                    relative_age(age),
                );
                let style = if stale {
                    Style::default().fg(self.colors.changes)
                } else {
                    muted
                };
                Span::styled(text, style)
            }
        }
    }

    /// Renders the entry table, or an empty hint.
    pub(super) fn render_body(&self, frame: &mut Frame, area: Rect) {
        // Remember the page size for page-wise navigation.
        self.list_height
            .set(area.height.saturating_sub(1).max(1) as usize);
        let view = self.ordered_view();
        if view.is_empty() {
            render_empty_hint(frame, area, empty_hint(self.tab), &self.colors);
            return;
        }
        if self.is_sectioned() {
            self.render_sections(frame, area, view.len());
            return;
        }
        let repos = self.service.repos();
        let visible: Vec<&Repo> = view.iter().map(|&i| &repos[i]).collect();
        let cursor = self.cursor.min(visible.len() - 1);
        // Rows still in flight show an animated spinner in the status column.
        let spinner =
            self.spinner_frame().map(|glyph| (&self.refreshing, glyph));
        // Which visible rows are part of the multi-selection.
        let selected: Vec<bool> =
            view.iter().map(|i| self.selected.contains(i)).collect();
        let query = self.filter.value();
        let table_view = crate::tui::table::TableView {
            tab: self.tab,
            config: &self.config,
            skin: &self.skin,
            colors: &self.colors,
            columns: self.columns(),
            code: &self.stats.code,
            git: &self.stats.git,
            computing: &self.computing,
            now: Local::now().timestamp(),
            icons: &self.icons,
            example_mode: self.config.example_mode,
            spinner,
            selected: &selected,
            has_selection: !self.selected.is_empty(),
            missing: &self.files_missing,
            show_slugs: self.show_slugs,
            // The flat view labels each row with its section when grouping is
            // off, so the section is still visible without header bars.
            show_section: !self.grouped(),
            query: self.filtering_active().then_some(query),
            zip_backups: &self.zip_backups,
            offset: &self.table_offset,
        };
        crate::tui::table::render_table(
            frame,
            area,
            &visible,
            cursor,
            &table_view,
        );
    }

    /// Renders the current tab as a sectioned list (`view_len` entries total).
    pub(super) fn render_sections(
        &self,
        frame: &mut Frame,
        area: Rect,
        view_len: usize,
    ) {
        let groups = self.section_groups();
        let cursor = self.cursor.min(view_len.saturating_sub(1));
        let spinner =
            self.spinner_frame().map(|glyph| (&self.refreshing, glyph));
        let view = crate::tui::sections_view::SectionedView {
            tab: self.tab,
            groups: &groups,
            repos: self.service.repos(),
            config: &self.config,
            icons: &self.icons,
            skin: &self.skin,
            colors: &self.colors,
            columns: self.columns(),
            code: &self.stats.code,
            git: &self.stats.git,
            computing: &self.computing,
            example_mode: self.config.example_mode,
            spinner,
            now: Local::now().timestamp(),
            selected: &self.selected,
            has_selection: !self.selected.is_empty(),
            missing: &self.files_missing,
            show_slugs: self.show_slugs,
            zip_backups: &self.zip_backups,
            offset: &self.list_offset,
        };
        crate::tui::sections_view::render(frame, area, cursor, &view);
    }

    /// Renders the active overlay, if any, over the dimmed live view (the
    /// snapshot taken at the end of [`App::render`]), clibase-style.
    pub(super) fn render_overlay(&self, frame: &mut Frame, area: Rect) {
        if matches!(self.overlay, Overlay::None) {
            return;
        }
        crate::tui::appframe::dim_backdrop(frame);
        let skin = &self.skin;
        match &self.overlay {
            Overlay::None => {}
            Overlay::Help => crate::tui::help::render(
                frame,
                area,
                skin,
                &HelpView {
                    sections: &self.help_sections(),
                    query: self.help_query.value(),
                },
                &self.help_scroll,
            ),
            Overlay::Confirm(modal, _) => modal.render(frame, area, skin),
            Overlay::Prompt(prompt, _) => prompt.render(frame, area, skin),
            Overlay::Form(form, _) => form.render(frame, area, skin),
            Overlay::SectionPicker(picker, _, _) => {
                picker.render(frame, area, skin)
            }
            Overlay::Picker(picker, _) => picker.render(frame, area, skin),
            Overlay::Errors(modal, _) => modal.render(frame, area, skin),
            Overlay::ErrorAction(modal, _) => modal.render(frame, area, skin),
            Overlay::SectionJump(modal, _) => modal.render(frame, area, skin),
            Overlay::Sort(modal, _) => modal.render(frame, area, skin),
            Overlay::Sections(modal) => modal.render(frame, area, skin),
            Overlay::SectionPrompt(prompt, _) => {
                prompt.render(frame, area, skin)
            }
            Overlay::SectionDelete(confirm, _) => {
                confirm.render(frame, area, skin)
            }
        }
    }
}
