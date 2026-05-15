//! Filesystem overlay used by the speculation engine.
//!
//! Mirrors v132's `eH8` (overlay creation), `nA_` (commit copy-back), and
//! `sY$` (cleanup with retry-3). Each overlay lives at
//! `<base>/<pid>/<tool_id>/` where `<base>` defaults to `/tmp/jfc-speculation`.
//! Inside the overlay, real-filesystem absolute paths are mirrored verbatim:
//! a write to `/home/u/proj/foo.txt` lands at `<overlay_root>/home/u/proj/foo.txt`.
//! On commit we walk the overlay and copy each file back to its real
//! location; on abort we just `rm -rf` the overlay dir.
use std::io;
use std::path::{Path, PathBuf};

use super::safety::{self, SafetyError};

/// A handle to a per-tool filesystem overlay.
///
/// Drop runs cleanup as a best effort, but explicit `commit()`/`abort()`
/// is preferred so callers can observe errors.
#[derive(Debug)]
pub struct Overlay {
    /// Project root the overlay is anchored to (absolute, lexically normal).
    cwd: PathBuf,
    /// `<base>/<pid>/<tool_id>/` — the root of the redirected mirror.
    overlay_root: PathBuf,
    /// Files that have been written into the overlay (relative to overlay_root).
    /// Used by `commit_changes` so we can copy back exactly what changed
    /// without globbing the whole tree.
    written: Vec<PathBuf>,
    /// Set when `commit()` or `abort()` has consumed this overlay.
    consumed: bool,
}

/// Default overlay base directory: `/tmp/jfc-speculation`.
pub fn default_base() -> PathBuf {
    std::env::temp_dir().join("jfc-speculation")
}

impl Overlay {
    /// Create the overlay directory tree for `tool_id` rooted at `cwd`.
    ///
    /// The overlay is allocated under `base/<pid>/<tool_id>/`; `base` is
    /// usually [`default_base`] but tests inject a temp dir.
    pub fn create(base: &Path, cwd: &Path, tool_id: &str) -> io::Result<Self> {
        let cwd_norm = safety::normalize(cwd).unwrap_or_else(|| cwd.to_owned());
        let pid = std::process::id();
        let overlay_root = base.join(pid.to_string()).join(sanitize(tool_id));
        std::fs::create_dir_all(&overlay_root)?;
        Ok(Self {
            cwd: cwd_norm,
            overlay_root,
            written: Vec::new(),
            consumed: false,
        })
    }

    /// Project root this overlay is anchored to.
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// Absolute path to the overlay root directory.
    pub fn overlay_root(&self) -> &Path {
        &self.overlay_root
    }

    /// Map a real-filesystem absolute path to its overlay equivalent.
    ///
    /// Validates `real` lies under `cwd` and isn't a symlink-out / system
    /// path. The overlay layout mirrors the absolute path under
    /// `overlay_root`, so `cwd/foo.txt` becomes
    /// `overlay_root/<cwd>/foo.txt`.
    pub fn redirect(&mut self, real: &Path) -> Result<PathBuf, SafetyError> {
        let normalized = safety::validate(&self.cwd, real)?;
        let rel = normalized
            .strip_prefix("/")
            .unwrap_or(normalized.as_path())
            .to_path_buf();
        let overlay_path = self.overlay_root.join(&rel);
        // Track for commit. De-dupe on repeat writes.
        if !self.written.contains(&rel) {
            self.written.push(rel);
        }
        Ok(overlay_path)
    }

    /// Write `contents` into the overlay at the location derived from
    /// `real`. Convenience wrapper used by Write-style tools.
    pub fn write_file(&mut self, real: &Path, contents: &[u8]) -> io::Result<PathBuf> {
        let overlay_path = self.redirect(real).map_err(io::Error::from)?;
        if let Some(parent) = overlay_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&overlay_path, contents)?;
        Ok(overlay_path)
    }

    /// Copy every tracked file from the overlay back to its real location.
    ///
    /// Returns the list of real-filesystem paths that were updated.
    pub fn commit_changes(&mut self) -> io::Result<Vec<PathBuf>> {
        if self.consumed {
            return Err(io::Error::other("overlay already consumed"));
        }
        let mut updated = Vec::with_capacity(self.written.len());
        for rel in &self.written {
            let src = self.overlay_root.join(rel);
            // Re-validate against the real cwd so a malicious mid-flight
            // mutation of `written` cannot push files outside the project.
            let real = PathBuf::from("/").join(rel);
            let validated = safety::validate(&self.cwd, &real).map_err(io::Error::from)?;
            if let Some(parent) = validated.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&src, &validated)?;
            updated.push(validated);
        }
        self.consumed = true;
        // Best-effort cleanup; commit succeeded regardless.
        let _ = remove_with_retry(&self.overlay_root, 3);
        Ok(updated)
    }

    /// Discard the overlay without copying anything back.
    pub fn abort(&mut self) -> io::Result<()> {
        if self.consumed {
            return Ok(());
        }
        self.consumed = true;
        remove_with_retry(&self.overlay_root, 3)
    }

    /// Remove the overlay directory (alias for `abort()` semantics) but
    /// without flipping the consumed flag — callers use this when they
    /// want to wipe the on-disk state but keep tracking metadata.
    pub fn cleanup(&self) -> io::Result<()> {
        remove_with_retry(&self.overlay_root, 3)
    }

    /// Paths that have been written into the overlay so far (relative to `/`).
    pub fn written_paths(&self) -> &[PathBuf] {
        &self.written
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        if !self.consumed {
            let _ = remove_with_retry(&self.overlay_root, 3);
        }
    }
}

/// Replace path-unsafe chars in a tool id with `_`.
fn sanitize(id: &str) -> String {
    id.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => c,
            _ => '_',
        })
        .collect()
}

/// `rm -rf` the path with up to `attempts` retries (mirrors `sY$` retry-3).
fn remove_with_retry(path: &Path, attempts: u32) -> io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut last = None;
    for _ in 0..attempts {
        match std::fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("remove failed")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fresh(base: &TempDir, cwd: &TempDir, tool: &str) -> Overlay {
        Overlay::create(base.path(), cwd.path(), tool).expect("overlay create")
    }

    #[test]
    fn redirect_maps_under_overlay_root_normal() {
        let base = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let mut ov = fresh(&base, &cwd, "tool-1");
        let real = cwd.path().join("foo.txt");
        let mapped = ov.redirect(&real).unwrap();
        assert!(mapped.starts_with(ov.overlay_root()));
        assert!(mapped.ends_with("foo.txt"));
    }

    #[test]
    fn redirect_rejects_dotdot_robust() {
        let base = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let mut ov = fresh(&base, &cwd, "tool-2");
        let outside = cwd.path().join("../../../etc/passwd");
        let err = ov.redirect(&outside).unwrap_err();
        assert!(matches!(
            err,
            SafetyError::Escape(_) | SafetyError::SystemPath(_)
        ));
    }

    #[test]
    fn redirect_rejects_symlink_escape_robust() {
        use std::os::unix::fs::symlink;
        let base = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        symlink("/tmp", cwd.path().join("escape")).unwrap();
        let mut ov = fresh(&base, &cwd, "tool-sym");
        let target = cwd.path().join("escape").join("hostile.txt");
        let err = ov.redirect(&target).unwrap_err();
        assert!(matches!(err, SafetyError::SymlinkEscape { .. }));
    }

    #[test]
    fn write_file_lands_in_overlay_normal() {
        let base = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let mut ov = fresh(&base, &cwd, "tool-3");
        let real = cwd.path().join("hello.txt");
        let mapped = ov.write_file(&real, b"hi").unwrap();
        assert!(mapped.exists(), "overlay file written");
        assert!(!real.exists(), "real file unchanged before commit");
    }

    #[test]
    fn commit_copies_overlay_to_real_normal() {
        let base = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let mut ov = fresh(&base, &cwd, "tool-4");
        let real = cwd.path().join("greet.txt");
        ov.write_file(&real, b"hello").unwrap();
        let updated = ov.commit_changes().unwrap();
        assert_eq!(updated.len(), 1);
        assert!(real.exists());
        assert_eq!(std::fs::read(&real).unwrap(), b"hello");
        // Overlay dir was cleaned up.
        assert!(!ov.overlay_root().exists());
    }

    #[test]
    fn abort_does_not_touch_real_robust() {
        let base = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let mut ov = fresh(&base, &cwd, "tool-5");
        let real = cwd.path().join("never.txt");
        ov.write_file(&real, b"discarded").unwrap();
        ov.abort().unwrap();
        assert!(!real.exists(), "abort must not write to real fs");
        assert!(!ov.overlay_root().exists(), "overlay removed on abort");
    }

    #[test]
    fn cleanup_removes_overlay_normal() {
        let base = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let ov = fresh(&base, &cwd, "tool-6");
        let root = ov.overlay_root().to_owned();
        assert!(root.exists());
        ov.cleanup().unwrap();
        assert!(!root.exists());
    }

    #[test]
    fn drop_cleans_unconsumed_overlay_robust() {
        let base = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let root = {
            let ov = fresh(&base, &cwd, "tool-drop");
            ov.overlay_root().to_owned()
        };
        assert!(!root.exists(), "Drop should rm -rf the overlay");
    }

    #[test]
    fn double_commit_errors_robust() {
        let base = TempDir::new().unwrap();
        let cwd = TempDir::new().unwrap();
        let mut ov = fresh(&base, &cwd, "tool-7");
        let real = cwd.path().join("x.txt");
        ov.write_file(&real, b"a").unwrap();
        ov.commit_changes().unwrap();
        let err = ov.commit_changes().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
    }

    #[test]
    fn sanitize_strips_unsafe_chars_robust() {
        assert_eq!(sanitize("toolu_01ABC-_xyz"), "toolu_01ABC-_xyz");
        assert_eq!(sanitize("../../etc"), "______etc");
        assert_eq!(sanitize("a/b\\c d"), "a_b_c_d");
    }
}
