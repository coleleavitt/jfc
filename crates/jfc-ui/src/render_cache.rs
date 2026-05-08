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
    /// Total number of *visual* rows after word-wrapping at the entry's
    /// stored `width`. Computing this requires instantiating a `Paragraph`
    /// + `WordWrapper` per line which is itself the second-largest hot
    /// spot in the renderer (see `crates/jfc-ui/src/message_view.rs:64-74`
    /// pre-cache). Caching it here turns an O(lines × graphemes) per-frame
    /// computation into a single hash lookup.
    wrapped_line_count: usize,
    /// Insertion order counter for LRU eviction.
    generation: u64,
}

/// Content-addressed render cache.
pub struct RenderCache {
    map: HashMap<(u64, u16), CacheEntry>,
    generation: u64,
    /// Dedicated single-slot cache for the actively-streaming message. Kept out
    /// of the main LRU so growing content doesn't leave one dead cache entry per
    /// stream chunk. The text hash still participates in the key so appends
    /// invalidate the slot immediately.
    streaming_slot: Option<StreamingEntry>,
}

/// Single-slot storage for the streaming message's rendered lines.
struct StreamingEntry {
    message_idx: usize,
    width: u16,
    text_hash: u64,
    lines: Vec<Line<'static>>,
    wrapped_line_count: usize,
}

impl RenderCache {
    pub fn new() -> Self {
        Self {
            map: HashMap::with_capacity(128),
            generation: 0,
            streaming_slot: None,
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
        let wrapped_line_count = compute_wrapped_line_count(&lines, width);
        self.map.insert(
            key,
            CacheEntry {
                lines,
                wrapped_line_count,
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

        let lines = f(text, width);
        if self.map.len() >= MAX_ENTRIES {
            self.evict_oldest();
        }
        let wrapped_line_count = compute_wrapped_line_count(&lines, width);
        self.map.insert(
            key,
            CacheEntry {
                lines,
                wrapped_line_count,
                generation: current_gen,
            },
        );
        &self.map.get(&key).unwrap().lines
    }

    /// Total visual rows for this entry after word-wrapping at the cached
    /// width. Returns `None` on miss; callers should compute and `insert` to
    /// populate. This is the per-frame hot path for scroll-bottom math.
    pub fn wrapped_line_count(&self, text: &str, width: u16) -> Option<usize> {
        let key = (hash_text(text), width);
        self.map.get(&key).map(|e| e.wrapped_line_count)
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
        self.streaming_slot = None;
        self.generation = 0;
    }

    /// Store rendered lines for the actively-streaming message. Replaces any
    /// previous streaming content in-place without touching the main LRU map.
    pub fn set_streaming(
        &mut self,
        message_idx: usize,
        width: u16,
        text: &str,
        lines: Vec<Line<'static>>,
    ) {
        let wrapped_line_count = compute_wrapped_line_count(&lines, width);
        self.streaming_slot = Some(StreamingEntry {
            message_idx,
            width,
            text_hash: hash_text(text),
            lines,
            wrapped_line_count,
        });
    }

    /// Retrieve the streaming slot if it matches the given message index, width,
    /// and current text content.
    pub fn get_streaming(
        &self,
        message_idx: usize,
        width: u16,
        text: &str,
    ) -> Option<&[Line<'static>]> {
        self.streaming_slot.as_ref().and_then(|entry| {
            if entry.message_idx == message_idx
                && entry.width == width
                && entry.text_hash == hash_text(text)
            {
                Some(entry.lines.as_slice())
            } else {
                None
            }
        })
    }

    /// Wrapped line count for the streaming slot (mirrors `wrapped_line_count`
    /// for the main cache). Returns 0 if no streaming entry matches.
    pub fn streaming_wrapped_line_count(&self, message_idx: usize, width: u16) -> usize {
        self.streaming_slot
            .as_ref()
            .filter(|e| e.message_idx == message_idx && e.width == width)
            .map_or(0, |e| e.wrapped_line_count)
    }

    /// Clear the streaming slot. Called on `StreamDone` so the next render of
    /// that message falls through to the full `to_lines` + main cache path.
    pub fn clear_streaming(&mut self) {
        self.streaming_slot = None;
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

/// Mirror of the per-line wrap calculation in `message_view::message_view_total_lines`,
/// hoisted from per-frame to per-cache-insert. Uses the same `Paragraph + Wrap{trim:false}`
/// path so the count matches what ratatui will actually render.
fn compute_wrapped_line_count(lines: &[Line<'static>], width: u16) -> usize {
    use ratatui::widgets::{Paragraph, Wrap};
    if width == 0 {
        return lines.len();
    }
    let mut total = 0usize;
    for line in lines {
        if line.width() == 0 {
            total += 1;
        } else {
            total += Paragraph::new(line.clone())
                .wrap(Wrap { trim: false })
                .line_count(width)
                .max(1);
        }
    }
    total
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

    // Normal: `len` mirrors the underlying map size.
    #[test]
    fn len_tracks_size_normal() {
        let mut cache = RenderCache::new();
        assert_eq!(cache.len(), 0);
        cache.insert("a", 80, vec![Line::from("x")]);
        assert_eq!(cache.len(), 1);
        cache.insert("b", 80, vec![Line::from("y")]);
        assert_eq!(cache.len(), 2);
    }

    // Normal: `clear` empties everything and resets the generation counter.
    #[test]
    fn clear_resets_state_normal() {
        let mut cache = RenderCache::new();
        cache.insert("a", 80, vec![Line::from("x")]);
        cache.insert("b", 80, vec![Line::from("y")]);
        assert_eq!(cache.len(), 2);
        cache.clear();
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.generation, 0);
        // After clear, lookups must miss.
        assert!(cache.get("a", 80).is_none());
    }

    // Normal: `line_count` returns the row count for a cached entry without
    // exposing the lines themselves.
    #[test]
    fn line_count_returns_cached_size_normal() {
        let mut cache = RenderCache::new();
        let lines = vec![Line::from("a"), Line::from("b"), Line::from("c")];
        cache.insert("text", 40, lines);
        assert_eq!(cache.line_count("text", 40), Some(3));
    }

    // Robust: `line_count` misses on unknown text/width tuples.
    #[test]
    fn line_count_miss_returns_none_robust() {
        let mut cache = RenderCache::new();
        assert_eq!(cache.line_count("nope", 80), None);
        cache.insert("a", 80, vec![Line::from("x")]);
        // Different width — different key.
        assert_eq!(cache.line_count("a", 60), None);
    }

    // Normal: `get_or_insert_with` populates on miss and returns the freshly
    // computed lines, then keeps the same lines on a subsequent hit (closure
    // is NOT invoked again).
    #[test]
    fn get_or_insert_with_caches_normal() {
        let mut cache = RenderCache::new();
        let mut calls = 0u32;
        {
            let lines = cache.get_or_insert_with("hello", 80, |t, _w| {
                calls += 1;
                vec![Line::from(t.to_owned())]
            });
            assert_eq!(lines.len(), 1);
        }
        assert_eq!(calls, 1, "miss should invoke the closure once");
        {
            let lines = cache.get_or_insert_with("hello", 80, |_, _| {
                calls += 1;
                vec![Line::from("WRONG")]
            });
            assert_eq!(lines.len(), 1);
        }
        assert_eq!(calls, 1, "hit must NOT re-run the closure");
    }

    // Robust: `get_or_insert_with` evicts when the cache is at capacity, but
    // still returns the freshly inserted lines for the new key.
    #[test]
    fn get_or_insert_with_triggers_eviction_robust() {
        let mut cache = RenderCache::new();
        // Pre-fill so the next insert pushes us over MAX_ENTRIES.
        for i in 0..MAX_ENTRIES {
            cache.insert(&format!("pre_{i}"), 80, vec![Line::from("x")]);
        }
        let lines = cache.get_or_insert_with("new_one", 80, |t, _| vec![Line::from(t.to_owned())]);
        assert_eq!(lines.len(), 1);
        assert!(
            cache.len() <= MAX_ENTRIES,
            "eviction should keep cache bounded: {}",
            cache.len()
        );
    }

    // Robust: a `get` after `clear` re-populates from a cold start (no stale
    // generation/value leaks).
    #[test]
    fn cold_start_after_clear_is_full_miss_robust() {
        let mut cache = RenderCache::new();
        cache.insert("k", 80, vec![Line::from("x")]);
        cache.clear();
        assert!(cache.get("k", 80).is_none());
        assert_eq!(cache.line_count("k", 80), None);
    }

    // Regression: the streaming slot must be reusable within a single frame,
    // but appending text must invalidate it before the next frame.
    #[test]
    fn streaming_slot_invalidates_when_text_changes_regression() {
        let mut cache = RenderCache::new();
        cache.set_streaming(7, 80, "partial", vec![Line::from("partial")]);

        assert!(cache.get_streaming(7, 80, "partial").is_some());
        assert!(cache.get_streaming(7, 80, "partial plus more").is_none());
    }

    // Normal: stream completion drops the single-slot cache so the final
    // rendered message can enter the full content-addressed cache path.
    #[test]
    fn clear_streaming_drops_streaming_slot_normal() {
        let mut cache = RenderCache::new();
        cache.set_streaming(7, 80, "partial", vec![Line::from("partial")]);

        cache.clear_streaming();

        assert!(cache.get_streaming(7, 80, "partial").is_none());
    }

    // Normal: `get` updates the generation so a recently-touched entry is
    // last to be evicted under pressure.
    #[test]
    fn get_marks_entry_recent_normal() {
        let mut cache = RenderCache::new();
        cache.insert("first", 80, vec![Line::from("a")]);
        cache.insert("second", 80, vec![Line::from("b")]);
        let gen_before = cache.generation;
        // Touching `first` shouldn't bump generation (only insert does); but
        // it should leave the entry retrievable.
        let _ = cache.get("first", 80);
        assert_eq!(
            cache.generation, gen_before,
            "get should not bump generation"
        );
        assert!(cache.get("first", 80).is_some());
    }
}
