//! Builds a ZIP archive of a repository for the `z`/`Z` backup.
//!
//! [`collect_files`] does a hand-rolled `std::fs` walk (no extra walk
//! dependency), pruning whole subtrees whose directory name starts with an
//! excluded build-artefact prefix (so `target` also covers `target.nosync`),
//! but keeping hidden directories such as `.git` so the archive stays a
//! restorable repository. [`write_zip`]
//! deflates the gathered files into the destination, preserving the top-level
//! folder name and reporting per-file progress. [`source_manifest`] and
//! [`zip_manifest`] produce a content fingerprint (name + size + CRC32) so a
//! backup is only rewritten when the content actually changed.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

/// A content fingerprint of an archive: entry name → (uncompressed size, CRC32).
/// Equal manifests mean equal content, regardless of file timestamps or order.
pub type Manifest = BTreeMap<String, (u64, u32)>;

/// Read-buffer size for streaming a file through the CRC32 hasher.
const CRC_CHUNK: usize = 64 * 1024;

/// The files to archive under `root`, with any excluded directory subtree
/// pruned. Hidden directories are kept (so `.git` is included). Symlinks are
/// recorded (stored as links by [`write_zip`]) but never followed. Returned in
/// directory order.
pub fn collect_files(root: &Path, exclude_dirs: &[String]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    walk(root, exclude_dirs, &mut files);
    files
}

/// Recurses into `dir`, collecting files and symlinks and pruning excluded
/// subtrees.
fn walk(dir: &Path, exclude_dirs: &[String], files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        // `file_type()` does not follow symlinks, so links are reported as such.
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name();
        let excluded = is_excluded(&name.to_string_lossy(), exclude_dirs);
        if file_type.is_symlink() {
            // Store the link itself (never recurse through it: avoids loops).
            if !excluded {
                files.push(entry.path());
            }
        } else if file_type.is_dir() {
            if !excluded {
                walk(&entry.path(), exclude_dirs, files);
            }
        } else if file_type.is_file() {
            files.push(entry.path());
        }
    }
}

/// Whether a directory `name` is excluded: its name starts with any entry in
/// `exclude_dirs` (so e.g. `target` also prunes `target.nosync` and
/// `target-old`). Empty entries are ignored so a stray `""` cannot prune
/// everything.
///
/// Shared with `util::diskusage`, so both agree on what counts as a build
/// directory.
pub fn is_excluded(name: &str, exclude_dirs: &[String]) -> bool {
    exclude_dirs
        .iter()
        .any(|dir| !dir.is_empty() && name.starts_with(dir.as_str()))
}

/// Writes `files` (gathered from `root`) as a deflated ZIP at `dest`. Archive
/// entry names keep `root`'s own folder as the top level (e.g. `hop/src/main.rs`);
/// Unix file permissions and symlinks are preserved. `on_progress` is called
/// with the running count after each entry.
///
/// The archive is built into a temporary file in the same directory and then
/// atomically renamed over `dest`, so an existing good backup survives if the
/// build fails partway.
///
/// # Errors
///
/// Returns an I/O error if the destination cannot be written or a source file
/// cannot be read.
pub fn write_zip(
    root: &Path,
    files: &[PathBuf],
    dest: &Path,
    on_progress: impl FnMut(usize),
) -> io::Result<()> {
    let tmp = temp_path(dest);
    match build_zip(root, files, &tmp, on_progress) {
        Ok(()) => replace_atomically(&tmp, dest),
        Err(error) => {
            // Leave the previous archive untouched; drop the partial temp.
            let _ = std::fs::remove_file(&tmp);
            Err(error)
        }
    }
}

/// Builds the ZIP at `tmp` (a regular write, no rename).
fn build_zip(
    root: &Path,
    files: &[PathBuf],
    tmp: &Path,
    mut on_progress: impl FnMut(usize),
) -> io::Result<()> {
    // Entry names are relative to the parent, so the repo's own folder name is
    // the archive's top-level directory.
    let base = root.parent().unwrap_or(root);
    let file = File::create(tmp)?;
    let mut zip = zip::ZipWriter::new(file);
    for (index, path) in files.iter().enumerate() {
        let name = archive_name(path, base);
        let meta = std::fs::symlink_metadata(path)?;
        if meta.file_type().is_symlink() {
            let target = std::fs::read_link(path)?;
            // Pass the raw target string (no path normalisation) so it matches
            // the bytes `source_manifest` fingerprints.
            zip.add_symlink(
                name,
                target.to_string_lossy(),
                SimpleFileOptions::default(),
            )?;
        } else {
            zip.start_file(name, file_options(&meta))?;
            let mut source = File::open(path)?;
            io::copy(&mut source, &mut zip)?;
        }
        on_progress(index + 1);
    }
    zip.finish()?;
    Ok(())
}

/// The deflate options for a regular file, carrying its Unix permission bits so
/// the executable bit survives a restore.
fn file_options(meta: &std::fs::Metadata) -> SimpleFileOptions {
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated);
    with_permissions(options, meta)
}

/// Adds the file's Unix permission bits to `options`.
#[cfg(unix)]
fn with_permissions(
    options: SimpleFileOptions,
    meta: &std::fs::Metadata,
) -> SimpleFileOptions {
    use std::os::unix::fs::PermissionsExt;
    options.unix_permissions(meta.permissions().mode())
}

/// No-op on platforms without Unix permission bits.
#[cfg(not(unix))]
fn with_permissions(
    options: SimpleFileOptions,
    _meta: &std::fs::Metadata,
) -> SimpleFileOptions {
    options
}

/// The sibling temp path used while building an archive (same directory, so the
/// final rename is atomic).
fn temp_path(dest: &Path) -> PathBuf {
    let mut name = dest.as_os_str().to_os_string();
    name.push(".part");
    PathBuf::from(name)
}

/// Renames `tmp` over `dest`. On Unix this atomically replaces an existing file;
/// on platforms where rename fails when the target exists, it removes first.
fn replace_atomically(tmp: &Path, dest: &Path) -> io::Result<()> {
    match std::fs::rename(tmp, dest) {
        Ok(()) => Ok(()),
        Err(_) if dest.exists() => {
            std::fs::remove_file(dest)?;
            std::fs::rename(tmp, dest)
        }
        Err(error) => Err(error),
    }
}

/// The forward-slash archive entry name for `path` relative to `base`.
fn archive_name(path: &Path, base: &Path) -> String {
    let relative = path.strip_prefix(base).unwrap_or(path);
    relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// The content manifest of `files` under `root` (the same naming and CRC32 that
/// [`write_zip`] would store), calling `on_progress` with the running count
/// after each file. Lets a caller decide whether an existing archive is stale
/// without building it.
///
/// # Errors
///
/// Returns an I/O error if a source file cannot be read.
pub fn source_manifest(
    root: &Path,
    files: &[PathBuf],
    mut on_progress: impl FnMut(usize),
) -> io::Result<Manifest> {
    let base = root.parent().unwrap_or(root);
    let mut manifest = Manifest::new();
    for (index, path) in files.iter().enumerate() {
        let fingerprint = fingerprint(path)?;
        manifest.insert(archive_name(path, base), fingerprint);
        on_progress(index + 1);
    }
    Ok(manifest)
}

/// The (size, CRC32) of `path` as it would be stored. For a symlink this is the
/// link target string (matching how [`write_zip`] stores it); for a regular
/// file it is the file contents.
fn fingerprint(path: &Path) -> io::Result<(u64, u32)> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(path)?;
        let bytes = target.to_string_lossy();
        let bytes = bytes.as_bytes();
        return Ok((bytes.len() as u64, crc32fast::hash(bytes)));
    }
    hash_file(path)
}

/// The (size, CRC32) of `path`'s contents, streamed in chunks so large files are
/// not held in memory. The CRC32 is the IEEE variant the ZIP format stores.
fn hash_file(path: &Path) -> io::Result<(u64, u32)> {
    let mut file = File::open(path)?;
    let mut hasher = crc32fast::Hasher::new();
    let mut buffer = vec![0u8; CRC_CHUNK];
    let mut size: u64 = 0;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size += read as u64;
    }
    Ok((size, hasher.finalize()))
}

/// A deterministic 64-bit digest of a manifest (FNV-1a over the sorted
/// name/size/CRC tuples). Lets a small cache record a whole archive's content
/// as one number. Stable across runs (unlike `DefaultHasher`).
pub fn manifest_hash(manifest: &Manifest) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(PRIME);
        }
    };
    for (name, (size, crc)) in manifest {
        feed(name.as_bytes());
        feed(&size.to_le_bytes());
        feed(&crc.to_le_bytes());
    }
    hash
}

/// The content manifest of an existing ZIP at `path`, read from its central
/// directory (no decompression). Directory entries are skipped.
///
/// # Errors
///
/// Returns an I/O error if the file cannot be read or is not a valid ZIP.
pub fn zip_manifest(path: &Path) -> io::Result<Manifest> {
    let file = File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut manifest = Manifest::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if entry.is_dir() {
            continue;
        }
        manifest
            .insert(entry.name().to_string(), (entry.size(), entry.crc32()));
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// A unique temp directory for one test, removed on drop.
    struct TempTree {
        root: PathBuf,
    }

    impl TempTree {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir()
                .join(format!("hop-archive-test-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            TempTree { root }
        }

        fn write(&self, rel: &str, contents: &str) {
            let path = self.root.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn excludes() -> Vec<String> {
        ["target", "node_modules"]
            .iter()
            .map(|s| (*s).to_string())
            .collect()
    }

    fn relative_names(root: &Path, files: &[PathBuf]) -> Vec<String> {
        let mut names: Vec<String> = files
            .iter()
            .map(|f| {
                f.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        names.sort();
        names
    }

    #[test]
    fn collect_skips_build_dirs_and_keeps_git() {
        let tree = TempTree::new("skip");
        tree.write("src/main.rs", "fn main() {}");
        tree.write(".git/config", "[core]");
        tree.write("target/debug/app", "binary");
        tree.write("node_modules/dep/index.js", "x");

        let files = collect_files(&tree.root, &excludes());
        let names = relative_names(&tree.root, &files);
        assert_eq!(names, vec![".git/config", "src/main.rs"]);
    }

    #[test]
    fn collect_prunes_nested_and_prefix_variants() {
        let tree = TempTree::new("nested");
        tree.write("a/keep.txt", "k");
        tree.write("a/b/target/junk", "j");
        // Any directory starting with an excluded name is pruned (not just an
        // exact match or a `.nosync` sibling).
        tree.write("target.nosync/debug/app", "b");
        tree.write("target-old/stale", "o");
        tree.write("a/node_modules.nosync/dep/x.js", "n");
        // A directory that does not start with an excluded name is kept.
        tree.write("docs/notes.md", "d");

        let files = collect_files(&tree.root, &excludes());
        let names = relative_names(&tree.root, &files);
        assert_eq!(names, vec!["a/keep.txt", "docs/notes.md"]);
    }

    #[test]
    fn write_zip_reports_progress_and_writes_dest() {
        let tree = TempTree::new("write");
        tree.write("src/main.rs", "fn main() {}");
        tree.write("README.md", "# hop");

        let files = collect_files(&tree.root, &excludes());
        let dest = tree.root.join("out.zip");
        let mut progress = Vec::new();
        write_zip(&tree.root, &files, &dest, |done| progress.push(done))
            .unwrap();

        assert!(dest.exists());
        assert_eq!(progress, vec![1, 2]);

        // The archive is readable and entries keep the repo folder as the top
        // level, with forward slashes.
        let archive = fs::File::open(&dest).unwrap();
        let mut zip = zip::ZipArchive::new(archive).unwrap();
        let mut names: Vec<String> = (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().to_string())
            .collect();
        names.sort();
        let top = tree.root.file_name().unwrap().to_string_lossy();
        assert_eq!(
            names,
            vec![format!("{top}/README.md"), format!("{top}/src/main.rs")]
        );
    }

    #[test]
    fn source_manifest_matches_built_zip() {
        let tree = TempTree::new("manifest");
        tree.write("src/main.rs", "fn main() {}");
        tree.write("README.md", "# hop");
        let files = collect_files(&tree.root, &excludes());

        let dest = tree.root.join("out.zip");
        write_zip(&tree.root, &files, &dest, |_| {}).unwrap();

        // The pre-build source fingerprint equals what the written zip stores,
        // so the change-detection precheck is exact.
        let source = source_manifest(&tree.root, &files, |_| {}).unwrap();
        let stored = zip_manifest(&dest).unwrap();
        assert_eq!(source, stored);
    }

    #[test]
    fn source_manifest_changes_with_content() {
        let tree = TempTree::new("manifest-change");
        tree.write("README.md", "# hop");
        let files = collect_files(&tree.root, &excludes());
        let before = source_manifest(&tree.root, &files, |_| {}).unwrap();

        tree.write("README.md", "# hop, edited");
        let after = source_manifest(&tree.root, &files, |_| {}).unwrap();
        assert_ne!(before, after);
    }

    #[test]
    fn write_zip_leaves_no_temp_file() {
        let tree = TempTree::new("atomic");
        tree.write("README.md", "# hop");
        let files = collect_files(&tree.root, &excludes());
        let dest = tree.root.join("out.zip");
        write_zip(&tree.root, &files, &dest, |_| {}).unwrap();

        assert!(dest.exists());
        assert!(!temp_path(&dest).exists());
    }

    #[test]
    fn manifest_hash_is_deterministic_and_content_sensitive() {
        let tree = TempTree::new("hash");
        tree.write("README.md", "# hop");
        let files = collect_files(&tree.root, &excludes());
        let first = source_manifest(&tree.root, &files, |_| {}).unwrap();
        let again = source_manifest(&tree.root, &files, |_| {}).unwrap();
        assert_eq!(manifest_hash(&first), manifest_hash(&again));

        tree.write("README.md", "# hop, edited");
        let changed = source_manifest(&tree.root, &files, |_| {}).unwrap();
        assert_ne!(manifest_hash(&first), manifest_hash(&changed));
    }

    #[cfg(unix)]
    #[test]
    fn preserves_unix_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let tree = TempTree::new("perms");
        tree.write("run.sh", "#!/bin/sh\n");
        let script = tree.root.join("run.sh");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
            .unwrap();

        let files = collect_files(&tree.root, &excludes());
        let dest = tree.root.join("out.zip");
        write_zip(&tree.root, &files, &dest, |_| {}).unwrap();

        let mut zip =
            zip::ZipArchive::new(fs::File::open(&dest).unwrap()).unwrap();
        let entry = zip.by_index(0).unwrap();
        assert_eq!(entry.unix_mode().unwrap() & 0o777, 0o755);
    }

    #[cfg(unix)]
    #[test]
    fn stores_and_fingerprints_symlinks() {
        use std::io::Read;

        let tree = TempTree::new("symlink");
        tree.write("real.txt", "hello");
        std::os::unix::fs::symlink("real.txt", tree.root.join("link.txt"))
            .unwrap();

        let files = collect_files(&tree.root, &excludes());
        let names = relative_names(&tree.root, &files);
        assert!(names.contains(&"link.txt".to_string()));

        let dest = tree.root.join("out.zip");
        write_zip(&tree.root, &files, &dest, |_| {}).unwrap();

        // The link is stored as a symlink pointing at the raw target.
        let mut zip =
            zip::ZipArchive::new(fs::File::open(&dest).unwrap()).unwrap();
        let top = tree.root.file_name().unwrap().to_string_lossy();
        let mut link = zip.by_name(&format!("{top}/link.txt")).unwrap();
        assert!(link.is_symlink());
        let mut target = String::new();
        link.read_to_string(&mut target).unwrap();
        assert_eq!(target, "real.txt");
        drop(link);

        // The precheck fingerprint still matches the stored archive.
        let source = source_manifest(&tree.root, &files, |_| {}).unwrap();
        assert_eq!(source, zip_manifest(&dest).unwrap());
    }
}
