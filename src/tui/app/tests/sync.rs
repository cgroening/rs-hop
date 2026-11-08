//! Tests of pulling, pushing and refreshing: which keys work where, the results
//! list and the spinners of a running job.

use super::*;

#[test]
fn pull_and_push_keys_only_work_on_the_git_tabs() {
    let mut app = sample_app();
    press(&mut app, KeyCode::Char('2'));
    for key in ['p', 'P', 'D', 'U'] {
        press(&mut app, KeyCode::Char(key));
        assert_eq!(status_text(&app), "pull/push: git tabs only", "{key}");
        assert!(matches!(app.overlay, Overlay::None), "{key}");
    }
}

#[test]
fn pull_and_push_never_run_git_in_example_mode() {
    // The sample app is in example mode: the git tab shows curated data, so
    // nothing may be pulled or pushed - not even after confirming.
    let mut app = sample_app();
    for key in ['p', 'P', 'D', 'U'] {
        press(&mut app, KeyCode::Char(key));
        assert!(status_text(&app).starts_with("example mode"), "{key}");
        assert!(!app.is_syncing(), "{key}");
    }
}

#[test]
fn the_results_list_closes_on_esc() {
    let mut app = sample_app();
    app.overlay = Overlay::SyncResults(SelectModal::new(
        " Pull results ",
        vec!["hop  pulled".to_string()],
        0,
    ));
    press(&mut app, KeyCode::Esc);
    assert!(matches!(app.overlay, Overlay::None));
}

/// A sample app on live (non-example) git status, with the start-up refresh
/// already drained so a pull or push may start.
fn live_app() -> App {
    let mut config = sample_config();
    config.example_mode = false;
    let mut app = app_with(config);
    settle(&mut app);
    app
}

/// Drains the background workers until nothing is in flight.
fn settle(app: &mut App) {
    for _ in 0..400 {
        app.drain_status();
        app.drain_sync();
        if !app.is_refreshing() && !app.is_syncing() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the background workers never finished");
}

/// A refresh job over `path` fed by the returned sender, as `refresh_paths`
/// would start it.
fn manual_refresh_job(path: &Path) -> (mpsc::Sender<StatusUpdate>, RefreshJob) {
    let (sender, rx) = mpsc::channel();
    let job = RefreshJob {
        rx,
        fetched: false,
        bar: true,
        remaining: HashSet::from([path.to_path_buf()]),
    };
    (sender, job)
}

#[test]
fn a_row_keeps_spinning_while_an_overlapping_refresh_still_owes_it() {
    let mut app = live_app();
    let path = Path::new("/code/hop");
    let (fast, fast_job) = manual_refresh_job(path);
    let (slow, slow_job) = manual_refresh_job(path);
    app.status_jobs.push(fast_job);
    app.status_jobs.push(slow_job);
    app.rebuild_in_flight();

    let done = |sender: &mpsc::Sender<StatusUpdate>| {
        sender
            .send(StatusUpdate::Done {
                path: path.to_path_buf(),
                info: GitInfo::default(),
            })
            .expect("the receiver lives in the app");
    };
    done(&fast);
    drop(fast);
    app.drain_status();
    assert!(app.is_refreshing(), "the slow job is still running");
    assert!(
        app.refreshing.contains(path),
        "the slow job still owes the row, so it keeps spinning"
    );

    done(&slow);
    drop(slow);
    app.drain_status();
    assert!(!app.is_refreshing());
    assert!(app.refreshing.is_empty(), "nothing owes the row any more");
}

#[test]
fn pulling_the_cursor_repo_spins_its_row_without_the_progress_bar() {
    let mut app = live_app();
    press(&mut app, KeyCode::Char('p'));
    assert!(app.is_syncing());
    assert_eq!(app.refreshing.len(), 1, "only the cursor row spins");
    assert!(app.loading.is_none(), "no progress bar for a target run");

    settle(&mut app);
    assert!(
        app.refreshing.is_empty(),
        "the spinner stops when it is done"
    );
    assert!(app.loading.is_none());
}

#[test]
fn pulling_the_whole_tab_shows_the_bar_and_spins_every_row() {
    let mut app = live_app();
    press(&mut app, KeyCode::Char('D'));
    assert!(matches!(app.overlay, Overlay::SyncConfirm(..)));
    press(&mut app, KeyCode::Char('y'));
    assert!(app.is_syncing());
    assert_eq!(app.loading, Some((0, 2)), "both active git repos");
    assert!(app.refreshing.contains(Path::new("/code/hop")));
    assert!(app.refreshing.contains(Path::new("/code/gone")));

    settle(&mut app);
    assert!(app.loading.is_none());
    assert!(app.refreshing.is_empty());
}

#[test]
fn a_running_target_pull_blocks_another_sync_and_a_backup() {
    let mut app = live_app();
    press(&mut app, KeyCode::Char('p'));
    for key in ['P', 'z'] {
        press(&mut app, KeyCode::Char(key));
        assert!(status_text(&app).starts_with("busy:"), "{key}");
    }
    settle(&mut app);
}
