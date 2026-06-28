//! Builds a ZIP archive of a repository for the `z`/`Z` backup.
//!
//! [`collect_files`] does a hand-rolled `std::fs` walk (no extra walk
//! dependency), pruning whole subtrees whose directory name starts with an
//! excluded build-artefact prefix (so `target` also covers `target.nosync`),
//! but keeping hidden directories such as `.git` so the archive stays a
//! restorable repository. [`write_zip`]
//! deflates the gathered files into the destination, preserving the top-level
//! folder name and reporting per-file progress.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

/// The files to archive under `root`, with any excluded directory subtree
/// pruned. Hidden directories are kept (so `.git` is included); symlinks are
/// not followed. Returned in directory order.
pub fn collect_files(root: &Path, exclude_dirs: &[String]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    walk(root, exclude_dirs, &mut files);
    files
}

/// Recurses into `dir`, collecting files and pruning excluded subtrees.
fn walk(dir: &Path, exclude_dirs: &[String], files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        // Never follow symlinks (avoids loops and escaping the tree).
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            let name = entry.file_name();
            if is_excluded(&name.to_string_lossy(), exclude_dirs) {
                continue;
            }
            walk(&entry.path(), exclude_dirs, files);
        } else if file_type.is_file() {
            files.push(entry.path());
        }
    }
}

/// Whether a directory `name` is excluded: its name starts with any entry in
/// `exclude_dirs` (so e.g. `target` also prunes `target.nosync` and
/// `target-old`). Empty entries are ignored so a stray `""` cannot prune
/// everything.
fn is_excluded(name: &str, exclude_dirs: &[String]) -> bool {
    exclude_dirs
        .iter()
        .any(|dir| !dir.is_empty() && name.starts_with(dir.as_str()))
}

/// Writes `files` (gathered from `root`) as a deflated ZIP at `dest`,
/// overwriting any existing file. Archive entry names keep `root`'s own folder
/// as the top level (e.g. `hop/src/main.rs`). `on_progress` is called with the
/// running count after each file is added.
///
/// # Errors
/// Returns an I/O error if the destination cannot be written or a source file
/// cannot be read.
pub fn write_zip(
    root: &Path,
    files: &[PathBuf],
    dest: &Path,
    mut on_progress: impl FnMut(usize),
) -> io::Result<()> {
    // Entry names are relative to the parent, so the repo's own folder name is
    // the archive's top-level directory.
    let base = root.parent().unwrap_or(root);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated);
    let file = File::create(dest)?;
    let mut zip = zip::ZipWriter::new(file);
    for (index, path) in files.iter().enumerate() {
        let name = archive_name(path, base);
        zip.start_file(name, options)?;
        let mut source = File::open(path)?;
        io::copy(&mut source, &mut zip)?;
        on_progress(index + 1);
    }
    zip.finish()?;
    Ok(())
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
}
