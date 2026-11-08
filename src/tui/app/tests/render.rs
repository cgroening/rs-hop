//! Tests of the drawn screen: the theme, the tabs, the column sets, the
//! overlays, the progress bar and the hint band.

use super::*;

#[test]
fn the_active_theme_colours_the_content_cells() {
    // The whole point of dropping `tui::colors`: a re-theme must reach the
    // table cells, not just the frame and the modals.
    let rose = Colors::from_palette(&sample_config().palette());
    let mut config = sample_config();
    config.appearance.theme = "monochrome".to_string();
    let mono = Colors::from_palette(&config.palette());
    assert_ne!(rose.accent, mono.accent, "the themes must differ");

    let app = app_with(config);
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| app.render(frame)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let foregrounds: Vec<_> =
        buffer.content().iter().map(|c| c.style().fg).collect();
    assert!(
        foregrounds.contains(&Some(mono.accent)),
        "the theme accent must appear in the rendered content"
    );
    assert!(
        !foregrounds.contains(&Some(rose.accent)),
        "no cell may keep the compiled-in rose accent"
    );
}

#[test]
fn renders_every_tab_without_panicking() {
    let mut app = sample_app();
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    for tab in ['1', '2', '3'] {
        press(&mut app, KeyCode::Char(tab));
        terminal.draw(|frame| app.render(frame)).unwrap();
    }
}

#[test]
fn c_cycles_the_column_sets_and_only_they_show_the_bar() {
    let mut app = sample_app();
    // Flat view (grouping off) shows the classic table with column headers.
    press(&mut app, KeyCode::Char('.'));
    // Standard looks exactly as it always did: no bar, no totals row.
    let standard = screen(&app, 120, 30);
    assert!(standard.contains("Branch"));
    assert!(!standard.contains("Columns"), "no bar in the standard set");

    press(&mut app, KeyCode::Char('c'));
    let code = screen(&app, 120, 30);
    assert!(code.contains("LOC") && code.contains("Language"));
    assert!(
        !code.contains("Branch"),
        "the standard columns are replaced"
    );
    assert!(code.contains("Columns"), "the bar names the sets");
    assert!(code.contains("projects"), "the totals row is shown");

    press(&mut app, KeyCode::Char('c'));
    let activity = screen(&app, 120, 30);
    assert!(activity.contains("Commits") && activity.contains("Authors"));

    press(&mut app, KeyCode::Char('c'));
    assert!(screen(&app, 120, 30).contains("Branch"), "back to standard");
}

#[test]
fn example_mode_shows_dashes_rather_than_spinning_forever() {
    // No worker ever runs in example mode, so a spinner would never stop.
    // Unicode glyphs, because the ASCII spinner uses `-` itself - which is
    // also the text for a value that will never arrive.
    let mut config = sample_config();
    config.appearance.glyphs = GlyphVariant::Unicode;
    let mut app = app_with(config);
    assert!(app.config.example_mode);
    press(&mut app, KeyCode::Char('c'));
    let code = screen(&app, 120, 30);
    assert!(code.contains('-'), "an uncomputable cell reads as a dash");
    for frame in spinner_frames(GlyphVariant::Unicode) {
        assert!(
            !code.contains(frame),
            "example mode must never spin: found {frame:?}"
        );
    }
}

/// Every distinct frame of the toolkit spinner, gathered by stepping it
/// until it wraps back to where it started.
fn spinner_frames(variant: GlyphVariant) -> Vec<&'static str> {
    let mut spinner = Spinner::new();
    let first = spinner.frame(variant);
    let mut frames = vec![first];
    loop {
        spinner.advance();
        let frame = spinner.frame(variant);
        if frame == first {
            return frames;
        }
        frames.push(frame);
    }
}

#[test]
fn the_bar_and_totals_vanish_on_a_short_terminal() {
    let mut app = sample_app();
    press(&mut app, KeyCode::Char('c'));
    assert!(screen(&app, 120, 30).contains("Columns"));
    // Too short to spare five rows: the list keeps them.
    assert!(!screen(&app, 120, 14).contains("Columns"));
}

#[test]
fn every_overlay_renders_at_a_normal_and_a_cramped_size() {
    // The form, the slug prompt and the path picker size their fields from
    // the box's inner width; on a terminal too small to hold the box that
    // width goes to zero, which must clamp rather than underflow.
    // Each case is the keys that open the overlay; `M` is Files-tab only.
    // The error list is left out: it needs a failing entry, and it is the
    // same `SelectModal` the sort picker already covers.
    let plain = |keys: &str| -> Vec<KeyEvent> {
        keys.chars()
            .map(|key| KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE))
            .collect()
    };
    let openers = [
        (plain("n"), "add form"),
        (plain("e"), "edit form"),
        (plain("d"), "delete confirm"),
        (plain("S"), "slug prompt"),
        (
            vec![KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL)],
            "path picker",
        ),
        (plain("2M"), "manage sections"),
        (plain("t"), "sort picker"),
        (plain("?"), "help"),
    ];
    for (keys, what) in openers {
        let mut app = sample_app();
        for key in &keys {
            app.handle_key(*key);
        }
        assert!(
            !matches!(app.overlay, Overlay::None),
            "{what} did not open on {keys:?}"
        );
        for (width, height) in [(100, 30), (20, 6)] {
            let _ = screen(&app, width, height);
        }
    }
}

#[test]
fn a_long_filter_query_scrolls_inside_the_status_band() {
    // The status band is one row wide; the toolkit's field scrolls the query
    // under the caret and marks the hidden head, rather than overrunning the
    // trailing hint. A hand-drawn caret used to just overflow.
    let mut app = sample_app();
    press(&mut app, KeyCode::Char('f'));
    for ch in "abcdefghijklmnopqrstuvwxyz0123456789".chars() {
        press(&mut app, KeyCode::Char(ch));
    }
    let narrow = screen(&app, 60, 20);
    assert!(narrow.contains('\u{2026}'), "the clipped head is marked");
    assert!(narrow.contains("Esc clear"), "the trailing hint survives");
    assert!(narrow.contains('9'), "the caret's end stays visible");
    // Given room, the whole query fits and nothing is marked as clipped.
    let wide = screen(&app, 120, 20);
    assert!(wide.contains("abcdefghijklmnopqrstuvwxyz0123456789"));
}

#[test]
fn the_progress_label_keeps_its_width_as_the_name_changes() {
    // The gauge centres the label, so a constant label width is what pins
    // the `XX %` column while entry names of different lengths come and go.
    let label_for = |name: &str| {
        ProgressText {
            prefix: " 50 %",
            name,
            ratio: 0.5,
            name_width: 6,
        }
        .label()
    };
    let widths: Vec<usize> = ["", "hop", "mdtask"]
        .iter()
        .map(|n| label_for(n).len())
        .collect();
    assert_eq!(widths, vec![widths[0]; 3]);
    assert!(label_for("hop").starts_with(" 50 % - hop"));
    // Without a name the separator is blanked out rather than dangling.
    assert!(!label_for("").contains('-'));
}

#[test]
fn progress_bar_paints_accent_fill_and_label() {
    // A half-filled bar: the left cells carry the accent background and the
    // centred percentage label is present.
    let config = Config::default();
    let colors = Colors::from_palette(&config.palette());
    let skin = config.skin();
    let mut terminal = Terminal::new(TestBackend::new(40, 2)).unwrap();
    terminal
        .draw(|frame| {
            render_progress(
                frame,
                frame.area(),
                &skin,
                ProgressText {
                    prefix: " 50 %",
                    name: "repo",
                    ratio: 0.5,
                    name_width: 4,
                },
            );
        })
        .unwrap();
    let buf = terminal.backend().buffer().clone();
    // A cell early in the bar (inside the filled region) is accent.
    assert_eq!(buf.cell((2, 0)).unwrap().style().bg, Some(colors.accent));
    // A cell near the right end (past the half fill) is the track colour.
    assert_eq!(
        buf.cell((38, 0)).unwrap().style().bg,
        Some(colors.selection_bg)
    );
    // The bar runs edge to edge: the first cell is filled and the last one
    // carries the track, so neither side is left blank.
    assert_eq!(buf.cell((0, 0)).unwrap().style().bg, Some(colors.accent));
    assert_eq!(
        buf.cell((39, 0)).unwrap().style().bg,
        Some(colors.selection_bg)
    );
    let text: String = buf
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect();
    assert!(text.contains('%') && text.contains("repo"));
}

#[test]
fn hint_band_grows_to_fit_all_hints() {
    // At a narrow width the hints wrap past two rows; the panel's hint band
    // must size to hold them all rather than clipping at a fixed height.
    let app = sample_app();
    let width = 60;
    let mut terminal = Terminal::new(TestBackend::new(width, 40)).unwrap();
    terminal.draw(|frame| app.render(frame)).unwrap();
    let rendered = terminal.backend().buffer().clone();
    let text: String = rendered
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect();
    // The last hint must be present, i.e. nothing was clipped away.
    assert!(text.contains("quit"), "last footer hint was clipped");
}

#[test]
fn filter_and_help_overlay_render() {
    let mut app = sample_app();
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    press(&mut app, KeyCode::Char('F'));
    press(&mut app, KeyCode::Char('h'));
    terminal.draw(|frame| app.render(frame)).unwrap();
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('?'));
    terminal.draw(|frame| app.render(frame)).unwrap();
}

#[test]
fn the_sync_overlays_render_at_a_normal_and_a_cramped_size() {
    let rows = vec![
        "hop     pulled".to_string(),
        "mdtask  failed: timeout".to_string(),
    ];
    let overlays = [
        Overlay::SyncResults(SelectModal::new(" Pull results ", rows, 0)),
        Overlay::SyncConfirm(
            ConfirmModal::new(" Push all ", "Push 2 repos?"),
            SyncOp::Push,
            vec![PathBuf::from("/a"), PathBuf::from("/b")],
        ),
    ];
    for overlay in overlays {
        let mut app = sample_app();
        app.overlay = overlay;
        let normal = screen(&app, 100, 30);
        assert!(
            normal.contains("Pull results") || normal.contains("Push 2 repos?"),
            "the overlay is drawn"
        );
        let _ = screen(&app, 20, 6);
    }
}
