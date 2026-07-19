//! TOML-backed cache of ZIP-backup fingerprints, keyed by archive path.
//!
//! Lets the backup decide whether a repo is unchanged without reading the
//! existing archive (which, on iCloud "optimize storage", would force a full
//! re-download). Each entry records the content hash plus the archive's mtime
//! and size at write time, so the cached hash is only trusted when the file on
//! disk is still exactly the one we wrote. Loading is best-effort: a missing or
//! corrupt cache yields an empty map.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::domain::error::{Error, Result};
use crate::util::fs::write_atomic;

/// The recorded fingerprint of one written archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZipFingerprint {
    /// Hash of the source content manifest at write time.
    pub hash: u64,
    /// The archive's modified time (Unix seconds) when written.
    pub mtime: i64,
    /// The archive's size in bytes when written.
    pub size: u64,
}

/// Fingerprints keyed by destination archive path.
pub type ZipCache = HashMap<PathBuf, ZipFingerprint>;

/// The on-disk cache document.
#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheDoc {
    #[serde(default)]
    entries: Vec<CacheEntry>,
}

/// One archive's fingerprint. The hash is stored as hex since TOML integers are
/// signed 64-bit and a `u64` hash can exceed that range.
#[derive(Debug, Serialize, Deserialize)]
struct CacheEntry {
    path: String,
    hash: String,
    mtime: i64,
    size: u64,
}

/// Loads the cache from `path`, returning an empty map when it is missing or
/// cannot be parsed.
pub fn load(path: &Path) -> ZipCache {
    let Ok(text) = fs::read_to_string(path) else {
        return ZipCache::new();
    };
    let Ok(doc) = toml::from_str::<CacheDoc>(&text) else {
        return ZipCache::new();
    };
    doc.entries
        .into_iter()
        .filter_map(|entry| {
            let hash = u64::from_str_radix(&entry.hash, 16).ok()?;
            Some((
                PathBuf::from(entry.path),
                ZipFingerprint {
                    hash,
                    mtime: entry.mtime,
                    size: entry.size,
                },
            ))
        })
        .collect()
}

/// Writes `cache` to `path`, creating the state directory if needed.
///
/// # Errors
///
/// Returns an error if the directory or file cannot be written.
pub fn save(path: &Path, cache: &ZipCache) -> Result<()> {
    let mut entries: Vec<CacheEntry> = cache
        .iter()
        .map(|(dest, fp)| CacheEntry {
            path: dest.to_string_lossy().into_owned(),
            hash: format!("{:016x}", fp.hash),
            mtime: fp.mtime,
            size: fp.size,
        })
        .collect();
    // Stable on-disk order for clean diffs.
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    let doc = CacheDoc { entries };
    let text = toml::to_string_pretty(&doc)
        .map_err(|e| Error::invalid(format!("serialise zip cache: {e}")))?;
    write_atomic(path, &text, "zip cache")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_then_load_round_trips() {
        let dir = std::env::temp_dir()
            .join(format!("hop-zipcache-test-{}", std::process::id()));
        let file = dir.join("zip-manifests.toml");
        let mut cache = ZipCache::new();
        cache.insert(
            PathBuf::from("/backups/rs-hop.zip"),
            ZipFingerprint {
                hash: 0xdead_beef_0000_1234,
                mtime: 1_700_000_000,
                size: 4096,
            },
        );
        save(&file, &cache).unwrap();
        let loaded = load(&file);
        assert_eq!(loaded, cache);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_yields_empty_cache() {
        let loaded = load(Path::new("/nonexistent/hop-zip-cache.toml"));
        assert!(loaded.is_empty());
    }
}
