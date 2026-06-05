//! Hand-rolled Adapton-style incremental query cache (Phase 5).
//!
//! ## Why
//!
//! DSL queries today are pure functions of `(query_text, graph_state)` —
//! `fn("foo") | callers | depth 3` re-runs from scratch on every call,
//! even when the graph hasn't changed. For interactive use (jfc-ui
//! sidebars, repeated tool invocations) the same query is run hundreds
//! of times against an unchanging graph.
//!
//! This cache memoises query results keyed by:
//!
//! 1. **Query hash** — a `u64` hash of the canonicalised query text.
//! 2. **Graph revision** — the `CodeGraph::current_revision()` at the
//!    point of caching. A bumped revision invalidates every entry
//!    monotonically; we don't need per-entry dependency tracking for
//!    coarse correctness.
//! 3. **Read-set fingerprint** (optional, finer-grained) — the set of
//!    `NodeId`s the query observed during execution. If only an
//!    unrelated file changed, the cache entry is still valid even
//!    though the global revision bumped.
//!
//! ## Adapton vs Salsa
//!
//! Salsa would impose a strict query-DSL with macro-generated derived
//! queries; we keep the user-level DSL flexible by running the cache
//! at the **outer** boundary (entire query → entire result). Cache
//! granularity is coarser, but the memory model is trivially correct:
//! every entry is either valid (its dependencies didn't change) or
//! invalidated (any dependency changed → recompute).
//!
//! ## Concurrency
//!
//! `DashMap` underlies the cache so concurrent readers never block each
//! other. Writers take a per-shard lock briefly. The cache itself is
//! `Send + Sync`.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use dashmap::DashMap;

use crate::nodes::NodeId;

/// Stable, content-addressed key for a cache entry. Includes both the
/// query and the graph revision at lookup time.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QueryKey {
    /// Hash of the canonicalised query text. Two queries that
    /// canonicalise to the same string produce the same hash.
    pub query_hash: u64,
    /// `CodeGraph::current_revision()` at cache time.
    pub revision: u64,
}

impl QueryKey {
    pub fn new(query_text: &str, revision: u64) -> Self {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        query_text.trim().hash(&mut h);
        Self {
            query_hash: h.finish(),
            revision,
        }
    }
}

/// Per-entry metadata: which `NodeId`s the cached result observed
/// during execution. Used by [`QueryCache::invalidate_for_node`] to
/// drop only the entries that actually depended on a changed node.
#[derive(Debug, Clone, Default)]
pub struct ReadSet {
    pub nodes: HashSet<NodeId>,
}

impl ReadSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, id: &NodeId) {
        self.nodes.insert(id.clone());
    }

    pub fn contains(&self, id: &NodeId) -> bool {
        self.nodes.contains(id)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// Cache entry: stores the result alongside its read-set.
#[derive(Debug, Clone)]
pub struct CacheEntry<V> {
    pub value: Arc<V>,
    pub read_set: ReadSet,
}

/// Concurrent query cache.
pub struct QueryCache<V> {
    map: DashMap<QueryKey, CacheEntry<V>>,
    /// Soft cap — when the cache grows past this we evict the oldest
    /// entries. 0 means unlimited.
    capacity: usize,
}

impl<V> QueryCache<V>
where
    V: Send + Sync,
{
    pub fn new() -> Self {
        Self {
            map: DashMap::new(),
            capacity: 1024,
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            map: DashMap::new(),
            capacity,
        }
    }

    /// Lookup. Returns `None` on miss or if the entry is stale (the
    /// staleness check is just `revision` equality at this layer; the
    /// finer-grained per-node invalidation is the caller's job via
    /// [`Self::invalidate_for_node`]).
    pub fn get(&self, key: &QueryKey) -> Option<Arc<V>> {
        self.map.get(key).map(|e| e.value.clone())
    }

    /// Insert. Soft-evicts when the cache exceeds `capacity`.
    pub fn put(&self, key: QueryKey, value: V, read_set: ReadSet) {
        let entry = CacheEntry {
            value: Arc::new(value),
            read_set,
        };
        self.map.insert(key, entry);
        self.maybe_evict();
    }

    /// Invalidate every entry whose read-set contains `node`. Called by
    /// `CodeGraph::update_file` when a file mutation rewrites the
    /// nodes for a given file.
    pub fn invalidate_for_node(&self, node: &NodeId) {
        self.map.retain(|_k, v| !v.read_set.contains(node));
    }

    /// Invalidate every entry whose recorded revision is strictly less
    /// than `current_rev`. Belt-and-suspenders fallback for when fine
    /// invalidation isn't tracked.
    pub fn invalidate_older_than(&self, current_rev: u64) {
        self.map.retain(|k, _| k.revision >= current_rev);
    }

    /// Drop everything.
    pub fn clear(&self) {
        self.map.clear();
    }

    /// Number of entries currently cached.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    fn maybe_evict(&self) {
        if self.capacity == 0 {
            return;
        }
        if self.map.len() <= self.capacity {
            return;
        }
        // Coarse eviction: drop entries with the smallest revision
        // numbers (oldest by graph time). DashMap doesn't expose
        // ordered iteration; collect, sort, drop.
        let mut keys: Vec<(QueryKey, u64)> = self
            .map
            .iter()
            .map(|r| (r.key().clone(), r.key().revision))
            .collect();
        keys.sort_by_key(|(_, rev)| *rev);
        let drop_count = self.map.len().saturating_sub(self.capacity);
        for (k, _) in keys.into_iter().take(drop_count) {
            self.map.remove(&k);
        }
    }
}

impl<V> Default for QueryCache<V>
where
    V: Send + Sync,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_id(n: u64) -> NodeId {
        NodeId(n)
    }

    #[test]
    fn cache_round_trip_normal() {
        let c: QueryCache<Vec<u32>> = QueryCache::new();
        let key = QueryKey::new("fn(\"foo\")", 1);
        let mut rs = ReadSet::new();
        rs.record(&mk_id(1));
        c.put(key.clone(), vec![1, 2, 3], rs);
        let got = c.get(&key).unwrap();
        assert_eq!(*got, vec![1, 2, 3]);
    }

    #[test]
    fn cache_miss_returns_none() {
        let c: QueryCache<u32> = QueryCache::new();
        assert!(c.get(&QueryKey::new("nope", 0)).is_none());
    }

    #[test]
    fn invalidate_for_node_drops_dependent_entries() {
        let c: QueryCache<u32> = QueryCache::new();
        let mut rs1 = ReadSet::new();
        rs1.record(&mk_id(7));
        c.put(QueryKey::new("a", 1), 1, rs1);

        let mut rs2 = ReadSet::new();
        rs2.record(&mk_id(99));
        c.put(QueryKey::new("b", 1), 2, rs2);

        c.invalidate_for_node(&mk_id(7));
        assert!(c.get(&QueryKey::new("a", 1)).is_none());
        assert_eq!(*c.get(&QueryKey::new("b", 1)).unwrap(), 2);
    }

    #[test]
    fn invalidate_older_than_drops_old_revisions() {
        let c: QueryCache<u32> = QueryCache::new();
        c.put(QueryKey::new("old", 1), 100, ReadSet::new());
        c.put(QueryKey::new("new", 5), 200, ReadSet::new());
        c.invalidate_older_than(3);
        assert!(c.get(&QueryKey::new("old", 1)).is_none());
        assert_eq!(*c.get(&QueryKey::new("new", 5)).unwrap(), 200);
    }

    #[test]
    fn capacity_evicts_oldest() {
        let c: QueryCache<u32> = QueryCache::with_capacity(2);
        c.put(QueryKey::new("a", 1), 1, ReadSet::new());
        c.put(QueryKey::new("b", 2), 2, ReadSet::new());
        c.put(QueryKey::new("c", 3), 3, ReadSet::new());
        // Either a or b is evicted; c must remain.
        assert!(c.get(&QueryKey::new("c", 3)).is_some());
        assert!(c.len() <= 2);
    }

    #[test]
    fn read_set_record_and_contains() {
        let mut rs = ReadSet::new();
        rs.record(&mk_id(1));
        rs.record(&mk_id(2));
        rs.record(&mk_id(1)); // dedup
        assert_eq!(rs.len(), 2);
        assert!(rs.contains(&mk_id(1)));
        assert!(!rs.contains(&mk_id(99)));
    }

    #[test]
    fn key_equality_canonicalises_whitespace() {
        let k1 = QueryKey::new("fn(\"x\")", 1);
        let k2 = QueryKey::new("  fn(\"x\")  ", 1);
        assert_eq!(k1, k2);
    }

    #[test]
    fn key_distinct_for_distinct_revisions() {
        let k1 = QueryKey::new("q", 1);
        let k2 = QueryKey::new("q", 2);
        assert_ne!(k1, k2);
    }

    #[test]
    fn clear_empties() {
        let c: QueryCache<u32> = QueryCache::new();
        c.put(QueryKey::new("x", 1), 1, ReadSet::new());
        c.clear();
        assert!(c.is_empty());
    }

    #[test]
    fn shared_value_is_arc() {
        let c: QueryCache<u32> = QueryCache::new();
        let key = QueryKey::new("x", 1);
        c.put(key.clone(), 42, ReadSet::new());
        let a = c.get(&key).unwrap();
        let b = c.get(&key).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
    }
}
