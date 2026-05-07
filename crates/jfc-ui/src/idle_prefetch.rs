//! Idle prefetch — pre-execute likely-next read-only tools while the
//! user is reading the assistant's response.
//!
//! v132 watches the assistant's text mid-stream, identifies references
//! to file paths or symbols, and pre-runs `Read`/`Grep` against them in
//! a sandbox overlay. By the time the model decides to call the tool,
//! the result is cached.
//!
//! This module exposes a minimal heuristic + a process-global cache.
//! Full integration (consuming stream chunks live, dispatching reads in
//! background, returning cached results from the tool dispatcher) is
//! still TODO — this is the seam.
//!
//! ## Heuristic
//!
//! `extract_candidates(text)` finds `path/like.rs:42` and bare
//! `path/like.rs` references in the assistant text. These are the
//! prefetch candidates.
//!
//! ## Cache
//!
//! Keyed on `(path, offset, limit)`. TTL is short (90s) since file
//! state can change.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const TTL: Duration = Duration::from_secs(90);

#[derive(Clone)]
struct Entry {
    body: String,
    fetched_at: Instant,
}

static CACHE: Mutex<Option<HashMap<String, Entry>>> = Mutex::new(None);

/// Stash a Read result in the prefetch cache so a subsequent tool call
/// can short-circuit. Key is `path|offset|limit`.
pub fn put(path: &str, offset: Option<u64>, limit: Option<u64>, body: String) {
    let key = key_for(path, offset, limit);
    let Ok(mut guard) = CACHE.lock() else {
        return;
    };
    let map = guard.get_or_insert_with(HashMap::new);
    map.insert(
        key,
        Entry {
            body,
            fetched_at: Instant::now(),
        },
    );
}

/// Look up a cached prefetch. Returns `None` if missing or stale.
pub fn get(path: &str, offset: Option<u64>, limit: Option<u64>) -> Option<String> {
    let key = key_for(path, offset, limit);
    let mut guard = CACHE.lock().ok()?;
    let map = guard.get_or_insert_with(HashMap::new);
    let stale = map
        .get(&key)
        .map(|e| e.fetched_at.elapsed() >= TTL)
        .unwrap_or(false);
    if stale {
        map.remove(&key);
        return None;
    }
    map.get(&key).map(|e| e.body.clone())
}

fn key_for(path: &str, offset: Option<u64>, limit: Option<u64>) -> String {
    format!(
        "{path}|{}|{}",
        offset.map(|o| o.to_string()).unwrap_or_else(|| "*".to_owned()),
        limit.map(|l| l.to_string()).unwrap_or_else(|| "*".to_owned())
    )
}

/// Pull file references out of a chunk of assistant text — anything
/// that looks like `path/with/dots.ext` (optionally with `:line`). Used
/// to decide what to prefetch.
pub fn extract_candidates(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for word in text.split_whitespace() {
        // Strip surrounding punctuation so `(src/foo.rs)` matches.
        let trimmed = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '/' && c != '.' && c != '_' && c != '-' && c != ':');
        if !looks_like_path(trimmed) {
            continue;
        }
        out.push(trimmed.split(':').next().unwrap_or(trimmed).to_owned());
    }
    out.sort();
    out.dedup();
    out
}

fn looks_like_path(s: &str) -> bool {
    if s.is_empty() || s.contains("://") {
        return false;
    }
    if !s.contains('/') {
        return false;
    }
    // Must contain at least one `.` after the last `/` (extension hint).
    let after_last_slash = s.rsplit('/').next().unwrap_or("");
    after_last_slash.contains('.')
}

#[cfg(test)]
pub fn clear_for_test() {
    if let Ok(mut g) = CACHE.lock() {
        *g = Some(HashMap::new());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_then_get_normal() {
        clear_for_test();
        put("src/foo.rs", None, None, "body".into());
        assert_eq!(get("src/foo.rs", None, None).as_deref(), Some("body"));
    }

    #[test]
    fn get_with_different_offset_misses_normal() {
        clear_for_test();
        put("src/foo.rs", None, None, "body".into());
        assert!(get("src/foo.rs", Some(10), None).is_none());
    }

    #[test]
    fn extract_candidates_finds_paths_normal() {
        let text = "Look at src/foo.rs:42 for the bug; also src/bar.rs has it.";
        let c = extract_candidates(text);
        assert!(c.contains(&"src/foo.rs".to_owned()));
        assert!(c.contains(&"src/bar.rs".to_owned()));
    }

    #[test]
    fn extract_candidates_skips_urls_robust() {
        let text = "See https://example.com/x.html";
        assert!(extract_candidates(text).is_empty());
    }

    #[test]
    fn extract_candidates_requires_extension_robust() {
        let text = "Check src/just_a_dir for files";
        assert!(extract_candidates(text).is_empty());
    }
}
