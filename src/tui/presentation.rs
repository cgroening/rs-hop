//! Glyphs (two tiers), status formatting and shared rendering helpers.
//!
//! Every marker is defined in two tiers - a plain Unicode symbol and an
//! ASCII-only fallback - and the configured [`GlyphVariant`] selects which
//! renders. All glyphs are single-cell (no Nerd Font, no emoji) so column
//! layout matches what the terminal draws.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use ratada::input::InputField;
use ratada::text::truncate;
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::Paragraph;

use crate::domain::repo::GitInfo;
use crate::theme::{GlyphVariant, Palette};
use crate::tui::skin::Colors;

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
}

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

/// A coloured status span for `info` (or a loading/missing marker for `None`),
/// shared by the table and the sectioned view: dim while loading, green when
/// clean, amber when there are uncommitted changes, default otherwise. The
/// per-row refresh spinner is handled by the caller, not here.
pub fn status_span(
    info: Option<&GitInfo>,
    icons: &IconSet,
    colors: &Colors,
) -> Span<'static> {
    let Some(info) = info else {
        return Span::styled("\u{2026}", Style::default().fg(colors.dim));
    };
    if info.is_path_missing() {
        return Span::styled("-", Style::default().fg(colors.dim));
    }
    let text = status_text(info, icons);
    let style = if info.is_clean() {
        Style::default().fg(colors.positive)
    } else if info.changes.unwrap_or(0) > 0 {
        Style::default().fg(colors.changes)
    } else {
        Style::default()
    };
    Span::styled(text, style)
}

/// The style for an inline slug shown after an entry name: dim and italic.
pub fn slug_style(colors: &Colors) -> Style {
    Style::default()
        .fg(colors.dim)
        .add_modifier(Modifier::ITALIC)
}

/// The plain name text used to size the name column: the name, plus ` slug`
/// when a slug is shown.
pub fn name_plain(name: &str, slug: Option<&str>) -> String {
    match slug {
        Some(slug) => format!("{name} {slug}"),
        None => name.to_string(),
    }
}

/// Spans for `name` with the characters matched by the fuzzy `query` shown in
/// accent + bold and the rest plain. Falls back to the plain name when the
/// query is empty or does not match.
pub fn highlight_name(
    name: &str,
    query: &str,
    colors: &Colors,
) -> Vec<Span<'static>> {
    let matched = match_indices(name, query);
    if matched.is_empty() {
        return vec![Span::raw(name.to_string())];
    }
    let hit = Style::default()
        .fg(colors.accent)
        .add_modifier(Modifier::BOLD);
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

/// Renders a dim, centred placeholder in the vertical middle of `area`.
pub fn render_empty_hint(
    frame: &mut Frame,
    area: Rect,
    text: &str,
    colors: &Colors,
) {
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
            .style(Style::default().fg(colors.dim))
            .alignment(Alignment::Center),
        row,
    );
}

/// One text field of a hop overlay, as it is about to be drawn.
pub struct FieldView<'a> {
    /// The field holding the text and its caret.
    pub field: &'a InputField,
    /// The active palette, which colours the caret and the scroll marker.
    pub palette: &'a Palette,
    /// The display columns the value may occupy, after any label.
    pub width: usize,
    /// Whether the field has the focus (only then is the caret drawn).
    pub focused: bool,
}

/// The spans of a single-line text field: the toolkit's block caret over a
/// horizontally scrolled value while focused, otherwise the plain value.
///
/// hop's overlays lay out their own lines, so they paint the field rather than
/// render `InputField` as a widget - but the caret, the scrolling and the `…`
/// marker still come from the toolkit rather than being drawn by hand.
pub fn field_spans(view: FieldView) -> Vec<Span<'static>> {
    if view.focused {
        return view.field.caret_spans(view.palette, view.width);
    }
    vec![Span::raw(truncate(view.field.value(), view.width))]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn unicode() -> IconSet {
        IconSet::new(GlyphVariant::Unicode)
    }

    /// The whole rendered text of a field, spans concatenated.
    fn rendered(field: &InputField, width: usize, focused: bool) -> String {
        let palette = Config::default().palette();
        field_spans(FieldView {
            field,
            palette: &palette,
            width,
            focused,
        })
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
    }

    #[test]
    fn a_focused_field_scrolls_under_the_caret_and_marks_the_clipped_head() {
        // The caret sits at the end, so the head scrolls out of view and is
        // marked. The hand-rolled caret this replaced simply overflowed.
        let field = InputField::new("/a/very/long/path/to/somewhere");
        let text = rendered(&field, 10, true);
        assert!(
            text.starts_with('\u{2026}'),
            "clipped head marked: {text:?}"
        );
        assert!(text.contains("where"), "the caret's end stays visible");
    }

    #[test]
    fn an_unfocused_field_is_truncated_and_carries_no_caret() {
        let field = InputField::new("/a/very/long/path");
        assert_eq!(rendered(&field, 10, false), "/a/very/l\u{2026}");
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
    fn name_plain_appends_slug_when_present() {
        assert_eq!(name_plain("hop", Some("hp")), "hop hp");
        assert_eq!(name_plain("hop", None), "hop");
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
        let colors =
            Colors::from_palette(&crate::config::Config::default().palette());
        // No query -> a single plain span.
        assert_eq!(highlight_name("hop", "", &colors).len(), 1);
        // A match produces more than one span (hit/non-hit runs).
        assert!(highlight_name("readme", "rm", &colors).len() > 1);
        // A non-match falls back to a single plain span.
        assert_eq!(highlight_name("abc", "xyz", &colors).len(), 1);
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
