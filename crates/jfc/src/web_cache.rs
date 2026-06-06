//! In-memory URL cache for `WebFetch` results with a TTL.
//!
//! v132 caches WebFetch responses by URL so the model can re-issue the same
//! fetch (e.g. while iterating on documentation it just retrieved) without
//! burning tokens on a fresh download. We mirror that with a small in-memory
//! map keyed on URL, capped at 64 entries to bound memory, with a 15-minute
//! TTL per the v132 timeout. Stale entries are pruned on lookup.
//!
//! Cache hits are logged via tracing and surfaced in the tool result body so
//! the model knows the content is from a previous turn — important if the
//! page was actually a live API endpoint.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const TTL: Duration = Duration::from_secs(15 * 60);
const MAX_ENTRIES: usize = 64;

#[derive(Debug, Clone)]
struct Entry {
    body: String,
    fetched_at: Instant,
}

static CACHE: Mutex<Option<HashMap<String, Entry>>> = Mutex::new(None);

/// Look up a URL in the cache. Returns the cached body if present and fresh,
/// `None` otherwise. Stale entries are evicted on access.
pub fn get(url: &str) -> Option<String> {
    let mut guard = CACHE.lock().ok()?;
    let map = guard.get_or_insert_with(HashMap::new);
    let stale = map
        .get(url)
        .map(|e| e.fetched_at.elapsed() >= TTL)
        .unwrap_or(false);
    if stale {
        map.remove(url);
        return None;
    }
    map.get(url).map(|e| e.body.clone())
}

/// Insert or refresh a URL in the cache. Evicts the oldest entry if at cap.
pub fn put(url: &str, body: String) {
    let Ok(mut guard) = CACHE.lock() else {
        return;
    };
    let map = guard.get_or_insert_with(HashMap::new);
    if map.len() >= MAX_ENTRIES
        && !map.contains_key(url)
        && let Some(oldest_key) = map
            .iter()
            .min_by_key(|(_, e)| e.fetched_at)
            .map(|(k, _)| k.clone())
    {
        map.remove(&oldest_key);
    }
    map.insert(
        url.to_owned(),
        Entry {
            body,
            fetched_at: Instant::now(),
        },
    );
}

/// Test-only: clear the cache between cases.
#[cfg(test)]
pub fn clear() {
    if let Ok(mut guard) = CACHE.lock() {
        *guard = Some(HashMap::new());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests share the process-global CACHE. Serialize them so a
    /// parallel test's `WebFetch` tool call can't `put()` between
    /// `clear()` and the assertion.
    fn test_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    #[test]
    fn put_then_get_returns_body_normal() {
        let _guard = test_lock().lock().unwrap_or_else(|p| p.into_inner());
        clear();
        put("https://a.example/", "hello".to_string());
        assert_eq!(get("https://a.example/"), Some("hello".to_string()));
    }

    #[test]
    fn miss_returns_none_normal() {
        let _guard = test_lock().lock().unwrap_or_else(|p| p.into_inner());
        clear();
        assert!(get("https://unknown.example/").is_none());
    }

    #[test]
    fn put_overwrites_existing_normal() {
        let _guard = test_lock().lock().unwrap_or_else(|p| p.into_inner());
        clear();
        put("https://b.example/", "first".to_string());
        put("https://b.example/", "second".to_string());
        assert_eq!(get("https://b.example/"), Some("second".to_string()));
    }

    #[test]
    fn cap_evicts_oldest_robust() {
        let _guard = test_lock().lock().unwrap_or_else(|p| p.into_inner());
        clear();
        for i in 0..MAX_ENTRIES {
            put(&format!("https://e{i}.example/"), format!("v{i}"));
        }
        // Fill to cap, then add one more — first entry should be gone.
        put("https://overflow.example/", "new".to_string());
        assert!(get("https://e0.example/").is_none());
        assert_eq!(get("https://overflow.example/"), Some("new".to_string()));
    }
}
