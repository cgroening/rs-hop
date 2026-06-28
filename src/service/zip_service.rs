//! Builds repo ZIP backups in the background for the `z`/`Z` keys.
//!
//! [`spawn_zip`] runs one worker thread that first gathers each job's file list
//! (to know the total for the progress bar), then for each repo compares the
//! current content fingerprint (name + size + CRC32) against the existing
//! archive: unchanged repos are left untouched (no needless rewrite / iCloud
//! re-upload), changed ones are deflated to disk. A local fingerprint cache
//! (`storage::zip_cache`) lets the unchanged-check avoid reading the existing
//! archive at all when the file on disk is still the one we wrote. Progress is
//! streamed as a [`ZipUpdate`] so the TUI bar advances while the list stays
//! responsive. Mirrors `status_service::spawn_refresh`.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::UNIX_EPOCH;

use crate::storage::zip_cache::{self, ZipCache, ZipFingerprint};
use crate::util::archive::{self, Manifest};

/// One archive to build: zip `src` into `dest` (overwriting).
pub struct ZipJob {
    /// The repository directory to archive.
    pub src: PathBuf,
    /// The destination `.zip` path.
    pub dest: PathBuf,
    /// The entry's display name, shown in the progress bar while it is zipped.
    pub name: String,
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
    /// The name of the entry currently being processed, if any.
    pub label: Option<String>,
}

impl ZipUpdate {
    /// A progress message at `done`/`total`, naming the current entry.
    fn progress(done: usize, total: usize, label: Option<String>) -> Self {
        ZipUpdate {
            done,
            total,
            finished: false,
            archives: 0,
            unchanged: 0,
            errors: 0,
            label,
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
            label: None,
        }
    }
}

/// Spawns a worker that backs up each job (skipping unchanged ones) and streams
/// progress. `cache_path` holds the fingerprint cache that lets unchanged repos
/// be detected without reading the existing archive. The receiver yields
/// progress updates, then one `finished` message, then closes. Dropping the
/// receiver lets the worker run to completion harmlessly.
pub fn spawn_zip(
    jobs: Vec<ZipJob>,
    exclude_dirs: Vec<String>,
    cache_path: PathBuf,
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
        if sender.send(ZipUpdate::progress(0, total, None)).is_err() {
            return;
        }

        let mut cache = zip_cache::load(&cache_path);
        let mut done = 0;
        let mut archives = 0;
        let mut unchanged = 0;
        let mut errors = 0;
        for (job, files) in &planned {
            let base = done;
            let name = job.name.clone();
            // Fingerprint the working tree (this reads every file once),
            // advancing the bar as we go.
            let source = archive::source_manifest(&job.src, files, |in_job| {
                let _ = sender.send(ZipUpdate::progress(
                    base + in_job,
                    total,
                    Some(name.clone()),
                ));
            });
            done = base + files.len();

            match decide(&source, &job.dest, &cache) {
                Decision::Skip(fp) => {
                    unchanged += 1;
                    cache.insert(job.dest.clone(), fp);
                }
                Decision::Write(src_hash) => {
                    write_job(job, files, src_hash, &mut cache)
                        .map_or_else(|| errors += 1, |()| archives += 1);
                }
            }
            let _ = sender.send(ZipUpdate::progress(done, total, Some(name)));
        }
        let _ = zip_cache::save(&cache_path, &cache);
        let _ = sender
            .send(ZipUpdate::finished(total, archives, unchanged, errors));
    });
    receiver
}

/// Writes one changed archive and records its fresh fingerprint. Returns `None`
/// on a write error (logged). `src_hash` is the source content hash, if known.
fn write_job(
    job: &ZipJob,
    files: &[PathBuf],
    src_hash: Option<u64>,
    cache: &mut ZipCache,
) -> Option<()> {
    match archive::write_zip(&job.src, files, &job.dest, |_| {}) {
        Ok(()) => {
            match (src_hash, dest_stat(&job.dest)) {
                (Some(hash), Some((mtime, size))) => {
                    cache.insert(
                        job.dest.clone(),
                        ZipFingerprint { hash, mtime, size },
                    );
                }
                // Without a fresh fingerprint the cache entry would be stale.
                _ => {
                    cache.remove(&job.dest);
                }
            }
            Some(())
        }
        Err(error) => {
            cache.remove(&job.dest);
            log::error!("could not write {}: {error}", job.dest.display());
            None
        }
    }
}

/// Whether to skip (with the fingerprint to cache) or rewrite an archive.
enum Decision {
    /// Content is unchanged; keep `dest` and ensure this fingerprint is cached.
    Skip(ZipFingerprint),
    /// Content changed (or could not be compared); rewrite. Carries the source
    /// hash to record afterwards, if it could be computed.
    Write(Option<u64>),
}

/// Decides whether `dest` is still current for the fingerprinted `source`. Uses
/// the cache (validated by the archive's mtime+size, a cheap stat that does not
/// download an iCloud-evicted file) and only falls back to reading the archive
/// when the cache is missing or the file changed externally.
fn decide(
    source: &std::io::Result<Manifest>,
    dest: &Path,
    cache: &ZipCache,
) -> Decision {
    let Ok(source) = source else {
        return Decision::Write(None);
    };
    let src_hash = archive::manifest_hash(source);
    let Some((mtime, size)) = dest_stat(dest) else {
        return Decision::Write(Some(src_hash));
    };
    // Trust the cached hash only when the file is exactly the one we wrote.
    if let Some(fp) = cache.get(dest)
        && fp.mtime == mtime
        && fp.size == size
    {
        return decide_with_hash(fp.hash, src_hash, mtime, size);
    }
    // No / stale cache entry: read the archive (may materialise it) to be sure.
    match archive::zip_manifest(dest) {
        Ok(existing) => decide_with_hash(
            archive::manifest_hash(&existing),
            src_hash,
            mtime,
            size,
        ),
        Err(_) => Decision::Write(Some(src_hash)),
    }
}

/// Skips when the existing hash matches the source, else rewrites.
fn decide_with_hash(
    existing_hash: u64,
    src_hash: u64,
    mtime: i64,
    size: u64,
) -> Decision {
    if existing_hash == src_hash {
        Decision::Skip(ZipFingerprint {
            hash: src_hash,
            mtime,
            size,
        })
    } else {
        Decision::Write(Some(src_hash))
    }
}

/// The (mtime in Unix seconds, size) of `dest`, or `None` if it does not exist.
/// A plain `stat`, so it does not download an iCloud-evicted archive.
fn dest_stat(dest: &Path) -> Option<(i64, u64)> {
    let meta = std::fs::metadata(dest).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64);
    Some((mtime, meta.len()))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// Runs one backup over `src`→`dest` and returns the final summary.
    fn run(src: &Path, dest: &Path, cache: &Path) -> ZipUpdate {
        let jobs = vec![ZipJob {
            src: src.to_path_buf(),
            dest: dest.to_path_buf(),
            name: "repo".to_string(),
        }];
        let rx = spawn_zip(jobs, Vec::new(), cache.to_path_buf());
        let mut last = None;
        while let Ok(update) = rx.recv() {
            if update.finished {
                last = Some(update);
            }
        }
        last.expect("a finished update")
    }

    #[test]
    fn writes_then_skips_unchanged_then_rewrites_changed() {
        let dir = std::env::temp_dir()
            .join(format!("hop-zipsvc-test-{}", std::process::id()));
        let src = dir.join("repo");
        let backups = dir.join("backups");
        fs::create_dir_all(src.join("src")).unwrap();
        fs::create_dir_all(&backups).unwrap();
        fs::write(src.join("src/main.rs"), "fn main() {}").unwrap();
        let dest = backups.join("repo.zip");
        let cache = dir.join("zip-manifests.toml");

        // First run writes the archive.
        let first = run(&src, &dest, &cache);
        assert_eq!((first.archives, first.unchanged, first.errors), (1, 0, 0));
        assert!(dest.exists());
        assert!(cache.exists());

        // Second run, nothing changed: skipped, archive untouched.
        let before = fs::metadata(&dest).unwrap().modified().unwrap();
        let second = run(&src, &dest, &cache);
        assert_eq!(
            (second.archives, second.unchanged, second.errors),
            (0, 1, 0)
        );
        let after = fs::metadata(&dest).unwrap().modified().unwrap();
        assert_eq!(before, after, "unchanged archive must not be rewritten");

        // Change a file: rewritten.
        fs::write(src.join("src/main.rs"), "fn main() { /* edit */ }").unwrap();
        let third = run(&src, &dest, &cache);
        assert_eq!((third.archives, third.unchanged, third.errors), (1, 0, 0));

        let _ = fs::remove_dir_all(&dir);
    }
}
