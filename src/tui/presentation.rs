//! Glyphs (two tiers), status formatting and shared rendering helpers.
//!
//! Every marker is defined in two tiers - a plain Unicode symbol and an
//! ASCII-only fallback - and the configured [`GlyphVariant`] selects which
//! renders. All glyphs are single-cell (no Nerd Font, no emoji) so column
//! layout matches what the terminal draws.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};
use unicode_width::UnicodeWidthStr;

use crate::domain::repo::GitInfo;
use crate::theme::GlyphVariant;
use crate::tui::colors::{ACCENT, DIM};

/// The resolved glyph set for the active icon variant. Every glyph is a
/// single-cell symbol (no Nerd Font icons, no colourful emoji), so column
/// layout matches what the terminal draws.
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
    /// Entry-count marker in the info line.
    pub count: &'static str,
    /// Sort-mode marker in the info line.
    pub sort: &'static str,
    /// Local-status (last gathered) marker in the info line.
    pub clock: &'static str,
    /// Remote (last fetched) marker in the info line.
    pub remote: &'static str,
    /// Marker shown when an entry is excluded from the "backup all" (`Z`) run.
    pub excluded: &'static str,
    /// Animation frames for the "refreshing" spinner (single-cell each).
    pub spinner: &'static [&'static str],
}

/// Braille spinner frames (single-cell, widely supported).
const UNICODE_SPINNER: &[&str] = &[
    "\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}",
    "\u{2826}", "\u{2827}", "\u{2807}", "\u{280f}",
];

/// ASCII spinner frames.
const ASCII_SPINNER: &[&str] = &["|", "/", "-", "\\"];

impl IconSet {
    /// Builds the glyph set for `variant`.
    pub fn new(variant: GlyphVariant) -> Self {
        match variant {
            GlyphVariant::Unicode => IconSet {
                missing: "\u{2717}",   // ✗
                favourite: "\u{2605}", // ★
                clean: "\u{2713}",     // ✓
                changes: "\u{2260}",   // ≠
                ahead: "\u{2191}",     // ↑
                behind: "\u{2193}",    // ↓
                count: "\u{2261}",     // ≡
                sort: "\u{2195}",      // ↕
                clock: "\u{21bb}",     // ↻
                remote: "\u{21a7}",    // ↧
                excluded: "\u{2298}",  // ⊘
                spinner: UNICODE_SPINNER,
            },
            GlyphVariant::Ascii => IconSet {
                missing: "!",
                favourite: "*",
                clean: "ok",
                changes: "~",
                ahead: "^",
                behind: "v",
                count: "#",
                sort: "~",
                clock: "@",
                remote: "v",
                excluded: "x",
                spinner: ASCII_SPINNER,
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

/// The style for an inline slug shown after an entry name: dim and italic.
pub fn slug_style() -> Style {
    Style::default().fg(DIM).add_modifier(Modifier::ITALIC)
}

/// The plain name text used to size the name column: the name, plus ` slug`
/// when a slug is shown.
pub fn name_plain(name: &str, slug: Option<&str>) -> String {
    match slug {
        Some(slug) => format!("{name} {slug}"),
        None => name.to_string(),
    }
}

/// The spans for an entry name: the name, plus a dim-italic ` slug` when shown.
/// Used by the git table, where ratatui clips to the column width.
pub fn name_spans(name: &str, slug: Option<&str>) -> Vec<Span<'static>> {
    let mut spans = vec![Span::raw(name.to_string())];
    if let Some(slug) = slug {
        spans.push(Span::styled(format!(" {slug}"), slug_style()));
    }
    spans
}

/// Spans for `name` with the characters matched by the fuzzy `query` shown in
/// accent + bold and the rest plain. Falls back to the plain name when the
/// query is empty or does not match.
pub fn highlight_name(name: &str, query: &str) -> Vec<Span<'static>> {
    let matched = match_indices(name, query);
    if matched.is_empty() {
        return vec![Span::raw(name.to_string())];
    }
    let hit = Style::default().fg(ACCENT).add_modifier(Modifier::BOLD);
    let mut spans: Vec<Span> = Vec::new();
    let mut current = String::new();
    let mut current_hit = false;
    for (index, ch) in name.chars().enumerate() {
        let is_hit = matched.contains(&(index as u32));
        if is_hit != current_hit && !current.is_empty() {
            spans.push(styled_run(
                std::mem::take(&mut current),
                current_hit,
                hit,
            ));
        }
        current_hit = is_hit;
        current.push(ch);
    }
    if !current.is_empty() {
        spans.push(styled_run(current, current_hit, hit));
    }
    spans
}

/// A run of characters styled as a match hit or plain text.
fn styled_run(text: String, hit: bool, hit_style: Style) -> Span<'static> {
    if hit {
        Span::styled(text, hit_style)
    } else {
        Span::raw(text)
    }
}

/// The char positions of `name` matched by the fuzzy `query` (empty when the
/// query is blank or there is no match).
fn match_indices(name: &str, query: &str) -> Vec<u32> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern =
        Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    let mut buf = Vec::new();
    let haystack = Utf32Str::new(name, &mut buf);
    let mut indices = Vec::new();
    pattern.indices(haystack, &mut matcher, &mut indices);
    indices.sort_unstable();
    indices.dedup();
    indices
}

/// Builds a browseable GitHub URL from a stored `github_repo_name`: an
/// `owner/repo` value maps directly; a bare `repo` (the owner was stripped as
/// the configured `username`) needs that `username` to rebuild the owner.
/// Returns `None` when no URL can be formed (assumes github.com).
pub fn github_url(name: &str, username: Option<&str>) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    if name.contains('/') {
        return Some(format!("https://github.com/{name}"));
    }
    let owner = username?.trim();
    if owner.is_empty() {
        return None;
    }
    Some(format!("https://github.com/{owner}/{name}"))
}

/// The separator placed between footer tokens (never at a line end).
const FOOTER_SEPARATOR: &str = " · ";

/// Wraps footer hints into lines, keeping each `(keys, description)` token
/// whole. Keys are in the soft accent colour, descriptions dim (like mdtask).
pub fn footer_lines(items: &[(&str, &str)], width: u16) -> Vec<Line<'static>> {
    let budget = (width.max(1) as usize).saturating_sub(1);
    let separator_width = UnicodeWidthStr::width(FOOTER_SEPARATOR);
    let key_style = Style::default().fg(ACCENT);
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
        IconSet::new(GlyphVariant::Unicode)
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

    #[test]
    fn name_plain_appends_slug_when_present() {
        assert_eq!(name_plain("hop", Some("hp")), "hop hp");
        assert_eq!(name_plain("hop", None), "hop");
    }

    #[test]
    fn name_spans_adds_slug_only_when_present() {
        assert_eq!(name_spans("hop", None).len(), 1);
        assert_eq!(name_spans("hop", Some("hp")).len(), 2);
    }

    #[test]
    fn status_text_formats_counts_and_clean() {
        let icons = IconSet::new(GlyphVariant::Ascii);
        // Invalid -> dash.
        let invalid = GitInfo::default();
        assert_eq!(status_text(&invalid, &icons), "-");
        // A clean valid tree -> the clean marker.
        let clean = GitInfo {
            valid: true,
            ..GitInfo::default()
        };
        assert_eq!(status_text(&clean, &icons), "ok");
        // Changes and ahead are formatted with their icons (changes, then
        // behind, then ahead).
        let dirty = GitInfo {
            valid: true,
            changes: Some(2),
            ahead: Some(1),
            ..GitInfo::default()
        };
        assert_eq!(status_text(&dirty, &icons), "~2 ^1");
        // A verbatim raw status wins.
        let raw = GitInfo {
            valid: true,
            raw_status: Some("custom".to_string()),
            ..GitInfo::default()
        };
        assert_eq!(status_text(&raw, &icons), "custom");
    }

    #[test]
    fn highlight_name_marks_matched_chars() {
        // No query -> a single plain span.
        assert_eq!(highlight_name("hop", "").len(), 1);
        // A match produces more than one span (hit/non-hit runs).
        assert!(highlight_name("readme", "rm").len() > 1);
        // A non-match falls back to a single plain span.
        assert_eq!(highlight_name("abc", "xyz").len(), 1);
    }

    #[test]
    fn github_url_handles_owner_repo_and_bare_repo() {
        assert_eq!(
            github_url("owner/repo", None).as_deref(),
            Some("https://github.com/owner/repo")
        );
        assert_eq!(
            github_url("hop", Some("cgroening")).as_deref(),
            Some("https://github.com/cgroening/hop")
        );
        // A bare repo without a username cannot be resolved.
        assert_eq!(github_url("hop", None), None);
        assert_eq!(github_url("", Some("x")), None);
    }
}
