//! Render-line cache for markdown content.
//!
//! `markdown::to_lines()` runs pulldown-cmark parsing + syntect highlighting on every
//! call. Without caching, this work is repeated 2× per frame (once for height
//! calculation, once for actual rendering). With a large conversation (100+ messages
//! with code blocks), that means ~200 full parses per frame at 12.5 FPS — the dominant
//! bottleneck for scroll jank.
//!
//! This module provides a content-addressed cache keyed on `(hash(text), width)`.
//! Entries self-invalidate when text content changes (streaming appends → different hash).
//! The cache is LRU-bounded to prevent unbounded memory growth during long sessions.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use ratatui::text::Line;

/// Maximum number of cached entries. At ~50 lines × 200 bytes per entry average,
/// 512 entries ≈ 5MB upper bound. Generous enough that a 200-message conversation
/// never thrashes, small enough to not bloat RSS.
const MAX_ENTRIES: usize = 512;

/// A single cached result from `markdown::to_lines()`.
#[derive(Clone)]
struct CacheEntry {
    lines: Vec<Line<'static>>,
    /// Insertion order counter for LRU eviction.
    generation: u64,
}

/// Content-addressed render cache.
pub struct RenderCache {
    map: HashMap<(u64, u16), CacheEntry>,
    generation: u64,
}

impl RenderCache {
    pub fn new() -> Self {
        Self {
            map: HashMap::with_capacity(128),
            generation: 0,
        }
    }

    /// Look up cached lines for `text` at `width`. Returns `None` on miss.
    pub fn get(&mut self, text: &str, width: u16) -> Option<&[Line<'static>]> {
        let key = (hash_text(text), width);
        if let Some(entry) = self.map.get_mut(&key) {
            entry.generation = self.generation;
            Some(&entry.lines)
        } else {
            None
        }
    }

    /// Insert rendered lines into the cache.
    pub fn insert(&mut self, text: &str, width: u16, lines: Vec<Line<'static>>) {
        if self.map.len() >= MAX_ENTRIES {
            self.evict_oldest();
        }
        let key = (hash_text(text), width);
        self.generation += 1;
        self.map.insert(
            key,
            CacheEntry {
                lines,
                generation: self.generation,
            },
        );
    }

    /// Get or compute: returns cached lines or calls `f` to produce them.
    pub fn get_or_insert_with<F>(&mut self, text: &str, width: u16, f: F) -> &[Line<'static>]
    where
        F: FnOnce(&str, u16) -> Vec<Line<'static>>,
    {
        let key = (hash_text(text), width);
        self.generation += 1;
        let current_gen = self.generation;

        if self.map.contains_key(&key) {
            let entry = self.map.get_mut(&key).unwrap();
            entry.generation = current_gen;
            return &entry.lines;
        }

        // Miss — compute and store
        let lines = f(text, width);
        if self.map.len() >= MAX_ENTRIES {
            self.evict_oldest();
        }
        self.map.insert(key, CacheEntry { lines, generation: current_gen });
        &self.map.get(&key).unwrap().lines
    }

    /// Return the line count without cloning the full vec.
    pub fn line_count(&mut self, text: &str, width: u16) -> Option<usize> {
        let key = (hash_text(text), width);
        if let Some(entry) = self.map.get_mut(&key) {
            entry.generation = self.generation;
            Some(entry.lines.len())
        } else {
            None
        }
    }

    /// Evict ~25% of oldest entries.
    fn evict_oldest(&mut self) {
        let target = MAX_ENTRIES * 3 / 4;
        if self.map.len() <= target {
            return;
        }

        // Find the generation threshold to evict below
        let mut gens: Vec<u64> = self.map.values().map(|e| e.generation).collect();
        gens.sort_unstable();
        let cutoff_idx = self.map.len() - target;
        let cutoff = gens[cutoff_idx];

        self.map.retain(|_, e| e.generation > cutoff);
    }

    /// Invalidate the entire cache (e.g. on terminal resize / theme change).
    pub fn clear(&mut self) {
        self.map.clear();
        self.generation = 0;
    }

    /// Number of cached entries (for diagnostics).
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.map.len()
    }
}

fn hash_text(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_cache_hit() {
        let mut cache = RenderCache::new();
        let lines = vec![Line::from("hello")];
        cache.insert("hello world", 80, lines.clone());
        assert_eq!(cache.get("hello world", 80).unwrap().len(), 1);
        assert!(cache.get("hello world", 60).is_none()); // different width
        assert!(cache.get("different", 80).is_none()); // different text
    }

    #[test]
    fn eviction_respects_max() {
        let mut cache = RenderCache::new();
        for i in 0..MAX_ENTRIES + 10 {
            let text = format!("entry_{i}");
            cache.insert(&text, 80, vec![Line::from("x")]);
        }
        // Should have evicted to stay bounded
        assert!(cache.map.len() <= MAX_ENTRIES);
    }
}
