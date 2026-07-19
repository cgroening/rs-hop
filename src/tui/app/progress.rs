//! The progress bar shown above the status band while a refresh or a ZIP
//! backup runs, and the fixed-width label that keeps its columns from shifting.

use super::*;

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::theme::Skin;

/// Progress-bar label while a background status refresh runs.
pub(super) const REFRESH_LABEL: &str = "refreshing";

/// Progress-bar label while a background ZIP backup runs.
pub(super) const ZIP_LABEL: &str = "zipping";

/// Padding width of the percentage in the progress text, so `XX %` keeps a
/// constant width from `0` through `100`.
pub(super) const PERCENT_WIDTH: usize = 3;

/// Separator between the percentage/counts prefix and the entry name.
pub(super) const PROGRESS_SEPARATOR: &str = " - ";

/// The fill ratio for `done` of `total`, clamped to `0.0..=1.0` (0 when empty).
pub(super) fn progress_ratio(done: usize, total: usize) -> f64 {
    if total == 0 {
        return 0.0;
    }
    (done as f64 / total as f64).clamp(0.0, 1.0)
}

/// The number of decimal digits in `n` (at least 1), for padding counts.
pub(super) fn digit_count(n: usize) -> usize {
    n.to_string().len()
}

/// The composed text and fill ratio for one frame of the progress bar.
pub(super) struct ProgressText<'a> {
    /// The fixed-width leading part (percentage, plus file counts when zipping).
    pub(super) prefix: &'a str,
    /// The entry name shown after the prefix; empty when none is known yet.
    pub(super) name: &'a str,
    /// Fill ratio in `0.0..=1.0`.
    pub(super) ratio: f64,
    /// Display width reserved for the name, so the prefix column stays put as
    /// names of different lengths come and go.
    pub(super) name_width: usize,
}

impl ProgressText<'_> {
    /// The bar's label, `prefix - name`, with the name padded to the widest name
    /// of the run. The gauge centres the label, so a constant label width pins
    /// the `XX %` column even as names of different lengths come and go. The
    /// name column is reserved before the first name arrives; the separator
    /// appears only once there is a name to separate.
    pub(super) fn label(&self) -> String {
        let separator = if self.name.is_empty() {
            " ".repeat(PROGRESS_SEPARATOR.chars().count())
        } else {
            PROGRESS_SEPARATOR.to_string()
        };
        format!(
            "{}{separator}{:<width$}",
            self.prefix,
            self.name,
            width = self.name_width
        )
    }
}

/// Renders a solid progress bar for an in-flight operation (status refresh or
/// ZIP backup) across `area`, leaving one blank cell of padding on each side.
pub(super) fn render_progress(
    frame: &mut Frame,
    area: Rect,
    skin: &Skin,
    text: ProgressText,
) {
    let area = Rect {
        x: area.x.saturating_add(1),
        width: area.width.saturating_sub(2),
        ..area
    };
    ratada::gauge::render(
        frame,
        area,
        &skin.palette,
        text.ratio,
        &text.label(),
    );
}

impl App {
    /// Paints the refresh/backup progress bar (pre-migration style) into the
    /// panel reserved above the status band, when a run is in flight.
    pub(super) fn render_progress_bar(&self, frame: &mut Frame, area: Rect) {
        let Some((done, total)) = self.loading else {
            return;
        };
        let ratio = progress_ratio(done, total);
        let prefix = self.progress_prefix(ratio, done, total);
        render_progress(
            frame,
            area,
            &self.skin,
            ProgressText {
                prefix: &prefix,
                name: self.loading_detail.as_deref().unwrap_or(""),
                ratio,
                name_width: self.loading_name_width,
            },
        );
    }

    /// The fixed-width leading part of the progress text: the percentage, plus
    /// the file counts while zipping. Widths are padded so the part keeps a
    /// constant width, pinning the `XX %` column for the whole run.
    pub(super) fn progress_prefix(
        &self,
        ratio: f64,
        done: usize,
        total: usize,
    ) -> String {
        let pct = (ratio * 100.0).round() as u16;
        let pw = PERCENT_WIDTH;
        if self.loading_label == ZIP_LABEL {
            let cw = digit_count(total);
            format!("{pct:>pw$} % ({done:>cw$}/{total})")
        } else {
            format!("{pct:>pw$} %")
        }
    }
}
