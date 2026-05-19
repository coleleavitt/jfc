use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::runtime::ExecutionResult;

type ScratchpadMap = BTreeMap<String, String>;

fn scratchpad_path() -> PathBuf {
    crate::daemon::DaemonPaths::default_user()
        .base_dir
        .join("scratchpad.json")
}

#[cfg(unix)]
fn lock_scratchpad(lock_path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::fd::AsRawFd;

    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .read(true)
        .open(lock_path)?;
    // SAFETY: flock operates on a valid file descriptor and the kernel
    // releases the lock when `file` is dropped.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(file)
}

#[cfg(not(unix))]
fn lock_scratchpad(lock_path: &Path) -> std::io::Result<std::fs::File> {
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .read(true)
        .open(lock_path)
}

fn load(path: &Path) -> ScratchpadMap {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<ScratchpadMap>(&raw).ok())
        .unwrap_or_default()
}

fn save(path: &Path, map: &ScratchpadMap) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(map).map_err(std::io::Error::other)?;
    crate::atomic_write::write_atomic_sync(path, json)
}

pub(crate) fn execute_scratchpad_read(key: &str) -> ExecutionResult {
    let path = scratchpad_path();
    let lock_path = path.with_extension("json.lock");
    let _guard = match lock_scratchpad(&lock_path) {
        Ok(guard) => guard,
        Err(e) => return ExecutionResult::failure(format!("Scratchpad lock failed: {e}")),
    };

    let map = load(&path);
    match map.get(key) {
        Some(value) => ExecutionResult::success(value.clone()),
        None => ExecutionResult::failure(format!(
            "Key '{key}' not found in scratchpad. Available keys: {}",
            map.keys().cloned().collect::<Vec<_>>().join(", ")
        )),
    }
}

pub(crate) fn execute_scratchpad_write(key: &str, value: &str) -> ExecutionResult {
    let path = scratchpad_path();
    let lock_path = path.with_extension("json.lock");
    let _guard = match lock_scratchpad(&lock_path) {
        Ok(guard) => guard,
        Err(e) => return ExecutionResult::failure(format!("Scratchpad lock failed: {e}")),
    };

    let mut map = load(&path);
    map.insert(key.to_string(), value.to_string());
    match save(&path, &map) {
        Ok(()) => ExecutionResult::success(format!(
            "Written to scratchpad key '{key}' ({} bytes)",
            value.len()
        )),
        Err(e) => ExecutionResult::failure(format!("Scratchpad write failed: {e}")),
    }
}
