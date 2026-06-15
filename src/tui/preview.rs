//! The optional detail/preview panel for the cursor entry.
//!
//! Shows the entry's in-memory fields (path, slug, section, kind, git status,
//! usage) plus a cached `git log` excerpt. The log is gathered lazily by the
//! app and passed in here; rendering itself does no I/O.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crate::domain::repo::{Repo, RepoKind, is_dir_target};
use crate::tui::colors::{ACCENT, DIM, MUTED};
use crate::tui::presentation::{IconSet, status_text, truncate};

/// Where the preview panel sits (persisted; cycled with `v`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreviewMode {
    /// Hidden.
    #[default]
    Off,
    /// To the right of the list.
    Right,
    /// Below the list.
    Bottom,
}

impl PreviewMode {
    /// The next mode in the cycle (off -> right -> bottom -> off).
    pub fn next(self) -> Self {
        match self {
            PreviewMode::Off => PreviewMode::Right,
            PreviewMode::Right => PreviewMode::Bottom,
            PreviewMode::Bottom => PreviewMode::Off,
        }
    }

    /// The stable key used to persist the mode.
    pub fn as_key(self) -> &'static str {
        match self {
            PreviewMode::Off => "off",
            PreviewMode::Right => "right",
            PreviewMode::Bottom => "bottom",
        }
    }

    /// Parses a persisted [`as_key`](Self::as_key) value, defaulting to off.
    pub fn from_key(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "right" => PreviewMode::Right,
            "bottom" => PreviewMode::Bottom,
            _ => PreviewMode::Off,
        }
    }
}

/// What the preview renders, bundled to keep the parameter count low.
pub struct PreviewContext<'a> {
    /// The cursor entry, if any.
    pub repo: Option<&'a Repo>,
    /// The glyph set.
    pub icons: &'a IconSet,
    /// Whether to show example git info instead of live status.
    pub example_mode: bool,
    /// The cached `git log --oneline` lines for the entry (may be empty).
    pub log: &'a [String],
    /// Whether the entry's log is still being fetched in the background.
    pub log_loading: bool,
}

/// Renders the preview panel into `area`.
pub fn render(frame: &mut Frame, area: Rect, ctx: PreviewContext) {
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(Span::styled(
            " Details ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(DIM));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(repo) = ctx.repo else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "no entry",
                Style::default().fg(MUTED),
            ))),
            inner,
        );
        return;
    };
    let width = inner.width as usize;
    let lines = entry_lines(repo, &ctx, width);
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The detail lines for `repo`.
fn entry_lines(
    repo: &Repo,
    ctx: &PreviewContext,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled(
        repo.display_name(),
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    ))];
    lines.push(dim_line(truncate(&repo.path.to_string_lossy(), width)));

    let mut meta: Vec<String> = vec![format!("kind: {}", kind_label(repo))];
    if let Some(slug) = &repo.slug {
        meta.push(format!("slug: {slug}"));
    }
    if let Some(section) = &repo.section {
        meta.push(format!("section: {section}"));
    }
    if repo.fav {
        meta.push("fav".to_string());
    }
    lines.push(dim_line(meta.join("  ·  ")));

    if repo.kind == RepoKind::Git {
        push_git_lines(&mut lines, repo, ctx);
    }
    push_usage_line(&mut lines, repo);

    push_log_lines(&mut lines, ctx, width);
    lines
}

/// The placeholder shown while the preview log is still loading.
const LOG_LOADING: &str = "loading…";

/// Appends the `git log` excerpt, or a loading placeholder while it is still
/// being fetched in the background.
fn push_log_lines(
    lines: &mut Vec<Line<'static>>,
    ctx: &PreviewContext,
    width: usize,
) {
    if ctx.log.is_empty() && !ctx.log_loading {
        return;
    }
    lines.push(dim_line("─ log ─".to_string()));
    if ctx.log.is_empty() {
        lines.push(dim_line(LOG_LOADING.to_string()));
        return;
    }
    for entry in ctx.log {
        lines.push(dim_line(truncate(entry, width)));
    }
}

/// Appends the branch/status line for a git entry.
fn push_git_lines(
    lines: &mut Vec<Line<'static>>,
    repo: &Repo,
    ctx: &PreviewContext,
) {
    let info = if ctx.example_mode {
        repo.example_git_info.as_ref()
    } else {
        repo.git_info.as_ref()
    };
    let Some(info) = info else {
        lines.push(dim_line("status: …".to_string()));
        return;
    };
    let mut parts = Vec::new();
    if let Some(branch) = &info.current_branch_name {
        parts.push(format!("[{branch}]"));
    }
    parts.push(status_text(info, ctx.icons));
    if let Some(name) = &info.github_repo_name {
        parts.push(name.clone());
    }
    lines.push(dim_line(parts.join("  ·  ")));
}

/// Appends the usage line (last used / open count) when known.
fn push_usage_line(lines: &mut Vec<Line<'static>>, repo: &Repo) {
    if repo.last_used.is_none() && repo.open_count == 0 {
        return;
    }
    let opens = format!("opens: {}", repo.open_count);
    lines.push(dim_line(opens));
}

/// A dim-styled single-line.
fn dim_line(text: String) -> Line<'static> {
    Line::from(Span::styled(text, Style::default().fg(MUTED)))
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
    fn preview_mode_cycles_and_round_trips() {
        assert_eq!(PreviewMode::Off.next(), PreviewMode::Right);
        assert_eq!(PreviewMode::Right.next(), PreviewMode::Bottom);
        assert_eq!(PreviewMode::Bottom.next(), PreviewMode::Off);
        for mode in [PreviewMode::Off, PreviewMode::Right, PreviewMode::Bottom]
        {
            assert_eq!(PreviewMode::from_key(mode.as_key()), mode);
        }
        assert_eq!(PreviewMode::from_key("bogus"), PreviewMode::Off);
    }
}
