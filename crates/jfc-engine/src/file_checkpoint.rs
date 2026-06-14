//! File-checkpoint store.
//!
//! Before any Write/Edit/MultiEdit tool mutates a file, the dispatcher
//! drops a snapshot of the *current* contents into `.jfc/checkpoints/`
//! so the user can recover from a botched edit. The directory layout is:
//!
//! ```text
//! .jfc/checkpoints/
//!   <unix-millis>__<sanitized-original-path>.bak
//! ```
//!
//! Restore copies the snapshot back over the original path. List walks
//! the directory and parses the filename for metadata (timestamp + the
//! original path). Prune removes anything older than `max_age`.
//!
//! Design notes:
//! - Snapshots are *copies*, not hard links — hard links would let a
//!   subsequent in-place edit corrupt the backup.
//! - Filename encoding uses `__` as a path separator (`/` → `__`) so
//!   we can recover the original path on listing without an index.
//! - Missing source files are *not* an error: the dispatcher calls
//!   `checkpoint_file` even for new-file writes, and we return a
//!   sentinel `.absent` marker so the restore can re-create the
//!   absence by deleting the file.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Default location for checkpoints, relative to cwd.
pub const CHECKPOINT_DIR: &str = ".jfc/checkpoints";

/// Filename suffix for "the file did not exist at checkpoint time".
const ABSENT_MARKER: &str = ".absent";

/// Filename suffix for normal backups.
const BACKUP_EXT: &str = "bak";

/// Path separator used inside encoded filenames.
const PATH_SEP_ESCAPE: &str = "__";

/// One row returned by [`list_checkpoints`].
#[derive(Debug, Clone)]
pub struct CheckpointEntry {
    pub original_path: PathBuf,
    pub backup_path: PathBuf,
    pub timestamp: SystemTime,
    pub size_bytes: u64,
}

/// Resolve the checkpoint directory and ensure it exists.
fn ensure_checkpoint_dir() -> io::Result<PathBuf> {
    let dir = PathBuf::from(CHECKPOINT_DIR);
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Encode a filesystem path into a single filename-safe token.
///
/// Strategy: leading `/` is dropped, every `/` becomes `__`, and the
/// `__` token itself is escaped to `_-_` so the reverse mapping stays
/// injective. We deliberately avoid percent-encoding (harder to read
/// in `ls` output).
fn encode_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    let stripped = s.strip_prefix('/').unwrap_or(&s);
    stripped.replace("__", "_-_").replace('/', PATH_SEP_ESCAPE)
}

fn decode_path(encoded: &str) -> PathBuf {
    let with_slashes = encoded.replace(PATH_SEP_ESCAPE, "/");
    let restored = with_slashes.replace("_-_", "__");
    PathBuf::from(format!("/{restored}"))
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default()
}

/// Snapshot `path`'s current contents to the checkpoint dir.
///
/// Returns the absolute backup path. If `path` does not currently
/// exist, an `.absent` sentinel file is written instead (zero bytes)
/// so restore can recreate the file's absence.
pub fn checkpoint_file(path: &Path) -> io::Result<PathBuf> {
    let dir = ensure_checkpoint_dir()?;
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let encoded = encode_path(&absolute);
    let ts = now_millis();

    let (ext, body): (&str, Option<Vec<u8>>) = match fs::read(&absolute) {
        Ok(bytes) => (BACKUP_EXT, Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            (ABSENT_MARKER.trim_start_matches('.'), None)
        }
        Err(e) => return Err(e),
    };

    let backup = dir.join(format!("{ts}__{encoded}.{ext}"));
    match body {
        Some(bytes) => fs::write(&backup, bytes)?,
        None => fs::write(&backup, [])?,
    }
    Ok(backup)
}

/// Restore the contents of `backup_path` onto `original_path`.
///
/// If the backup is an `.absent` sentinel, `original_path` is removed
/// (the file did not exist when the snapshot was taken).
pub fn restore_checkpoint(backup_path: &Path, original_path: &Path) -> io::Result<()> {
    let backup_name = backup_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if backup_name.ends_with(ABSENT_MARKER) {
        match fs::remove_file(original_path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    } else {
        if let Some(parent) = original_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(backup_path, original_path).map(|_| ())
    }
}

/// Walk the checkpoint directory and return every recoverable entry,
/// newest first. Files we can't parse are silently skipped.
pub fn list_checkpoints() -> io::Result<Vec<CheckpointEntry>> {
    let dir = PathBuf::from(CHECKPOINT_DIR);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    for dirent in fs::read_dir(&dir)? {
        let dirent = match dirent {
            Ok(d) => d,
            Err(_) => continue,
        };
        let path = dirent.path();
        let Some(parsed) = parse_entry(&path) else {
            continue;
        };
        entries.push(parsed);
    }
    entries.sort_by_key(|b| std::cmp::Reverse(b.timestamp));
    Ok(entries)
}

fn parse_entry(path: &Path) -> Option<CheckpointEntry> {
    let name = path.file_name()?.to_str()?;
    // Strip extension (.bak or .absent).
    let stem = name
        .strip_suffix(&format!(".{BACKUP_EXT}"))
        .or_else(|| name.strip_suffix(ABSENT_MARKER))?;
    let (ts_str, encoded) = stem.split_once("__")?;
    let ts: u128 = ts_str.parse().ok()?;
    let metadata = fs::metadata(path).ok()?;
    let timestamp = UNIX_EPOCH + Duration::from_millis(ts as u64);
    Some(CheckpointEntry {
        original_path: decode_path(encoded),
        backup_path: path.to_path_buf(),
        timestamp,
        size_bytes: metadata.len(),
    })
}

/// Delete checkpoints older than `max_age`. Returns the number of
/// snapshots removed.
pub fn prune_old_checkpoints(max_age: Duration) -> io::Result<usize> {
    let entries = list_checkpoints()?;
    let cutoff = SystemTime::now() - max_age;
    let mut removed = 0usize;
    for entry in entries {
        if entry.timestamp < cutoff {
            // Best-effort delete — failure to remove one file should
            // not abort pruning of the rest.
            if fs::remove_file(&entry.backup_path).is_ok() {
                removed += 1;
            }
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    // CHECKPOINT_DIR is process-global (relative cwd). Mutating the process
    // cwd races EVERY other test that reads cwd or forks a subprocess (the
    // bash/git/graph tests). A private mutex here only serialized checkpoint
    // tests against each other — not against those victims. Each cwd-mutating
    // test is therefore `#[serial_test::serial]` (a process-global lock shared
    // with the bash/git tests below), so no test ever observes a half-swapped
    // or already-deleted cwd.
    fn with_temp_cwd<R>(f: impl FnOnce(&Path) -> R) -> R {
        let tmp = tempfile::tempdir().unwrap();
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        let result = f(tmp.path());
        std::env::set_current_dir(original).unwrap();
        result
    }

    #[test]
    #[serial_test::serial]
    fn checkpoint_then_restore_round_trips_content() {
        with_temp_cwd(|root| {
            let target = root.join("hello.txt");
            fs::write(&target, b"original").unwrap();
            let backup = checkpoint_file(&target).unwrap();
            assert!(backup.exists());

            // Mutate the file.
            fs::write(&target, b"clobbered").unwrap();
            assert_eq!(fs::read(&target).unwrap(), b"clobbered");

            restore_checkpoint(&backup, &target).unwrap();
            assert_eq!(fs::read(&target).unwrap(), b"original");
        });
    }

    #[test]
    #[serial_test::serial]
    fn checkpoint_of_missing_file_records_absence() {
        with_temp_cwd(|root| {
            let target = root.join("never-existed.txt");
            let backup = checkpoint_file(&target).unwrap();
            assert!(backup.exists());
            assert!(
                backup
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .ends_with(".absent")
            );

            // Create the file, then restore — the file should be removed.
            fs::write(&target, b"created later").unwrap();
            restore_checkpoint(&backup, &target).unwrap();
            assert!(!target.exists());
        });
    }

    #[test]
    #[serial_test::serial]
    fn list_returns_newest_first() {
        with_temp_cwd(|root| {
            let a = root.join("a.txt");
            fs::write(&a, b"A").unwrap();
            let _b1 = checkpoint_file(&a).unwrap();
            // Ensure timestamps differ even on coarse clocks.
            std::thread::sleep(Duration::from_millis(5));
            fs::write(&a, b"AA").unwrap();
            let _b2 = checkpoint_file(&a).unwrap();

            let entries = list_checkpoints().unwrap();
            assert!(entries.len() >= 2);
            // Newest first.
            assert!(entries[0].timestamp >= entries[1].timestamp);
        });
    }

    #[test]
    #[serial_test::serial]
    fn prune_removes_old_entries() {
        with_temp_cwd(|root| {
            let a = root.join("a.txt");
            fs::write(&a, b"x").unwrap();
            let _ = checkpoint_file(&a).unwrap();

            // max_age=0 → everything is "old".
            let removed = prune_old_checkpoints(Duration::from_secs(0)).unwrap();
            assert!(removed >= 1);
            let entries = list_checkpoints().unwrap();
            assert!(entries.is_empty());
        });
    }

    #[test]
    fn encode_decode_round_trip() {
        let original = PathBuf::from("/home/user/project/src/main.rs");
        let encoded = encode_path(&original);
        assert!(!encoded.contains('/'));
        let decoded = decode_path(&encoded);
        assert_eq!(decoded, original);
    }

    #[test]
    fn encode_handles_double_underscore_in_path() {
        let original = PathBuf::from("/a/b__c/d.txt");
        let encoded = encode_path(&original);
        let decoded = decode_path(&encoded);
        assert_eq!(decoded, original);
    }
}
