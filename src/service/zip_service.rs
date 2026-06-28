//! Builds repo ZIP backups in the background for the `z`/`Z` keys.
//!
//! [`spawn_zip`] runs one worker thread that first gathers each job's file list
//! (to know the total for the progress bar), then for each repo compares the
//! current content fingerprint (name + size + CRC32) against the existing
//! archive: unchanged repos are left untouched (no needless rewrite / iCloud
//! re-upload), changed ones are deflated to disk. Progress is streamed as a
//! [`ZipUpdate`] so the TUI bar advances while the list stays responsive.
//! Mirrors `status_service::spawn_refresh`.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use crate::util::archive::{self, Manifest};

/// One archive to build: zip `src` into `dest` (overwriting).
pub struct ZipJob {
    /// The repository directory to archive.
    pub src: PathBuf,
    /// The destination `.zip` path.
    pub dest: PathBuf,
}

/// Progress of the background ZIP run, streamed file by file.
pub struct ZipUpdate {
    /// Files examined so far across all jobs.
    pub done: usize,
    /// Total files to examine across all jobs (known once counted).
    pub total: usize,
    /// Whether the run has finished (the final message).
    pub finished: bool,
    /// Archives (re)written because their content changed (when `finished`).
    pub archives: usize,
    /// Archives left untouched because their content was unchanged.
    pub unchanged: usize,
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
            unchanged: 0,
            errors: 0,
        }
    }

    /// The final message summarising the run.
    fn finished(
        total: usize,
        archives: usize,
        unchanged: usize,
        errors: usize,
    ) -> Self {
        ZipUpdate {
            done: total,
            total,
            finished: true,
            archives,
            unchanged,
            errors,
        }
    }
}

/// Spawns a worker that backs up each job (skipping unchanged ones) and streams
/// progress. The receiver yields progress updates, then one `finished` message,
/// then closes. Dropping the receiver lets the worker run to completion
/// harmlessly.
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
        let mut unchanged = 0;
        let mut errors = 0;
        for (job, files) in &planned {
            let base = done;
            // Fingerprint the working tree (this reads every file once),
            // advancing the bar as we go.
            let source = archive::source_manifest(&job.src, files, |in_job| {
                let _ = sender.send(ZipUpdate::progress(base + in_job, total));
            });
            done = base + files.len();

            if is_unchanged(&source, &job.dest) {
                unchanged += 1;
                let _ = sender.send(ZipUpdate::progress(done, total));
                continue;
            }
            // Content differs (or could not be fingerprinted): (re)write it.
            match archive::write_zip(&job.src, files, &job.dest, |_| {}) {
                Ok(()) => archives += 1,
                Err(error) => {
                    errors += 1;
                    log::error!(
                        "could not write {}: {error}",
                        job.dest.display()
                    );
                }
            }
            let _ = sender.send(ZipUpdate::progress(done, total));
        }
        let _ = sender
            .send(ZipUpdate::finished(total, archives, unchanged, errors));
    });
    receiver
}

/// Whether `dest` already holds the same content as `source` describes, so the
/// archive can be left untouched. A read error on either side counts as changed
/// (rebuild to be safe).
fn is_unchanged(source: &std::io::Result<Manifest>, dest: &Path) -> bool {
    let Ok(source) = source else {
        return false;
    };
    if !dest.exists() {
        return false;
    }
    archive::zip_manifest(dest).is_ok_and(|existing| &existing == source)
}
