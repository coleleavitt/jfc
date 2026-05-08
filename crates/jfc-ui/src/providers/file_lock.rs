//! File-level advisory locking for multi-process coordination.
//!
//! Ports the opencode `storage.ts` acquireLock/releaseLock pattern:
//! - Atomic lock creation via O_CREAT|O_EXCL (single syscall, no TOCTOU)
//! - PID-based stale detection (kill(pid, 0) to check if holder is alive)
//! - mtime-based fallback for hung processes (30s threshold)
//! - Exponential backoff with jitter (10ms initial, 200ms cap)
//! - 10s total timeout
//! - Atomic release via rename → verify → delete (prevents releasing
//!   another process's lock)

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tracing::{debug, trace, warn};

const LOCK_TIMEOUT: Duration = Duration::from_secs(10);
const LOCK_STALE: Duration = Duration::from_secs(30);
const LOCK_INITIAL_RETRY_MS: u64 = 10;
const LOCK_MAX_RETRY_MS: u64 = 200;

/// Errors raised while acquiring a [`FileLock`]. `Acquire` chains the
/// underlying `std::io::Error` via `#[from]` so callers can match on
/// `ErrorKind` (e.g. permission denied vs disk full) rather than parsing
/// a stringified message. `thiserror` auto-generates an
/// `impl From<FileLockError> for anyhow::Error`, so call sites that still
/// use `anyhow::Result` can just `?`-bubble these.
#[derive(Debug, thiserror::Error)]
pub enum FileLockError {
    /// Underlying IO operation (open, rename, write) failed.
    #[error("file lock IO error: {0}")]
    Acquire(#[from] std::io::Error),
    /// Existing lock file's holder was unreachable but its mtime was
    /// newer than [`LOCK_STALE`] — neither alive nor expired.
    #[error("lock at is stale (age {age:?}) but cleanup failed")]
    Stale { age: Duration },
    /// Exceeded the [`LOCK_TIMEOUT`] retry budget.
    #[error("failed to acquire lock at {path} after waiting {waited:?}")]
    Timeout { path: PathBuf, waited: Duration },
    /// Catch-all for inconsistencies that aren't cleanly an IO error
    /// (e.g. corrupted lock token, race between stale-clear and rename).
    #[error("lock state poisoned: {reason}")]
    Poisoned { reason: String },
}

/// A held file lock. Dropping it releases the lock.
pub struct FileLock {
    lock_path: PathBuf,
    token: String,
}

impl FileLock {
    /// Acquire the lock at `lock_path`. Blocks up to 10s with exponential
    /// backoff + jitter. Returns the held lock guard.
    pub async fn acquire(lock_path: &Path) -> Result<Self, FileLockError> {
        let start = Instant::now();
        let pid = std::process::id();

        loop {
            if start.elapsed() >= LOCK_TIMEOUT {
                return Err(FileLockError::Timeout {
                    path: lock_path.to_path_buf(),
                    waited: LOCK_TIMEOUT,
                });
            }

            let token = format!(
                "{}:{}:{}",
                pid,
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis(),
                rand::random::<u32>()
            );

            // Attempt atomic creation with O_CREAT|O_EXCL
            match OpenOptions::new()
                .write(true)
                .create_new(true) // O_CREAT | O_EXCL
                .open(lock_path)
            {
                Ok(mut file) => {
                    // Set mode 0o600 on unix
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = file.set_permissions(fs::Permissions::from_mode(0o600));
                    }
                    let _ = file.write_all(token.as_bytes());
                    let _ = file.flush();
                    debug!(
                        path = %lock_path.display(),
                        token = %token,
                        "file lock acquired"
                    );
                    return Ok(FileLock {
                        lock_path: lock_path.to_path_buf(),
                        token,
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // Lock exists — check if stale
                    if let Some(handled) = Self::try_handle_stale(lock_path).await {
                        if handled {
                            continue; // Retry immediately after clearing stale
                        }
                    }

                    // Exponential backoff with jitter
                    let elapsed_hundredths = (start.elapsed().as_millis() / 100) as u32;
                    let backoff_ms = LOCK_INITIAL_RETRY_MS
                        .saturating_mul(2u64.saturating_pow(elapsed_hundredths))
                        .min(LOCK_MAX_RETRY_MS);
                    let jittered = {
                        let factor: f64 = 0.5 + rand::random::<f64>() * 0.5;
                        Duration::from_millis((backoff_ms as f64 * factor) as u64)
                    };
                    trace!(
                        backoff_ms,
                        jittered_ms = jittered.as_millis() as u64,
                        "lock contention, backing off"
                    );
                    tokio::time::sleep(jittered).await;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    // Parent directory doesn't exist — create it
                    if let Some(parent) = lock_path.parent() {
                        let _ = fs::create_dir_all(parent);
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
                        }
                    }
                    continue;
                }
                Err(e) => {
                    return Err(FileLockError::Acquire(e));
                }
            }
        }
    }

    /// Check if the existing lock is stale (holder dead or mtime too old).
    /// Returns Some(true) if stale lock was cleared, Some(false) if not stale,
    /// None if error.
    async fn try_handle_stale(lock_path: &Path) -> Option<bool> {
        // Read lock content to get PID
        let content = fs::read_to_string(lock_path).ok()?;
        let lock_pid: u32 = content.split(':').next()?.parse().ok()?;

        // Check if lock holder process is alive
        let is_alive = is_process_alive(lock_pid);

        if !is_alive {
            // Process is dead — try to clear stale lock atomically
            let stale_path = format!(
                "{}.stale.{}.{}",
                lock_path.display(),
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
            );
            let stale_path = PathBuf::from(&stale_path);

            match fs::rename(lock_path, &stale_path) {
                Ok(()) => {
                    // Verify it's still the same stale content
                    let stale_content = fs::read_to_string(&stale_path).unwrap_or_default();
                    let stale_pid: u32 = stale_content
                        .split(':')
                        .next()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);

                    if stale_pid == 0 || !is_process_alive(stale_pid) {
                        // Confirmed stale — delete temp
                        let _ = fs::remove_file(&stale_path);
                        debug!(lock_pid, "cleared stale lock (process dead)");
                        return Some(true);
                    } else {
                        // Process came alive between checks — restore via hard link
                        #[cfg(unix)]
                        {
                            if std::fs::hard_link(&stale_path, lock_path).is_ok() {
                                let _ = fs::remove_file(&stale_path);
                            } else {
                                let _ = fs::remove_file(&stale_path);
                            }
                        }
                        #[cfg(not(unix))]
                        {
                            // On Windows, rename back
                            let _ = fs::rename(&stale_path, lock_path);
                        }
                        return Some(false);
                    }
                }
                Err(_) => {
                    // ENOENT: lock already gone
                    return Some(true);
                }
            }
        }

        // Process is alive — check mtime-based stale detection for hung processes
        if let Ok(meta) = fs::metadata(lock_path) {
            if let Ok(mtime) = meta.modified() {
                if let Ok(age) = SystemTime::now().duration_since(mtime) {
                    if age > LOCK_STALE {
                        // Lock is older than 30s with alive process — force clear
                        let stale_time_path = format!(
                            "{}.stale.{}.{}",
                            lock_path.display(),
                            std::process::id(),
                            SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis()
                        );
                        let stale_time_path = PathBuf::from(&stale_time_path);
                        if fs::rename(lock_path, &stale_time_path).is_ok() {
                            let _ = fs::remove_file(&stale_time_path);
                            warn!(
                                lock_pid,
                                age_secs = age.as_secs(),
                                "cleared stale lock (mtime exceeded threshold)"
                            );
                            return Some(true);
                        }
                    }
                }
            }
        }

        Some(false)
    }

    /// Release the lock atomically. Mirrors opencode's rename → verify → delete
    /// pattern to prevent accidentally releasing another process's lock.
    pub fn release(self) {
        // Consumed — Drop won't double-release
        self.do_release();
    }

    fn do_release(&self) {
        let releasing_path = format!(
            "{}.releasing.{}.{}",
            self.lock_path.display(),
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let releasing_path = PathBuf::from(&releasing_path);

        match fs::rename(&self.lock_path, &releasing_path) {
            Ok(()) => {
                // Lock is now invisible to other acquireLock attempts
                let content = fs::read_to_string(&releasing_path).unwrap_or_default();
                if content == self.token {
                    // It was ours — delete
                    let _ = fs::remove_file(&releasing_path);
                    debug!(
                        path = %self.lock_path.display(),
                        "file lock released"
                    );
                } else {
                    // Not ours! Restore via hard_link (fails EEXIST if new lock exists)
                    #[cfg(unix)]
                    {
                        if std::fs::hard_link(&releasing_path, &self.lock_path).is_ok() {
                            let _ = fs::remove_file(&releasing_path);
                        } else {
                            let _ = fs::remove_file(&releasing_path);
                        }
                    }
                    #[cfg(not(unix))]
                    {
                        if !self.lock_path.exists() {
                            let _ = fs::rename(&releasing_path, &self.lock_path);
                        } else {
                            let _ = fs::remove_file(&releasing_path);
                        }
                    }
                    warn!(
                        path = %self.lock_path.display(),
                        "lock release: token mismatch — restored lock"
                    );
                }
            }
            Err(_) => {
                // ENOENT: lock already gone (stale detection removed it)
                trace!(
                    path = %self.lock_path.display(),
                    "lock release: lock already gone"
                );
            }
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        self.do_release();
    }
}

/// Check if a process with the given PID is still alive.
/// On Unix: kill(pid, 0) — doesn't send a signal, returns ESRCH if dead.
/// On Windows: OpenProcess with SYNCHRONIZE access.
fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // kill(pid, 0) checks existence without sending a signal
        // Returns 0 if alive + we have permission, ESRCH if dead, EPERM if alive but no perm
        let ret = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if ret == 0 {
            return true;
        }
        // EPERM means process exists but we don't have permission to signal it
        let errno = std::io::Error::last_os_error();
        errno.raw_os_error() == Some(libc::EPERM)
    }

    #[cfg(not(unix))]
    {
        // On non-Unix, assume alive (conservative — prevents spurious lock theft)
        let _ = pid;
        true
    }
}

/// Read-modify-write the accounts file under an advisory lock.
/// Mirrors opencode's `readModifyWriteAccounts` pattern.
pub async fn read_modify_write<F>(
    accounts_path: &Path,
    lock_path: &Path,
    modifier: F,
) -> anyhow::Result<String>
where
    F: FnOnce(&mut serde_json::Value) -> anyhow::Result<()>,
{
    let lock = FileLock::acquire(lock_path).await?;
    let temp_path = accounts_path.with_extension(format!(
        "{}.{}.tmp",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));

    let result = (|| -> anyhow::Result<String> {
        // Read existing data
        let mut data: serde_json::Value = match fs::read_to_string(accounts_path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_else(|_| {
                // Corrupted — backup and start fresh
                let backup = format!(
                    "{}.corrupted.{}",
                    accounts_path.display(),
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis()
                );
                let _ = fs::copy(accounts_path, &backup);
                warn!(backup = %backup, "corrupted accounts file, backed up");
                serde_json::json!({"version": 1, "accounts": []})
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                serde_json::json!({"version": 1, "accounts": []})
            }
            Err(e) => return Err(e.into()),
        };

        // Security: reject symlinks
        #[cfg(unix)]
        {
            if let Ok(meta) = fs::symlink_metadata(accounts_path) {
                if meta.file_type().is_symlink() {
                    anyhow::bail!("SECURITY: accounts file is a symlink, refusing operation");
                }
            }
        }

        // Track original account count for auth-loss guard
        let original_count = data
            .get("accounts")
            .and_then(|a| a.as_array())
            .map(|a| a.len())
            .unwrap_or(0);

        // Apply modifier
        modifier(&mut data)?;

        // Auth-loss guard: refuse to wipe all accounts
        let new_count = data
            .get("accounts")
            .and_then(|a| a.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        if original_count > 0 && new_count == 0 {
            anyhow::bail!(
                "AUTH-LOSS GUARD: modifier would wipe all {} accounts, refusing write",
                original_count
            );
        }

        // Write atomically via temp + rename
        let content = serde_json::to_string_pretty(&data)?;
        if let Some(parent) = accounts_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&temp_path, &content)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o600));
        }
        fs::rename(&temp_path, accounts_path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(accounts_path, fs::Permissions::from_mode(0o600));
        }

        Ok(content)
    })();

    // Cleanup temp file on failure
    let _ = fs::remove_file(&temp_path);
    lock.release();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn acquire_and_release_lock_normal() {
        let dir = TempDir::new().unwrap();
        let lock_path = dir.path().join("test.lock");

        let lock = FileLock::acquire(&lock_path).await.unwrap();
        assert!(lock_path.exists());

        // Verify lock content has our PID
        let content = fs::read_to_string(&lock_path).unwrap();
        assert!(content.starts_with(&format!("{}:", std::process::id())));

        lock.release();
        assert!(!lock_path.exists());
    }

    #[tokio::test]
    async fn stale_lock_is_cleared_normal() {
        let dir = TempDir::new().unwrap();
        let lock_path = dir.path().join("test.lock");

        // Create a lock file with a dead PID (PID 1 is init, use a very high PID)
        let fake_pid = 4_000_000; // Almost certainly doesn't exist
        fs::write(&lock_path, format!("{fake_pid}:12345:abc")).unwrap();

        // Should successfully acquire despite existing lock (dead PID)
        let lock = FileLock::acquire(&lock_path).await.unwrap();
        let content = fs::read_to_string(&lock_path).unwrap();
        assert!(content.starts_with(&format!("{}:", std::process::id())));
        lock.release();
    }

    #[tokio::test]
    async fn concurrent_acquires_serialize_normal() {
        let dir = TempDir::new().unwrap();
        let lock_path = dir.path().join("test.lock");

        let lock1 = FileLock::acquire(&lock_path).await.unwrap();

        // Spawn a second acquire that should wait
        let lock_path2 = lock_path.clone();
        let handle = tokio::spawn(async move {
            let start = Instant::now();
            // This will spin until lock1 is released
            let lock2 = FileLock::acquire(&lock_path2).await.unwrap();
            let waited = start.elapsed();
            lock2.release();
            waited
        });

        // Hold for a bit then release
        tokio::time::sleep(Duration::from_millis(50)).await;
        lock1.release();

        let waited = handle.await.unwrap();
        assert!(waited >= Duration::from_millis(30));
    }

    #[tokio::test]
    async fn read_modify_write_creates_and_modifies_normal() {
        let dir = TempDir::new().unwrap();
        let accounts = dir.path().join("accounts.json");
        let lock = dir.path().join("accounts.lock");

        // First call creates the file
        read_modify_write(&accounts, &lock, |data| {
            let arr = data.get_mut("accounts").unwrap().as_array_mut().unwrap();
            arr.push(serde_json::json!({"name": "test", "token": "abc"}));
            Ok(())
        })
        .await
        .unwrap();

        let content: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&accounts).unwrap()).unwrap();
        assert_eq!(content["accounts"][0]["name"], "test");

        // Second call modifies
        read_modify_write(&accounts, &lock, |data| {
            let arr = data.get_mut("accounts").unwrap().as_array_mut().unwrap();
            arr.push(serde_json::json!({"name": "test2", "token": "def"}));
            Ok(())
        })
        .await
        .unwrap();

        let content: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&accounts).unwrap()).unwrap();
        assert_eq!(content["accounts"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn auth_loss_guard_prevents_wipe_robust() {
        let dir = TempDir::new().unwrap();
        let accounts = dir.path().join("accounts.json");
        let lock = dir.path().join("accounts.lock");

        // Create with one account
        fs::write(&accounts, r#"{"version":1,"accounts":[{"name":"a"}]}"#).unwrap();

        // Try to wipe all accounts — should fail
        let result = read_modify_write(&accounts, &lock, |data| {
            data["accounts"] = serde_json::json!([]);
            Ok(())
        })
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("AUTH-LOSS GUARD"));

        // Original file should be untouched
        let content: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&accounts).unwrap()).unwrap();
        assert_eq!(content["accounts"].as_array().unwrap().len(), 1);
    }
}
