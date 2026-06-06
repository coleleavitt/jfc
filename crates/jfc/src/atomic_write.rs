//! Crash-safe file replacement via temp + fsync + rename.
//!
//! Direct `fs::write(path, content)` is **not** crash-safe — a SIGKILL or
//! power loss mid-write leaves `path` truncated or partially written.
//! For newline-delimited formats (JSONL sessions) a partial final line
//! breaks parsing of every later read; for JSON it can wipe the entire
//! file if the write failed before the closing brace.
//!
//! The recipe is the textbook POSIX one:
//!
//!   1. Write the full payload to `<path>.tmp.<pid>.<unix_micros>`.
//!   2. `fsync` the temp file so the bytes hit disk.
//!   3. `rename(tmp, path)` — atomic on every modern filesystem.
//!
//! On failure at any step, the temp file is removed so the directory
//! does not accumulate orphans.
//!
//! This mirrors the inline pattern that already exists in
//! `providers/file_lock.rs:421` for the Anthropic OAuth accounts store
//! — extracted here so session save, memory create, config save, and
//! team-memory writes can share the same guarantees without copying
//! the recipe four ways.
//!
//! ## Mode preservation
//!
//! `rename` does NOT carry mode bits from the temp file forward —
//! credential-grade callers (anthropic-accounts) should re-apply
//! `0o600` on the destination after `write_atomic` returns. That's the
//! same dance `file_lock.rs` already does and we keep that pattern;
//! this helper only handles the durability piece, not the permissions
//! piece, because the right mode is caller-specific.

use std::io;
use std::path::{Path, PathBuf};

/// Build the sibling temp path used for the atomic write. The format
/// `<name>.tmp.<pid>.<unix_micros>` collision-avoids parallel writers
/// against the same path from the same process.
fn temp_path_for(path: &Path) -> PathBuf {
    let pid = std::process::id();
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros())
        .unwrap_or(0);
    let mut suffix = std::ffi::OsString::from(".tmp.");
    suffix.push(pid.to_string());
    suffix.push(".");
    suffix.push(unique.to_string());
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(&suffix);
    PathBuf::from(tmp)
}

/// Synchronously write `content` to `path` atomically (temp + fsync + rename).
///
/// On failure the temp file is removed. The destination is either the
/// previous contents (if rename did not run) or the new contents (if
/// rename completed). Readers will never observe a half-written file.
pub fn write_atomic_sync(path: &Path, content: impl AsRef<[u8]>) -> io::Result<()> {
    use std::fs::{File, remove_file, rename};
    use std::io::Write;

    let tmp = temp_path_for(path);
    let result = (|| -> io::Result<()> {
        let mut f = File::create(&tmp)?;
        f.write_all(content.as_ref())?;
        f.sync_all()?;
        drop(f);
        rename(&tmp, path)?;
        // fsync the parent directory so the rename (the directory entry
        // update) is itself durable. Without this, a power loss after
        // rename returns can still lose the new entry on some filesystems.
        if let Some(parent) = path.parent()
            && let Ok(dir) = File::open(parent)
        {
            let _ = dir.sync_all();
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = remove_file(&tmp);
    }
    result
}

/// Asynchronously write `content` to `path` atomically. Same guarantees
/// as `write_atomic_sync`; uses `tokio::fs` so the write does not block
/// the runtime.
///
/// `fsync` is performed before rename — without it, the rename can land
/// on a kernel that has not yet flushed the file's data pages, and a
/// power loss between rename and flush leaves a zero-byte destination.
pub async fn write_atomic(path: &Path, content: impl AsRef<[u8]>) -> io::Result<()> {
    use tokio::fs::{File, remove_file, rename};
    use tokio::io::AsyncWriteExt;

    let tmp = temp_path_for(path);
    let bytes = content.as_ref();

    let result = async {
        let mut f = File::create(&tmp).await?;
        f.write_all(bytes).await?;
        f.sync_all().await?;
        drop(f);
        rename(&tmp, path).await?;
        // fsync the parent directory so the rename is durable across a
        // power loss (see the sync variant for the full rationale).
        if let Some(parent) = path.parent()
            && let Ok(dir) = File::open(parent).await
        {
            let _ = dir.sync_all().await;
        }
        Ok::<(), io::Error>(())
    }
    .await;

    if result.is_err() {
        let _ = remove_file(&tmp).await;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // Normal: a clean write replaces the destination atomically and
    // leaves no temp files behind.
    #[test]
    fn sync_write_replaces_destination_normal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.json");
        std::fs::write(&path, b"old").unwrap();

        write_atomic_sync(&path, b"new").unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        // No leftover temp siblings.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name())
            .filter(|n| n.to_string_lossy().contains(".tmp."))
            .collect();
        assert!(
            leftovers.is_empty(),
            "atomic_write left temp siblings: {leftovers:?}"
        );
    }

    // Normal: writing to a fresh path creates the file with the exact
    // bytes — no truncation, no extra bytes from a stale temp.
    #[test]
    fn sync_write_creates_fresh_file_normal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fresh.json");
        assert!(!path.exists());

        write_atomic_sync(&path, b"hello world").unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"hello world");
    }

    // Robust: failures during the write do not delete the previous
    // destination — `rename` only fires after the temp is durable on
    // disk, so a mid-write crash leaves the OLD contents intact, not
    // a corrupted/half-written destination.
    #[test]
    fn sync_write_preserves_old_on_dest_unwritable_robust() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.json");
        std::fs::write(&path, b"old").unwrap();

        // Make the parent directory read-only so the temp create fails.
        // On platforms that ignore dir mode (Windows), this test is a
        // no-op assertion that the original file survives.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500));
            let result = write_atomic_sync(&path, b"new");
            // Restore so the tempdir can clean up.
            let _ = std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700));
            assert!(
                result.is_err(),
                "expected write to fail under read-only dir"
            );
        }
        // Whether the write failed or not, the destination must be a
        // valid full payload (never a half-written one).
        let observed = std::fs::read(&path).unwrap();
        assert!(
            observed == b"old" || observed == b"new",
            "destination is half-written: {observed:?}"
        );
    }

    // Normal: async path round-trips bytes identically to the sync path.
    #[tokio::test]
    async fn async_write_round_trips_normal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("async.json");

        write_atomic(&path, b"async-payload").await.unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"async-payload");
    }

    // Robust: temp path is a sibling of `path`, not a child of it.
    // Required because `rename` only works within a filesystem; placing
    // the temp in `/tmp` would cross mount points on systems where
    // `~/.config/jfc/...` is on a separate filesystem from `/tmp`.
    #[test]
    fn temp_path_is_sibling_of_destination_robust() {
        let path = Path::new("/home/user/.config/jfc/sessions/ses_x.json");
        let tmp = temp_path_for(path);
        assert_eq!(
            tmp.parent(),
            path.parent(),
            "temp must live in the destination's parent so rename is atomic"
        );
        let tmp_name = tmp.file_name().unwrap().to_string_lossy().into_owned();
        let path_name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            tmp_name.starts_with(&path_name),
            "temp name {tmp_name} should be prefixed with destination name {path_name}"
        );
        assert!(
            tmp_name.contains(".tmp."),
            "temp name {tmp_name} should carry the .tmp. marker"
        );
    }
}
