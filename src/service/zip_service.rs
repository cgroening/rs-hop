//! Builds repo ZIP backups in the background for the `z`/`Z` keys.
//!
//! [`spawn_zip`] runs one worker thread that first gathers each job's file list
//! (to know the total for the progress bar), then deflates each archive,
//! streaming a [`ZipUpdate`] as files are added so the TUI bar advances while
//! the list stays responsive. Mirrors `status_service::spawn_refresh`.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use crate::util::archive;

/// One archive to build: zip `src` into `dest` (overwriting).
pub struct ZipJob {
    /// The repository directory to archive.
    pub src: PathBuf,
    /// The destination `.zip` path.
    pub dest: PathBuf,
}

/// Progress of the background ZIP run, streamed file by file.
pub struct ZipUpdate {
    /// Files added so far across all jobs.
    pub done: usize,
    /// Total files to add across all jobs (known once counted).
    pub total: usize,
    /// Whether the run has finished (the final message).
    pub finished: bool,
    /// Archives written successfully (only meaningful when `finished`).
    pub archives: usize,
    /// Archives that failed to write (only meaningful when `finished`).
    pub errors: usize,
}

impl ZipUpdate {
    /// A progress message at `done`/`total`.
    fn progress(done: usize, total: usize) -> Self {
        ZipUpdate {
            done,
            total,
            finished: false,
            archives: 0,
            errors: 0,
        }
    }

    /// The final message summarising the run.
    fn finished(total: usize, archives: usize, errors: usize) -> Self {
        ZipUpdate {
            done: total,
            total,
            finished: true,
            archives,
            errors,
        }
    }
}

/// Spawns a worker that builds each job's archive and streams progress. The
/// receiver yields progress updates, then one `finished` message, then closes.
/// Dropping the receiver lets the worker run to completion harmlessly.
pub fn spawn_zip(
    jobs: Vec<ZipJob>,
    exclude_dirs: Vec<String>,
) -> Receiver<ZipUpdate> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        // Gather every job's files first, so the total is known up front.
        let planned: Vec<(ZipJob, Vec<PathBuf>)> = jobs
            .into_iter()
            .map(|job| {
                let files = archive::collect_files(&job.src, &exclude_dirs);
                (job, files)
            })
            .collect();
        let total: usize = planned.iter().map(|(_, files)| files.len()).sum();
        if sender.send(ZipUpdate::progress(0, total)).is_err() {
            return;
        }

        let mut done = 0;
        let mut archives = 0;
        let mut errors = 0;
        for (job, files) in &planned {
            let base = done;
            let result =
                archive::write_zip(&job.src, files, &job.dest, |in_job| {
                    // Best-effort progress; a dropped receiver ends the run.
                    let _ =
                        sender.send(ZipUpdate::progress(base + in_job, total));
                });
            match result {
                Ok(()) => archives += 1,
                Err(error) => {
                    errors += 1;
                    log::error!(
                        "could not write {}: {error}",
                        job.dest.display()
                    );
                }
            }
            done = base + files.len();
        }
        let _ = sender.send(ZipUpdate::finished(total, archives, errors));
    });
    receiver
}
