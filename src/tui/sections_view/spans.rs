//! The spans of an entry row: its stats, its files and its git status, and the
//! widths of the git columns.

use super::*;

/// The statistics cells of one entry, right-aligned under their headers.
pub(super) fn stat_spans(
    view: &SectionedView,
    repo: &Repo,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for column in stat_columns(view.columns) {
        let text = view.stat_text(repo, *column);
        let width = column.width() as usize;
        let cell = if column.is_numeric() {
            format!("{text:>width$}")
        } else {
            pad(&text, width)
        };
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            cell,
            Style::default().fg(view.colors.foreground),
        ));
    }
    spans
}

/// The Files-tab columns after the name/slug prefix: type, the dim path and the
/// ZIP date. `prefix` is the name column plus the optional slug column.
pub(super) fn files_spans(
    repo: &Repo,
    view: &SectionedView,
    prefix: usize,
    width: usize,
) -> Vec<Span<'static>> {
    let kind = pad(type_label(repo), TYPE_WIDTH);
    let path = pad(
        &repo.path.to_string_lossy(),
        files_path_width(prefix, width),
    );
    let zip = format!("{:>ZIP_WIDTH$}", zip_cell_text(repo, view));
    vec![
        Span::raw("  "),
        Span::raw(kind),
        Span::raw("  "),
        Span::styled(path, Style::default().fg(view.colors.dim)),
        Span::raw("  "),
        Span::styled(zip, Style::default().fg(view.colors.dim)),
    ]
}

/// The git-tab columns after the name: branch and GitHub name in the foreground
/// (like the name), the coloured status, and the dim ZIP date. GitHub flexes
/// into the leftover width.
pub(super) fn git_spans(
    repo: &Repo,
    view: &SectionedView,
    prefix: usize,
    width: usize,
) -> Vec<Span<'static>> {
    let (branch_w, status_w, github_w) = git_column_widths(view, prefix, width);
    let info = effective_info(repo, view.example_mode);
    let branch = pad(&branch_text(info), branch_w);
    let github = pad(&github_text(info), github_w);
    let zip = format!("{:>ZIP_WIDTH$}", zip_cell_text(repo, view));
    let fg = Style::default().fg(view.colors.foreground);
    let mut spans =
        vec![Span::raw("  "), Span::styled(branch, fg), Span::raw("  ")];
    spans.extend(git_status_spans(repo, view, info, status_w));
    spans.extend([
        Span::raw("  "),
        Span::styled(github, fg),
        Span::raw("  "),
        Span::styled(zip, Style::default().fg(view.colors.dim)),
    ]);
    spans
}

/// The status spans padded to `width`: the spinner while the row is in flight,
/// otherwise the shared coloured status followed by padding.
pub(super) fn git_status_spans(
    repo: &Repo,
    view: &SectionedView,
    info: Option<&crate::domain::repo::GitInfo>,
    width: usize,
) -> Vec<Span<'static>> {
    if view.is_in_flight(repo) {
        return vec![Span::styled(
            pad(view.spinner_glyph(), width),
            Style::default().fg(view.colors.accent),
        )];
    }
    let text = status_display(info, view.icons);
    let used = UnicodeWidthStr::width(text.as_str()).min(width);
    vec![
        status_span(info, view.icons, view.colors),
        Span::raw(" ".repeat(width.saturating_sub(used))),
    ]
}

/// The git columns' widths: branch and status sized to their bounded content,
/// GitHub taking the leftover width (so the row fills without overflowing).
pub(super) fn git_column_widths(
    view: &SectionedView,
    prefix: usize,
    width: usize,
) -> (usize, usize, usize) {
    let branch = col_content(view, |r| {
        branch_text(effective_info(r, view.example_mode))
    })
    .clamp(BRANCH_MIN, BRANCH_MAX);
    let status = col_content(view, |r| {
        status_display(effective_info(r, view.example_mode), view.icons)
    })
    .clamp(STATUS_MIN, STATUS_MAX);
    // lead(2) + marker(1) + fav(1) + space(1) + prefix + gap(2) + branch
    // + gap(2) + status + gap(2) + github + gap(2) + ZIP; GitHub flexes into
    // the rest.
    // `prefix` = the name column plus the optional slug column.
    let used =
        2 + 1 + 1 + 1 + prefix + 2 + branch + 2 + status + 2 + 2 + ZIP_WIDTH;
    let github = width.saturating_sub(used);
    (branch, status, github)
}
