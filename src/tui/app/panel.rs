//! The detail panel: where it sits, how big it is, and the lazily fetched git
//! log it shows.

use super::*;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Local;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};

use crate::domain::repo::RepoKind;
use crate::service::preview_service::{self};

/// How long the cursor must rest on an entry before its preview `git log` is
/// fetched, so quick scrolling does not spawn a fetch per row.
pub(super) const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(120);

/// How many `git log` lines the preview shows.
pub(super) const PREVIEW_LOG_LINES: usize = 5;

/// The blank cell (or row) between the list and the detail panel.
/// How many rows one page of the detail panel scrolls by.
pub(super) const PREVIEW_PAGE_ROWS: i32 = 10;

pub(super) const PANEL_GUTTER: u16 = 1;
/// The narrowest list a side panel may squeeze the body down to, in columns.
pub(super) const MIN_LIST_COLS: u16 = 20;
/// The shortest list a bottom panel may squeeze the body down to, in rows.
/// A column count would leave no room for the panel at all on a normal screen.
pub(super) const MIN_LIST_ROWS: u16 = 3;

impl App {
    /// Shows or hides the detail panel and persists the choice.
    pub(super) fn toggle_preview(&mut self) {
        self.preview.toggle();
        self.preview_scroll.reset();
        self.save_ui_state();
    }

    /// Moves the detail panel to the other side.
    pub(super) fn flip_preview_position(&mut self) {
        self.preview.flip_position();
        self.save_ui_state();
    }

    /// Grows or shrinks the detail panel along its current axis.
    pub(super) fn resize_preview(&mut self, step: i16) {
        if !self.preview.visible {
            return;
        }
        self.preview.resize(step);
        self.save_ui_state();
    }

    /// Scrolls the detail panel.
    pub(super) fn scroll_preview(&mut self, delta: i32) {
        if self.preview.visible {
            self.preview_scroll.scroll_by(delta);
        }
    }

    /// Scrolls the detail panel by a whole page.
    pub(super) fn page_preview(&mut self, pages: i32) {
        if self.preview.visible {
            self.preview_scroll.scroll_by(pages * PREVIEW_PAGE_ROWS);
        }
    }

    /// Splits `body` into the list area and an optional panel area, per the
    /// active [`PreviewLayout`]. A one-cell gutter separates the two, so the
    /// panel's border never touches the list.
    pub(super) fn split_preview(&self, body: Rect) -> (Rect, Option<Rect>) {
        if !self.preview.visible {
            return (body, None);
        }
        let (direction, minimum, panel) = match self.preview.position {
            crate::tui::preview::PreviewPosition::Right => (
                Direction::Horizontal,
                MIN_LIST_COLS,
                Constraint::Percentage(self.preview.width_pct),
            ),
            crate::tui::preview::PreviewPosition::Bottom => (
                Direction::Vertical,
                MIN_LIST_ROWS,
                Constraint::Length(self.preview.height_rows),
            ),
        };
        let parts = Layout::default()
            .direction(direction)
            .constraints([
                Constraint::Min(minimum),
                Constraint::Length(PANEL_GUTTER),
                panel,
            ])
            .split(body);
        (parts[0], Some(parts[2]))
    }

    /// The cursor entry's path when it is a git repo whose log the preview
    /// would show (preview visible, not example mode); otherwise `None`.
    pub(super) fn preview_log_path(&self) -> Option<PathBuf> {
        if !self.preview.visible || self.config.example_mode {
            return None;
        }
        let repo = self.selected_index().and_then(|i| self.service.get(i))?;
        if repo.kind != RepoKind::Git {
            return None;
        }
        Some(repo.path.clone())
    }

    /// Requests the cursor entry's preview `git log` once the cursor has rested
    /// on it for [`PREVIEW_DEBOUNCE`], so quick scrolling never blocks. The
    /// fetch itself runs on a background worker (see [`fetch_logs`]).
    pub(super) fn request_preview_log(&mut self) {
        let Some(path) = self.preview_log_path() else {
            self.preview_target = None;
            return;
        };
        if self.preview_log.contains_key(&path)
            || self.preview_pending.contains(&path)
        {
            return;
        }
        if self.preview_target.as_deref() != Some(&path) {
            self.preview_target = Some(path);
            self.preview_target_at = Instant::now();
            return;
        }
        if self.preview_target_at.elapsed() < PREVIEW_DEBOUNCE {
            return;
        }
        self.fetch_logs(vec![path]);
    }

    /// Spawns a background worker to fetch the preview logs for `paths`,
    /// skipping any already cached and marking the rest as pending.
    pub(super) fn fetch_logs(&mut self, paths: Vec<PathBuf>) {
        let wanted: Vec<PathBuf> = paths
            .into_iter()
            .filter(|path| !self.preview_log.contains_key(path))
            .collect();
        if wanted.is_empty() {
            return;
        }
        for path in &wanted {
            self.preview_pending.insert(path.clone());
        }
        preview_service::spawn_logs(
            Arc::clone(&self.git_client),
            wanted,
            PREVIEW_LOG_LINES,
            self.preview_tx.clone(),
        );
    }

    /// Applies any background preview-log results without blocking.
    pub(super) fn drain_preview(&mut self) {
        while let Ok(log) = self.preview_rx.try_recv() {
            self.preview_pending.remove(&log.path);
            self.preview_log.insert(log.path, log.lines);
        }
    }

    /// Whether the preview is waiting on a log (debouncing or fetching), so the
    /// loop should poll faster to pick the result up promptly.
    pub(super) fn preview_busy(&self) -> bool {
        if !self.preview_pending.is_empty() {
            return true;
        }
        self.preview_log_path()
            .is_some_and(|path| !self.preview_log.contains_key(&path))
    }

    /// Drops the cached and pending preview logs for `paths`, so they are
    /// re-fetched on demand. Does nothing for paths not in the cache.
    pub(super) fn invalidate_logs(&mut self, paths: &[PathBuf]) {
        for path in paths {
            self.preview_log.remove(path);
            self.preview_pending.remove(path);
        }
    }

    /// Renders the detail/preview panel for the cursor entry.
    pub(super) fn render_preview(&self, frame: &mut Frame, area: Rect) {
        let repo = self.selected_index().and_then(|i| self.service.get(i));
        let log = repo
            .map(|r| self.preview_log.get(&r.path).map(Vec::as_slice))
            .unwrap_or(None);
        let log_loading = self
            .preview_log_path()
            .is_some_and(|path| !self.preview_log.contains_key(&path));
        crate::tui::preview::render(
            frame,
            area,
            &self.skin,
            crate::tui::preview::PreviewContext {
                repo,
                icons: &self.icons,
                colors: &self.colors,
                example_mode: self.config.example_mode,
                log: log.unwrap_or(&[]),
                log_loading,
                code: repo.and_then(|r| self.stats.code.get(&r.path)),
                git: repo.and_then(|r| self.stats.git.get(&r.path)),
                now: Local::now().timestamp(),
                scroll: &self.preview_scroll,
            },
        );
    }
}
