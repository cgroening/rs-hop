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

/// The tab bar labels (key, label) in view order.
pub const TAB_LABELS: [(&str, &str); 3] = [
    ("1", "Git Repos"),
    ("2", "Files and Folders"),
    ("3", "Archiv"),
];

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

/// Renders the full app frame: the tinted header panel, a 1-row content padding,
/// the tinted content surface, the tinted status band (`status` lines) and the
/// backgroundless hints below a blank separator. Returns the content [`Rect`]
/// the caller fills.
pub fn render_frame(
    frame: &mut Frame<'_>,
    skin: &Skin,
    active: usize,
    status: Vec<Line>,
    hints: &[(String, String)],
) -> Rect {
    let area = frame.area();
    frame.render_widget(
        Block::default().style(ratada::style::base(
            skin.palette.foreground,
            skin.palette.background,
        )),
        area,
    );
    let width = area.width.saturating_sub(2) as usize;
    let header_h = header_height(width);
    let status_h = status.len() as u16;
    let hints_h = if hints.is_empty() {
        0
    } else {
        hints_height(hints, width)
    };
    let chunks = Layout::vertical([
        Constraint::Length(header_h),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(status_h),
        Constraint::Length(hints_h),
    ])
    .split(area);

    fill(frame, chunks[1], skin.palette.surface);
    fill(frame, chunks[2], skin.palette.surface);
    render_header(frame, chunks[0], skin, active);
    if status_h > 0 {
        fill(frame, chunks[3], skin.palette.footer);
        frame.render_widget(
            Paragraph::new(status)
                .block(Block::default().padding(Padding::horizontal(1))),
            chunks[3],
        );
    }
    if hints_h > 0 {
        render_hints(frame, chunks[4], skin, hints);
    }
    // The content surface, inset by one cell left/right; the dedicated content-
    // padding row above already gives the top gap.
    Block::default()
        .padding(Padding::new(1, 1, 0, 0))
        .inner(chunks[2])
}

/// The header-panel height at `width`: the tab rows plus the 1-cell panel
/// padding top and bottom.
fn header_height(width: usize) -> u16 {
    ratada::tabs::height(BRAND, &TAB_LABELS, width) + 2
}

/// Renders the tinted, borderless header panel with the tab bar.
fn render_header(
    frame: &mut Frame<'_>,
    area: Rect,
    skin: &Skin,
    active: usize,
) {
    let block = Block::default()
        .style(ratada::style::bg(skin.palette.header))
        .padding(Padding::uniform(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let tab_height =
        ratada::tabs::height(BRAND, &TAB_LABELS, inner.width as usize);
    let tab_area = Rect {
        height: tab_height.min(inner.height),
        ..inner
    };
    ratada::tabs::render(frame, tab_area, skin, BRAND, &TAB_LABELS, active);
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

/// One flat, label-less hint group over the `(key, description)` pairs.
fn hint_group(
    hints: &[(String, String)],
) -> [ratada::shortcut_hints::HintGroup<'_, String>; 1] {
    [ratada::shortcut_hints::HintGroup { label: "", hints }]
}

/// The hint-band height for the flat `hints` at `width`, including the
/// top-margin line.
fn hints_height(hints: &[(String, String)], width: usize) -> u16 {
    ratada::shortcut_hints::height(&hint_group(hints), width, 1)
}

/// Renders the backgroundless, flat key hints, inset by 1 cell to match the
/// panels, with the blank separator supplied by `top_margin`.
fn render_hints(
    frame: &mut Frame<'_>,
    area: Rect,
    skin: &Skin,
    hints: &[(String, String)],
) {
    let opts = hint_style(skin);
    let hint_area = Rect {
        x: area.x + 1,
        width: area.width.saturating_sub(2),
        ..area
    };
    ratada::shortcut_hints::render(frame, hint_area, &hint_group(hints), &opts);
}
