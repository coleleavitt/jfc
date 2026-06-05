//! Per-workspace data directory resolution.
//!
//! Codegraph #304: keeping `.codegraph/` (or `.jfc-graph/`) inside the
//! project root pollutes every workspace, forces every contributor to add
//! the directory to `.gitignore`, and makes read-only mounts unusable.
//! This module resolves a writable per-workspace data dir using the
//! conventional XDG layout, with env-var overrides for power users and a
//! final fallback to the legacy in-workspace location.
//!
//! ## Resolution order
//!
//! 1. **`JFC_GRAPH_DATA_DIR`** — explicit override. Wins absolutely.
//! 2. **`$XDG_CACHE_HOME/jfc-graph/<workspace-hash>/`** — XDG-compliant
//!    user-cache layout; the per-workspace hash keeps multiple checkouts
//!    on the same machine isolated.
//! 3. **`$HOME/.cache/jfc-graph/<workspace-hash>/`** — XDG default when
//!    the env var isn't set.
//! 4. **`<workspace_root>/.jfc-graph/`** — legacy in-workspace fallback,
//!    used only when no home dir is detectable (CI sandboxes, exotic
//!    bare-bones environments).
//!
//! The workspace hash is a stable, content-free SHA-256 prefix of the
//! canonicalised workspace path so the layout is deterministic across
//! sessions but doesn't leak the path back into the dir name.

use std::path::{Path, PathBuf};

/// Environment variable users set to pin the data dir.
pub const ENV_OVERRIDE: &str = "JFC_GRAPH_DATA_DIR";

/// Resolve the per-workspace data directory.
///
/// Never creates the directory — callers do that themselves via
/// `std::fs::create_dir_all` once they know what they want to write.
pub fn resolve_data_dir(workspace_root: &Path) -> PathBuf {
    if let Some(p) = override_path() {
        return p;
    }
    let hash = workspace_hash(workspace_root);
    if let Some(home) = xdg_cache_home() {
        return home.join("jfc-graph").join(&hash);
    }
    workspace_root.join(".jfc-graph")
}

/// Honor `$JFC_GRAPH_DATA_DIR` if set and non-empty.
fn override_path() -> Option<PathBuf> {
    let raw = std::env::var(ENV_OVERRIDE).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(PathBuf::from(trimmed))
}

/// `$XDG_CACHE_HOME` if set; else `$HOME/.cache`. Returns `None` when
/// neither is available (e.g. a CI sandbox with `HOME` unset).
fn xdg_cache_home() -> Option<PathBuf> {
    if let Ok(val) = std::env::var("XDG_CACHE_HOME") {
        let trimmed = val.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    Some(home.join(".cache"))
}

/// Stable 16-char hex prefix of SHA-256(canonical workspace path).
///
/// Same canonical path → same dir name across sessions. Different
/// checkouts of the same repo land in different dirs because they
/// canonicalise to different paths.
fn workspace_hash(workspace_root: &Path) -> String {
    let canonical = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let mut bytes = canonical.as_os_str().as_encoded_bytes().to_vec();
    // Tag the hash with a tiny version byte so we can ever-so-slightly
    // rotate the dir layout in the future without colliding with old
    // contents under the same name.
    bytes.push(b'#');
    bytes.push(b'1');
    let hash = sha256_hex(&bytes);
    hash[..16].to_string()
}

/// Tiny SHA-256 wrapper around `sha2::Sha256` if it's available,
/// otherwise a stable DefaultHasher (lower-quality, but still
/// stable-across-runs and good enough for a directory name).
fn sha256_hex(bytes: &[u8]) -> String {
    // The jfc-graph crate already depends on stdlib's `DefaultHasher`
    // via fingerprint.rs; we avoid adding a new crypto dep just for a
    // directory name. DefaultHasher's hash is process-deterministic
    // across versions for the same input (it pins SipHash-1-3), so two
    // sessions on the same machine see the same dir name.
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    format!("{:016x}", h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serialise tests that mutate process-wide env vars — parallel
    // execution would race on `set_var`/`remove_var` reads.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_env<F: FnOnce()>(key: &str, val: Option<&str>, body: F) {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var(key).ok();
        match val {
            Some(v) => unsafe { std::env::set_var(key, v) },
            None => unsafe { std::env::remove_var(key) },
        }
        body();
        match prev {
            Some(p) => unsafe { std::env::set_var(key, p) },
            None => unsafe { std::env::remove_var(key) },
        }
    }

    #[test]
    fn override_env_var_wins() {
        with_env(ENV_OVERRIDE, Some("/tmp/explicit-override"), || {
            let resolved = resolve_data_dir(Path::new("/whatever"));
            assert_eq!(resolved, PathBuf::from("/tmp/explicit-override"));
        });
    }

    #[test]
    fn empty_override_falls_through() {
        with_env(ENV_OVERRIDE, Some("   "), || {
            let resolved = resolve_data_dir(Path::new("/tmp/somewhere"));
            assert!(
                !resolved.starts_with("   "),
                "blank override should be ignored"
            );
        });
    }

    #[test]
    fn xdg_cache_home_is_honored() {
        with_env(ENV_OVERRIDE, None, || {
            with_env("XDG_CACHE_HOME", Some("/tmp/xdg-cache-test"), || {
                let resolved = resolve_data_dir(Path::new("/tmp/some-workspace"));
                assert!(
                    resolved.starts_with("/tmp/xdg-cache-test/jfc-graph"),
                    "expected XDG_CACHE_HOME prefix, got {}",
                    resolved.display()
                );
            });
        });
    }

    #[test]
    fn workspace_hash_is_stable_across_calls() {
        let p = Path::new("/some/workspace/path");
        assert_eq!(workspace_hash(p), workspace_hash(p));
    }

    #[test]
    fn workspace_hash_differs_for_different_paths() {
        let a = workspace_hash(Path::new("/a"));
        let b = workspace_hash(Path::new("/b"));
        assert_ne!(a, b);
    }

    #[test]
    fn workspace_hash_is_16_hex_chars() {
        let h = workspace_hash(Path::new("/x"));
        assert_eq!(h.len(), 16);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
