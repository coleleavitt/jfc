//! Crash-safe file replacement via temp + fsync + rename.

use std::io;
use std::path::{Path, PathBuf};

/// Build the sibling temp path used for the atomic write.
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
        if let Some(parent) = path.parent()
            && let Ok(dir) = File::open(parent)
        {
            dir.sync_all().ok();
        }
        Ok(())
    })();

    if result.is_err() {
        remove_file(&tmp).ok();
    }
    result
}
