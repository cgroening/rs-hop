//! Glyphs (three tiers), status formatting and shared rendering helpers.
//!
//! Every marker is defined in three tiers - a Nerd Font icon, a plain Unicode
//! symbol and an ASCII fallback - and the configured [`IconVariant`] selects
//! which renders, so a polished look stays possible without excluding terminals
//! that lack the font.

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};
use unicode_width::UnicodeWidthStr;

use crate::config::IconVariant;
use crate::domain::repo::GitInfo;
use crate::tui::colors::{ACCENT, DIM};

/// The resolved glyph set for the active icon variant.
#[derive(Debug, Clone, Copy)]
pub struct IconSet {
    /// Marker shown when a path no longer exists.
    pub missing: &'static str,
    /// Favourite marker.
    pub favourite: &'static str,
    /// Clean-repository marker.
    pub clean: &'static str,
    /// Uncommitted-changes marker (prefixes the count).
    pub changes: &'static str,
    /// Ahead-of-upstream marker (prefixes the count).
    pub ahead: &'static str,
    /// Behind-upstream marker (prefixes the count).
    pub behind: &'static str,
    /// Folder marker (Files and Folders tab).
    pub folder: &'static str,
    /// File marker (Files and Folders tab).
    pub file: &'static str,
}

impl IconSet {
    /// Builds the glyph set for `variant`.
    pub fn new(variant: IconVariant) -> Self {
        match variant {
            IconVariant::NerdFont => IconSet {
                missing: "\u{f071}",   // nf-fa-warning
                favourite: "\u{f005}", // nf-fa-star
                clean: "\u{f00c}",     // nf-fa-check
                changes: "\u{f448}",   // nf-oct-diff
                ahead: "\u{f062}",     // nf-fa-arrow_up
                behind: "\u{f063}",    // nf-fa-arrow_down
                folder: "\u{f07b}",    // nf-fa-folder
                file: "\u{f15b}",      // nf-fa-file
            },
            IconVariant::Unicode => IconSet {
                missing: "!",
                favourite: "\u{2605}", // ★
                clean: "\u{2713}",     // ✓
                changes: "\u{2260}",   // ≠
                ahead: "\u{2191}",     // ↑
                behind: "\u{2193}",    // ↓
                folder: "\u{1f4c1}",   // 📁
                file: "\u{1f4c4}",     // 📄
            },
            IconVariant::Ascii => IconSet {
                missing: "!",
                favourite: "*",
                clean: "ok",
                changes: "~",
                ahead: "^",
                behind: "v",
                folder: "d",
                file: "f",
            },
        }
    }
}

/// The status string for `info` using `icons`. A verbatim `raw_status` (example
/// mode) wins; otherwise the structured counts are formatted, collapsing to the
/// clean marker when there is nothing to report.
pub fn status_text(info: &GitInfo, icons: &IconSet) -> String {
    if !info.valid {
        return "-".to_string();
    }
    if let Some(raw) = &info.raw_status {
        return raw.clone();
    }
    let mut parts: Vec<String> = Vec::new();
    if let Some(changes) = info.changes
        && changes > 0
    {
        parts.push(format!("{}{changes}", icons.changes));
    }
    if let Some(behind) = info.behind
        && behind > 0
    {
        parts.push(format!("{}{behind}", icons.behind));
    }
    if let Some(ahead) = info.ahead
        && ahead > 0
    {
        parts.push(format!("{}{ahead}", icons.ahead));
    }
    if parts.is_empty() {
        return icons.clean.to_string();
    }
    parts.join(" ")
}

/// Truncates `text` to `width` display columns, adding `…` when cut.
pub fn truncate(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut result = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = UnicodeWidthStr::width(ch.to_string().as_str());
        if used + w > width.saturating_sub(1) {
            break;
        }
        result.push(ch);
        used += w;
    }
    result.push('…');
    result
}

/// Maximum footer rows, so a long hint never crowds out the list.
pub const FOOTER_MAX_ROWS: usize = 3;

/// The separator placed between footer tokens (never at a line end).
const FOOTER_SEPARATOR: &str = " · ";

/// Wraps footer hints into lines, keeping each `(keys, description)` token
/// whole. Keys are accent-coloured and bold, descriptions dim.
pub fn footer_lines(items: &[(&str, &str)], width: u16) -> Vec<Line<'static>> {
    let budget = (width.max(1) as usize).saturating_sub(1);
    let separator_width = UnicodeWidthStr::width(FOOTER_SEPARATOR);
    let key_style = Style::default().fg(ACCENT).add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(DIM);

    let mut lines: Vec<Line> = Vec::new();
    let mut current: Vec<Span> = Vec::new();
    let mut used = 0usize;

    for (keys, description) in items {
        let token_width = UnicodeWidthStr::width(*keys)
            + 1
            + UnicodeWidthStr::width(*description);
        let needed = if current.is_empty() {
            token_width
        } else {
            separator_width + token_width
        };
        if !current.is_empty() && used + needed > budget {
            lines.push(finish_footer_line(std::mem::take(&mut current)));
            used = 0;
        }
        if !current.is_empty() {
            current.push(Span::styled(FOOTER_SEPARATOR, dim));
            used += separator_width;
        }
        current.push(Span::styled(keys.to_string(), key_style));
        current.push(Span::styled(format!(" {description}"), dim));
        used += token_width;
    }
    if !current.is_empty() {
        lines.push(finish_footer_line(current));
    }
    lines.truncate(FOOTER_MAX_ROWS);
    lines
}

/// Prepends the left-pad space and builds a footer line.
fn finish_footer_line(mut spans: Vec<Span<'static>>) -> Line<'static> {
    spans.insert(0, Span::raw(" "));
    Line::from(spans)
}

/// Renders a dim vertical scrollbar reflecting `position` within a `total`-row
/// list shown through a `viewport`-row window.
pub fn render_scrollbar(
    frame: &mut Frame,
    area: Rect,
    total: usize,
    position: usize,
    viewport: usize,
) {
    if total <= viewport {
        return;
    }
    let content_length = total.saturating_sub(viewport).saturating_add(1);
    let mut state = ScrollbarState::new(content_length)
        .position(position)
        .viewport_content_length(viewport);
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(None)
        .end_symbol(None)
        .style(Style::default().fg(DIM));
    frame.render_stateful_widget(scrollbar, area, &mut state);
}

/// Renders a dim, centred placeholder in the vertical middle of `area`.
pub fn render_empty_hint(frame: &mut Frame, area: Rect, text: &str) {
    if area.height < 1 || area.width < 4 {
        return;
    }
    let row = Rect {
        x: area.x + 1,
        y: area.y + area.height / 2,
        width: area.width.saturating_sub(2),
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(text.to_string())
            .style(Style::default().fg(DIM))
            .alignment(Alignment::Center),
        row,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unicode() -> IconSet {
        IconSet::new(IconVariant::Unicode)
    }

    #[test]
    fn clean_status_when_no_changes() {
        let info = GitInfo {
            valid: true,
            changes: Some(0),
            ahead: Some(0),
            behind: Some(0),
            ..GitInfo::default()
        };
        assert_eq!(status_text(&info, &unicode()), "\u{2713}");
    }

    #[test]
    fn structured_status_lists_changes_behind_ahead() {
        let info = GitInfo {
            valid: true,
            changes: Some(5),
            behind: Some(2),
            ahead: Some(1),
            ..GitInfo::default()
        };
        assert_eq!(
            status_text(&info, &unicode()),
            "\u{2260}5 \u{2193}2 \u{2191}1"
        );
    }

    #[test]
    fn raw_status_overrides_counts() {
        let info = GitInfo {
            valid: true,
            raw_status: Some("custom".to_string()),
            changes: Some(9),
            ..GitInfo::default()
        };
        assert_eq!(status_text(&info, &unicode()), "custom");
    }

    #[test]
    fn invalid_status_is_a_dash() {
        let info = GitInfo {
            valid: false,
            ..GitInfo::default()
        };
        assert_eq!(status_text(&info, &unicode()), "-");
    }

    #[test]
    fn truncate_adds_ellipsis_when_too_long() {
        assert_eq!(truncate("hello", 3), "he…");
        assert_eq!(truncate("hi", 5), "hi");
    }
}
