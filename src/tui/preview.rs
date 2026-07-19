//! The optional detail panel for the cursor entry.
//!
//! Shows the entry's in-memory fields, its statistics and a cached `git log`.
//! Rendering itself does no I/O: the app hands in what it already knows.
//!
//! The panel does **not** clear its area. `appframe` has already filled the
//! content band with the surface tint, and a bare `Clear` would punch a hole in
//! it down to the terminal's default background - which is what used to make
//! the panel look like it had a background of its own.

use ratada::text::truncate;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Padding, Paragraph};

use crate::domain::repo::{Repo, RepoKind, is_dir_target};
use crate::domain::stats::{
    CodeEntry, GitStats, format_age, format_bytes, format_count,
    language_shares,
};
use crate::theme::Skin;
use crate::tui::detail::{bar, field, log_line, section_header};
use crate::tui::git_columns::effective_info;
use crate::tui::presentation::{IconSet, status_text};
use crate::tui::scroll::Scroll;
use crate::tui::skin::Colors;

/// Width of the label column inside the panel.
const LABEL_WIDTH: usize = 12;
/// How many languages the CODE section lists.
const LANGUAGES_SHOWN: usize = 4;
/// Cells the language bar occupies.
const BAR_WIDTH: usize = 16;
/// Smallest inner width at which the panel still renders its rows.
const MIN_INNER_WIDTH: u16 = 12;

/// The narrowest the panel may be made, in percent of the body.
const MIN_WIDTH_PCT: u16 = 20;
/// The widest the panel may be made, in percent of the body.
const MAX_WIDTH_PCT: u16 = 70;
/// The shortest the panel may be made, in rows.
const MIN_HEIGHT_ROWS: u16 = 5;
/// The tallest the panel may be made, in rows.
const MAX_HEIGHT_ROWS: u16 = 30;
/// How much one resize step moves the width.
const WIDTH_STEP: i16 = 5;
/// How much one resize step moves the height.
const HEIGHT_STEP: i16 = 1;

/// Which side of the list the panel sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewPosition {
    /// To the right of the list.
    Right,
    /// Below the list.
    Bottom,
}

impl PreviewPosition {
    /// The other side.
    pub fn flip(self) -> Self {
        match self {
            PreviewPosition::Right => PreviewPosition::Bottom,
            PreviewPosition::Bottom => PreviewPosition::Right,
        }
    }
}

/// Where the detail panel sits and how big it is (persisted).
///
/// The two sizes are separate, each naming its unit: one number meaning percent
/// on one side and rows on the other could not be read correctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewLayout {
    /// Whether the panel is shown at all (`v`).
    pub visible: bool,
    /// Which side it sits on (`V`).
    pub position: PreviewPosition,
    /// Its width as a percentage of the body, when it sits to the right.
    pub width_pct: u16,
    /// Its height in rows, when it sits below.
    pub height_rows: u16,
}

impl Default for PreviewLayout {
    fn default() -> Self {
        PreviewLayout {
            visible: false,
            position: PreviewPosition::Right,
            width_pct: 40,
            height_rows: 9,
        }
    }
}

impl PreviewLayout {
    /// Rebuilds the layout from the persisted key and sizes. An unknown key -
    /// including the `"off"` an older hop wrote - hides the panel.
    pub fn from_state(key: &str, width_pct: u16, height_rows: u16) -> Self {
        let (visible, position) = match key.trim().to_lowercase().as_str() {
            "right" => (true, PreviewPosition::Right),
            "bottom" => (true, PreviewPosition::Bottom),
            _ => (false, PreviewPosition::Right),
        };
        PreviewLayout {
            visible,
            position,
            width_pct: width_pct.clamp(MIN_WIDTH_PCT, MAX_WIDTH_PCT),
            height_rows: height_rows.clamp(MIN_HEIGHT_ROWS, MAX_HEIGHT_ROWS),
        }
    }

    /// The stable persisted key: the position when visible, else `"off"`.
    pub fn as_key(self) -> &'static str {
        if !self.visible {
            return "off";
        }
        match self.position {
            PreviewPosition::Right => "right",
            PreviewPosition::Bottom => "bottom",
        }
    }

    /// Shows or hides the panel (`v`).
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    /// Moves the panel to the other side (`V`). A hidden panel is left alone,
    /// so the key cannot open it by surprise.
    pub fn flip_position(&mut self) {
        if self.visible {
            self.position = self.position.flip();
        }
    }

    /// Grows (`step > 0`) or shrinks the panel along its current axis, clamped
    /// to the permitted range. The axis follows the position, so `Ctrl+Right`
    /// widens a side panel and heightens a bottom one.
    pub fn resize(&mut self, step: i16) {
        match self.position {
            PreviewPosition::Right => {
                self.width_pct = shift(
                    self.width_pct,
                    step * WIDTH_STEP,
                    MIN_WIDTH_PCT,
                    MAX_WIDTH_PCT,
                );
            }
            PreviewPosition::Bottom => {
                self.height_rows = shift(
                    self.height_rows,
                    step * HEIGHT_STEP,
                    MIN_HEIGHT_ROWS,
                    MAX_HEIGHT_ROWS,
                );
            }
        }
    }
}

/// Adds `delta` to `value`, clamped to `min..=max` without wrapping.
fn shift(value: u16, delta: i16, min: u16, max: u16) -> u16 {
    let moved = i32::from(value) + i32::from(delta);
    moved.clamp(i32::from(min), i32::from(max)) as u16
}

/// What the preview renders, bundled to keep the parameter count low.
pub struct PreviewContext<'a> {
    /// The cursor entry, if any.
    pub repo: Option<&'a Repo>,
    /// The glyph set.
    pub icons: &'a IconSet,
    /// The colour roles resolved from the active theme.
    pub colors: &'a Colors,
    /// Whether to show example git info instead of live status.
    pub example_mode: bool,
    /// The cached `git log --oneline` lines for the entry (may be empty).
    pub log: &'a [String],
    /// Whether the entry's log is still being fetched in the background.
    pub log_loading: bool,
    /// The entry's cached code and size statistics, when computed.
    pub code: Option<&'a CodeEntry>,
    /// The entry's cached history statistics, when computed.
    pub git: Option<&'a GitStats>,
    /// The reference time for ages, in unix seconds.
    pub now: i64,
    /// The panel's scroll position, carried across frames.
    pub scroll: &'a Scroll,
}

/// Renders the detail panel into `area`.
pub fn render(frame: &mut Frame, area: Rect, skin: &Skin, ctx: PreviewContext) {
    let title = match ctx.repo {
        Some(repo) => format!("Details - {}", repo.display_name()),
        None => "Details".to_string(),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(ratada::style::border(&skin.palette))
        .padding(Padding::horizontal(1))
        .title(ratada::chrome::border_title(
            skin,
            &title,
            ratada::style::accent(&skin.palette),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width < MIN_INNER_WIDTH || inner.height == 0 {
        return;
    }

    let Some(repo) = ctx.repo else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "no entry",
                Style::default().fg(ctx.colors.muted),
            ))),
            inner,
        );
        return;
    };

    let lines = entry_lines(repo, &ctx, inner.width as usize);
    let total = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    let offset = ctx.scroll.fit(total, inner.height);
    frame.render_widget(Paragraph::new(lines).scroll((offset, 0)), inner);
    ratada::scroll::render_scrollbar(
        frame,
        inner,
        skin,
        ratada::nav::ScrollView {
            total: total as usize,
            offset: offset as usize,
            viewport: inner.height as usize,
        },
    );
}

/// The detail lines for `repo`.
fn entry_lines(
    repo: &Repo,
    ctx: &PreviewContext,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::raw(""),
        Line::from(Span::styled(
            truncate(&repo.path.to_string_lossy(), width),
            Style::default().fg(ctx.colors.muted),
        )),
        Line::raw(""),
    ];
    push_identity(&mut lines, repo, ctx);
    push_code(&mut lines, ctx, width);
    if repo.kind == RepoKind::Git {
        push_git(&mut lines, repo, ctx, width);
    }
    push_log(&mut lines, ctx, width);
    lines
}

/// The kind, slug, section and usage counters, all already in memory.
fn push_identity(
    lines: &mut Vec<Line<'static>>,
    repo: &Repo,
    ctx: &PreviewContext,
) {
    let colors = ctx.colors;
    lines.push(field("Kind", kind_label(repo), LABEL_WIDTH, colors));
    if let Some(slug) = &repo.slug {
        lines.push(field("Slug", slug, LABEL_WIDTH, colors));
    }
    if let Some(section) = &repo.section {
        lines.push(field("Section", section, LABEL_WIDTH, colors));
    }
    if repo.fav {
        lines.push(field("Favourite", "yes", LABEL_WIDTH, colors));
    }
    lines.push(field(
        "Opens",
        &format_count(repo.open_count),
        LABEL_WIDTH,
        colors,
    ));
    if let Some(last_used) = repo.last_used {
        let age = format!("{} ago", format_age(last_used, ctx.now));
        lines.push(field("Last used", &age, LABEL_WIDTH, colors));
    }
}

/// The CODE section: totals, the size split and the language bars. Skipped when
/// nothing has been counted for this entry yet.
fn push_code(
    lines: &mut Vec<Line<'static>>,
    ctx: &PreviewContext,
    width: usize,
) {
    let Some(entry) = ctx.code else {
        return;
    };
    let colors = ctx.colors;
    let total = entry.code.total();
    lines.push(Line::raw(""));
    lines.push(section_header("Code", width, colors));
    lines.push(field(
        "Lines",
        &format_count(total.code as u64),
        LABEL_WIDTH,
        colors,
    ));
    let files = format!(
        "{} source / {} total",
        format_count(total.files as u64),
        format_count(entry.disk.files)
    );
    lines.push(field("Files", &files, LABEL_WIDTH, colors));
    let mut size = format_bytes(entry.disk.total_bytes);
    if entry.disk.git_bytes > 0 {
        size.push_str(&format!(
            " (.git {})",
            format_bytes(entry.disk.git_bytes)
        ));
    }
    lines.push(field("Size", &size, LABEL_WIDTH, colors));
    for share in language_shares(&entry.code, LANGUAGES_SHOWN) {
        let percent = (share.share * 100.0).round() as u32;
        let value = format!("{}  {percent:>3} %", bar(share.share, BAR_WIDTH));
        lines.push(field(&share.name, &value, LABEL_WIDTH, colors));
    }
}

/// The GIT section: the live status plus the history the worker gathered.
fn push_git(
    lines: &mut Vec<Line<'static>>,
    repo: &Repo,
    ctx: &PreviewContext,
    width: usize,
) {
    let colors = ctx.colors;
    lines.push(Line::raw(""));
    lines.push(section_header("Git", width, colors));

    let info = effective_info(repo, ctx.example_mode);
    match info {
        Some(info) => {
            if let Some(branch) = &info.current_branch_name {
                lines.push(field("Branch", branch, LABEL_WIDTH, colors));
            }
            lines.push(field(
                "Status",
                &status_text(info, ctx.icons),
                LABEL_WIDTH,
                colors,
            ));
            if let Some(name) = &info.github_repo_name {
                lines.push(field("GitHub", name, LABEL_WIDTH, colors));
            }
        }
        None => lines.push(field("Status", "\u{2026}", LABEL_WIDTH, colors)),
    }

    let Some(stats) = ctx.git else {
        return;
    };
    let commits = format!(
        "{}  \u{b7}  {} last 30d",
        format_count(stats.commits),
        format_count(stats.commits_recent)
    );
    lines.push(field("Commits", &commits, LABEL_WIDTH, colors));
    let authors = format!(
        "{}  \u{b7}  {} branches",
        format_count(stats.contributors),
        format_count(stats.branches)
    );
    lines.push(field("Authors", &authors, LABEL_WIDTH, colors));
    if stats.tags > 0 {
        lines.push(field(
            "Tags",
            &format_count(stats.tags),
            LABEL_WIDTH,
            colors,
        ));
    }
    if let Some(first) = stats.first_commit {
        lines.push(field(
            "Age",
            &format_age(first, ctx.now),
            LABEL_WIDTH,
            colors,
        ));
    }
    if let Some(last) = stats.last_commit {
        let age = format!("{} ago", format_age(last, ctx.now));
        lines.push(field("Last commit", &age, LABEL_WIDTH, colors));
    }
}

/// The LOG section, or a placeholder while it is still being fetched.
fn push_log(
    lines: &mut Vec<Line<'static>>,
    ctx: &PreviewContext,
    width: usize,
) {
    if ctx.log.is_empty() && !ctx.log_loading {
        return;
    }
    lines.push(Line::raw(""));
    lines.push(section_header("Log", width, ctx.colors));
    if ctx.log.is_empty() {
        lines.push(Line::from(Span::styled(
            "loading\u{2026}",
            Style::default().fg(ctx.colors.dim),
        )));
        return;
    }
    for entry in ctx.log {
        lines.push(log_line(&truncate(entry, width), ctx.colors));
    }
}

/// The type label for the entry.
fn kind_label(repo: &Repo) -> &'static str {
    match repo.kind {
        RepoKind::Git => "git",
        RepoKind::Path if is_dir_target(&repo.path) => "folder",
        RepoKind::Path => "file",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_round_trips_through_its_key() {
        for key in ["off", "right", "bottom"] {
            assert_eq!(PreviewLayout::from_state(key, 40, 9).as_key(), key);
        }
        // An unknown key hides the panel rather than guessing.
        assert!(!PreviewLayout::from_state("bogus", 40, 9).visible);
    }

    #[test]
    fn from_state_clamps_a_size_written_by_hand() {
        let layout = PreviewLayout::from_state("right", 5, 900);
        assert_eq!(layout.width_pct, MIN_WIDTH_PCT);
        assert_eq!(layout.height_rows, MAX_HEIGHT_ROWS);
    }

    #[test]
    fn toggle_shows_and_hides_without_losing_the_position() {
        let mut layout = PreviewLayout::from_state("bottom", 40, 9);
        layout.toggle();
        assert!(!layout.visible);
        layout.toggle();
        assert!(layout.visible);
        assert_eq!(layout.position, PreviewPosition::Bottom);
    }

    #[test]
    fn flip_position_leaves_a_hidden_panel_alone() {
        let mut hidden = PreviewLayout::from_state("off", 40, 9);
        hidden.flip_position();
        assert!(!hidden.visible, "V must not open the panel");

        let mut shown = PreviewLayout::from_state("right", 40, 9);
        shown.flip_position();
        assert_eq!(shown.position, PreviewPosition::Bottom);
        assert_eq!(shown.as_key(), "bottom");
    }

    #[test]
    fn resize_follows_the_axis_and_clamps() {
        let mut right = PreviewLayout::from_state("right", 40, 9);
        right.resize(1);
        assert_eq!(right.width_pct, 45);
        assert_eq!(right.height_rows, 9, "the other axis is untouched");
        for _ in 0..20 {
            right.resize(1);
        }
        assert_eq!(right.width_pct, MAX_WIDTH_PCT);
        for _ in 0..40 {
            right.resize(-1);
        }
        assert_eq!(right.width_pct, MIN_WIDTH_PCT);

        let mut bottom = PreviewLayout::from_state("bottom", 40, 9);
        bottom.resize(-1);
        assert_eq!(bottom.height_rows, 8);
        assert_eq!(bottom.width_pct, 40);
    }
}
