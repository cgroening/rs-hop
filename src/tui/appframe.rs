//! The shared app-frame chrome: the tinted, borderless header panel with the
//! tab bar, the content-surface / status-band tints and the backgroundless
//! key-hint band (clibase panel layout).
//!
//! Every view renders through [`render_frame`], which draws the panels and
//! returns the content [`Rect`] the caller fills, so the app frame is identical
//! in every view (no border lines on the frame itself). Modals dim the last
//! drawn frame (snapshotted by [`snapshot_frame`]) rather than a black screen.

use std::cell::RefCell;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::Line;
use ratatui::widgets::{Block, Clear, Padding, Paragraph};

use crate::theme::Skin;

/// The brand shown at the start of the tab bar.
pub const BRAND: &str = "hop";

/// The two primary tabs (key, base label) in view order. Each kind's archive is
/// reached by a second press of its key, not shown as its own tab.
const TAB_NAMES: [(&str, &str); 2] =
    [("1", "Git Repos"), ("2", "Files and Folders")];

/// Suffix added to the active tab's label when its archive is showing.
const ARCHIVE_SUFFIX: &str = " \u{00b7} Archiv";

/// Which tab the header bar highlights and whether its archive is showing.
#[derive(Debug, Clone, Copy)]
pub struct TabBar {
    /// The active kind index (0 = git, 1 = files).
    pub active: usize,
    /// Whether the active kind's archive view is showing.
    pub archived: bool,
}

/// The tab (key, label) pairs, suffixing the active label with `Archiv` when its
/// archive is showing.
fn tab_labels(bar: TabBar) -> Vec<(&'static str, String)> {
    TAB_NAMES
        .iter()
        .enumerate()
        .map(|(index, (key, name))| {
            let label = if index == bar.active && bar.archived {
                format!("{name}{ARCHIVE_SUFFIX}")
            } else {
                (*name).to_string()
            };
            (*key, label)
        })
        .collect()
}

/// Borrows the owned tab labels into the `(&str, &str)` pairs the toolkit takes.
fn borrow_pairs<'a>(
    labels: &'a [(&'static str, String)],
) -> Vec<(&'static str, &'a str)> {
    labels
        .iter()
        .map(|(key, label)| (*key, label.as_str()))
        .collect()
}

/// How far a modal darkens the view behind it (`0.0` = black, `1.0` = as-is);
/// mirrors `ratada::overlay`'s own scrim so hand-rolled and toolkit modals dim
/// identically.
const SCRIM_FACTOR: f32 = 0.4;

/// `OKLab` lightness lift for a popup fill above the content surface, so a modal
/// reads as a raised, brighter panel over the dimmed backdrop.
const POPUP_LIGHTEN: f32 = 0.08;

thread_local! {
    /// The last fully-drawn view frame, snapshotted by [`snapshot_frame`] so a
    /// following modal can dim it as its backdrop instead of a black screen.
    static LAST_FRAME: RefCell<Option<Buffer>> = const { RefCell::new(None) };
}

/// Snapshots the just-rendered frame; call at the very end of a view draw so a
/// modal opened next can dim it as its backdrop (see [`dim_backdrop`]).
pub fn snapshot_frame(frame: &mut Frame<'_>) {
    let snapshot = frame.buffer_mut().clone();
    LAST_FRAME.with(|last| *last.borrow_mut() = Some(snapshot));
}

/// Blits the last snapshotted view into `frame` as a modal backdrop (no dim). A
/// no-op until a frame is snapshotted or if the terminal was resized since.
pub fn blit_backdrop(frame: &mut Frame<'_>) {
    LAST_FRAME.with(|last| {
        let borrow = last.borrow();
        if let Some(buf) = borrow.as_ref() {
            let dst = frame.buffer_mut();
            if buf.area == dst.area {
                dst.content.clone_from(&buf.content);
            }
        }
    });
}

/// Blits the last snapshotted view and dims it - the backdrop for a modal so it
/// overlays the dimmed live view rather than a black screen.
pub fn dim_backdrop(frame: &mut Frame<'_>) {
    blit_backdrop(frame);
    ratada::overlay::dim(frame, SCRIM_FACTOR);
}

/// Prepares a modal region: dims the live view behind it (backdrop), clears the
/// popup `area` and fills it a step lighter so the box stands out from the
/// dimmed view. Boxes drawn via [`ratada::chrome::modal_block`] repaint their own
/// (matching) fill on top.
pub fn popup_backdrop(frame: &mut Frame<'_>, skin: &Skin, area: Rect) {
    dim_backdrop(frame);
    frame.render_widget(Clear, area);
    let bg = skin.palette.surface.lighten(POPUP_LIGHTEN);
    frame.render_widget(Block::default().style(ratada::style::bg(bg)), area);
}

/// Height of the progress panel above the status band (a refresh/backup bar).
const PROGRESS_HEIGHT: u16 = 1;

/// The rects [`render_frame`] hands back for the caller to paint into: the
/// content surface, plus the optional progress-bar region above the status band
/// (present only while a refresh or backup is running).
pub struct FrameAreas {
    /// The content surface (inset by one cell left/right).
    pub content: Rect,
    /// The progress-bar row above the status band, when shown.
    pub progress: Option<Rect>,
}

/// Renders the full app frame: the tinted header panel, a 1-row content padding,
/// the tinted content surface, an optional progress region (when
/// `show_progress`), the tinted status band (`status` lines) and the
/// backgroundless hints below a blank separator. Returns the rects the caller
/// paints into (see [`FrameAreas`]).
pub fn render_frame(
    frame: &mut Frame<'_>,
    skin: &Skin,
    tabs: TabBar,
    status: Vec<Line>,
    hints: &[(String, Vec<(String, String)>)],
    show_progress: bool,
) -> FrameAreas {
    let area = frame.area();
    frame.render_widget(
        Block::default().style(ratada::style::base(
            skin.palette.foreground,
            skin.palette.background,
        )),
        area,
    );
    let width = area.width.saturating_sub(2) as usize;
    let labels = tab_labels(tabs);
    let pairs = borrow_pairs(&labels);
    let header_h = header_height(&pairs, width);
    let status_h = status.len() as u16;
    let progress_h = if show_progress { PROGRESS_HEIGHT } else { 0 };
    let hints_h = if hints.is_empty() {
        0
    } else {
        hints_height(hints, width)
    };
    let chunks = Layout::vertical([
        Constraint::Length(header_h),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(progress_h),
        Constraint::Length(status_h),
        Constraint::Length(hints_h),
    ])
    .split(area);

    fill(frame, chunks[1], skin.palette.surface);
    fill(frame, chunks[2], skin.palette.surface);
    render_header(frame, chunks[0], skin, &pairs, tabs.active);
    if status_h > 0 {
        fill(frame, chunks[4], skin.palette.footer);
        frame.render_widget(
            Paragraph::new(status)
                .block(Block::default().padding(Padding::horizontal(1))),
            chunks[4],
        );
    }
    if hints_h > 0 {
        render_hints(frame, chunks[5], skin, hints);
    }
    // The content surface, inset by one cell left/right; the dedicated content-
    // padding row above already gives the top gap.
    let content = Block::default()
        .padding(Padding::new(1, 1, 0, 0))
        .inner(chunks[2]);
    let progress = (progress_h > 0).then_some(chunks[3]);
    FrameAreas { content, progress }
}

/// The header-panel height at `width`: the tab rows plus the 1-cell panel
/// padding top and bottom.
fn header_height(pairs: &[(&str, &str)], width: usize) -> u16 {
    ratada::tabs::height(BRAND, pairs, width) + 2
}

/// Renders the tinted, borderless header panel with the tab bar.
fn render_header(
    frame: &mut Frame<'_>,
    area: Rect,
    skin: &Skin,
    pairs: &[(&str, &str)],
    active: usize,
) {
    let block = Block::default()
        .style(ratada::style::bg(skin.palette.header))
        .padding(Padding::uniform(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let tab_height = ratada::tabs::height(BRAND, pairs, inner.width as usize);
    let tab_area = Rect {
        height: tab_height.min(inner.height),
        ..inner
    };
    ratada::tabs::render(frame, tab_area, skin, BRAND, pairs, active);
}

/// Fills `area` with a solid background tint.
fn fill(frame: &mut Frame<'_>, area: Rect, color: crate::theme::Color) {
    frame.render_widget(Block::default().style(ratada::style::bg(color)), area);
}

/// The key-hint band style: keys in the accent (bold), descriptions dim, a
/// 1-row top margin (the blank separator line) and no background.
fn hint_style(skin: &Skin) -> ratada::shortcut_hints::HintStyle {
    ratada::shortcut_hints::HintStyle {
        key: ratada::style::fg(skin.palette.accent)
            .add_modifier(Modifier::BOLD),
        label: ratada::style::dim(),
        description: ratada::style::secondary(&skin.palette),
        top_margin: 1,
        background: None,
    }
}

/// Borrows the owned `(label, pairs)` hint groups into toolkit
/// [`HintGroup`](ratada::shortcut_hints::HintGroup)s (aligned label column).
fn to_hint_groups(
    groups: &[(String, Vec<(String, String)>)],
) -> Vec<ratada::shortcut_hints::HintGroup<'_, String>> {
    groups
        .iter()
        .map(|(label, pairs)| ratada::shortcut_hints::HintGroup {
            label: label.as_str(),
            hints: pairs.as_slice(),
        })
        .collect()
}

/// The hint-band height for the grouped `hints` at `width`, including the
/// top-margin line.
fn hints_height(
    hints: &[(String, Vec<(String, String)>)],
    width: usize,
) -> u16 {
    ratada::shortcut_hints::height(&to_hint_groups(hints), width, 1)
}

/// Renders the backgroundless, grouped key hints, inset by 1 cell to match the
/// panels, with the blank separator supplied by `top_margin`.
fn render_hints(
    frame: &mut Frame<'_>,
    area: Rect,
    skin: &Skin,
    hints: &[(String, Vec<(String, String)>)],
) {
    let opts = hint_style(skin);
    let hint_area = Rect {
        x: area.x + 1,
        width: area.width.saturating_sub(2),
        ..area
    };
    ratada::shortcut_hints::render(
        frame,
        hint_area,
        &to_hint_groups(hints),
        &opts,
    );
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn skin() -> Skin {
        crate::config::Config::default().skin()
    }

    /// Renders the frame and returns the content and progress rects.
    fn areas(show_progress: bool) -> FrameAreas {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut out = None;
        terminal
            .draw(|frame| {
                out = Some(render_frame(
                    frame,
                    &skin(),
                    TabBar {
                        active: 0,
                        archived: false,
                    },
                    vec![Line::raw("info")],
                    &[(
                        "App".to_string(),
                        vec![("q".to_string(), "quit".to_string())],
                    )],
                    show_progress,
                ));
            })
            .unwrap();
        out.unwrap()
    }

    #[test]
    fn reserves_a_progress_region_when_shown() {
        let with = areas(true);
        let progress = with.progress.expect("progress region when shown");
        assert_eq!(progress.height, PROGRESS_HEIGHT);
        // The content shrinks by the progress height versus the hidden case.
        let without = areas(false);
        assert!(without.progress.is_none());
        assert_eq!(
            without.content.height - with.content.height,
            PROGRESS_HEIGHT,
        );
    }
}
