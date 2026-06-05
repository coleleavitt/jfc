//! Iteration-order-independent fingerprints for graph state.
//!
//! ## Why this exists
//!
//! When a graph (or graph subset) is hashed for use as a *cache key* —
//! "have I seen this graph state before?" — the hash MUST be stable across
//! process restarts and independent of the order nodes/edges were inserted.
//!
//! [`std::collections::HashMap`] iteration order is randomized per process
//! (DOS-resistance via a per-process seed), so any fingerprint that folds a
//! `HashMap`'s `iter()` directly will be different on every run. Worse, it
//! may *occasionally* match by coincidence and serve stale data.
//!
//! See Michael Woerister's note on `HashStable` in t-compiler/incremental:
//!
//! > It's impossible to take iteration order into account when fingerprinting
//! > these data structures, because the iteration order might be different in
//! > every compilation session due to outside factors.
//!
//! The remedy in rustc-incremental is `UnorderedMap` and explicit
//! sort-before-hash. We follow the same idiom: any container whose iteration
//! order is not part of its semantic identity is sorted by a stable key
//! before being folded into the hasher.
//!
//! ## Status
//!
//! There is currently no live consumer of [`Fingerprintable`] inside
//! `jfc-graph` — incremental caching, on-disk cache validation, and
//! cross-session memoization haven't landed yet. This module exists as
//! **forward infrastructure**: it defines the trait and a correct
//! implementation for [`CodeGraph`] so that when the first consumer arrives
//! they cannot accidentally introduce iteration-order leakage.
//!
//! ## Choice of digest size
//!
//! [`Fingerprint`] wraps a `u64` truncated from a BLAKE3 digest.
//! BLAKE3 is deterministic and platform-independent, so fingerprints
//! computed on CI can be shared with developer machines without
//! worrying about per-process seeds or compiler-version drift (the
//! original SipHash-based `DefaultHasher` suffered from both issues).
//!
//! The backing is still **non-cryptographic** in spirit — we only keep
//! 64 bits, so collisions are at the birthday-bound (~2³²). This is
//! fine for trusted cache keys; if a future consumer needs full
//! collision resistance, widen to `[u8; 32]` — the trait shape stays
//! the same.
//!
//! ## Sort key contract
//!
//! Every implementation MUST canonicalize:
//!
//! - `HashMap<K: Ord, V>` → collect to `Vec<(&K, &V)>`, sort by `K`, then hash.
//! - `HashSet<T: Ord>` → collect to `Vec<&T>`, sort, then hash.
//! - Sequences whose order *is* meaningful (e.g. an event log) → hash in
//!   sequence order. Document this on the impl.
//! - Structs with all-deterministic fields → hash field-by-field in
//!   declaration order (this is the Rust derive convention; it's stable).

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

use crate::cache;
use crate::edges::EdgeData;
use crate::graph::CodeGraph;
use crate::nodes::{NodeData, NodeId};

/// Iteration-order-independent digest of some graph state.
///
/// See the [module docs](self) for the semantics, the choice of `u64`
/// backing, and the sort-before-hash contract that every [`Fingerprintable`]
/// implementation must uphold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Fingerprint(u64);

impl Fingerprint {
    /// Wrap a precomputed digest value. Prefer [`FingerprintHasher`] for
    /// computing a fingerprint from input data.
    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    /// Raw underlying digest. Exposed for serialization and equality with
    /// values stored in caches keyed on `u64`.
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

// --- bridges to `crate::cache::Fingerprint` -----------------------------
//
// `crate::cache` defines its own `Fingerprint(u64)` for content-of-bytes
// keys feeding `AnalysisCache`. The two carry the same payload but model
// different concerns (graph-state digest vs. file-content digest). Bridging
// them here lets a future consumer feed a `CodeGraph` fingerprint into the
// analysis cache without re-hashing.

impl From<cache::Fingerprint> for Fingerprint {
    fn from(value: cache::Fingerprint) -> Self {
        Self(value.as_u64())
    }
}

impl From<Fingerprint> for cache::Fingerprint {
    fn from(value: Fingerprint) -> Self {
        cache::Fingerprint::from_u64(value.as_u64())
    }
}

/// Builder for fingerprints — wraps a [`blake3::Hasher`] behind the
/// [`std::hash::Hasher`] trait so that `Hash`-able types can be streamed
/// into a BLAKE3 digest without any call-site changes.
///
/// Using a dedicated newtype (instead of exposing the inner hasher
/// directly) keeps the public API stable: swapping the backing digest
/// (previously SipHash, now BLAKE3) required zero changes to callers.
pub struct FingerprintHasher(Blake3StdHasher);

/// Adapter: exposes a [`blake3::Hasher`] through [`std::hash::Hasher`]
/// so `Hash::hash()` calls land in BLAKE3.
struct Blake3StdHasher(blake3::Hasher);

impl Hasher for Blake3StdHasher {
    fn write(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    fn finish(&self) -> u64 {
        let hash = self.0.finalize();
        let bytes: [u8; 8] = hash.as_bytes()[..8]
            .try_into()
            .expect("BLAKE3 output is always >= 8 bytes");
        u64::from_le_bytes(bytes)
    }
}

impl FingerprintHasher {
    pub fn new() -> Self {
        Self(Blake3StdHasher(blake3::Hasher::new()))
    }

    /// Stream a `Hash`-able value into the digest. Use this for primitives
    /// and pre-canonicalized sequences.
    pub fn update<T: Hash + ?Sized>(&mut self, value: &T) {
        value.hash(&mut self.0);
    }

    /// Stream a `HashMap` whose iteration order must NOT influence the
    /// digest. Sorts by key, then folds `(key, value)` pairs in sorted order.
    pub fn update_unordered_map<K, V>(&mut self, map: &HashMap<K, V>)
    where
        K: Hash + Ord,
        V: Hash,
    {
        let mut entries: Vec<(&K, &V)> = map.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        // Length-prefix to disambiguate {(a, x)} from {(a, x), ()}, etc.
        entries.len().hash(&mut self.0);
        for (k, v) in entries {
            k.hash(&mut self.0);
            v.hash(&mut self.0);
        }
    }

    /// Finalize and return the fingerprint.
    pub fn finish(self) -> Fingerprint {
        Fingerprint(self.0.finish())
    }
}

impl Default for FingerprintHasher {
    fn default() -> Self {
        Self::new()
    }
}

/// Types that can produce a stable, iteration-order-independent fingerprint
/// of their semantic content.
///
/// Two values that are "the same graph state" — same nodes, same edges, same
/// per-node metadata — MUST produce the same [`Fingerprint`] regardless of
/// the order they were assembled in or the process that ran the computation.
///
/// Implementations MUST:
/// 1. Canonicalize any unordered container (sort by key/element).
/// 2. Length-prefix variable-length sequences to prevent boundary collisions.
/// 3. Hash fields in a fixed declaration order.
pub trait Fingerprintable {
    fn fingerprint(&self) -> Fingerprint;
}

// --- impls --------------------------------------------------------------

/// Fingerprint a `NodeData` deterministically.
///
/// `NodeData::metadata` is a `HashMap<String, String>`, so we canonicalize
/// it before hashing. All other fields are scalars/strings whose `Hash` impl
/// is order-stable.
fn hash_node_data(hasher: &mut FingerprintHasher, node: &NodeData) {
    hasher.update(&node.id);
    hasher.update(&node.kind);
    hasher.update(&node.name);
    hasher.update(&node.qualified_name);
    hasher.update(&node.file_path);
    hasher.update(&node.span);
    hasher.update(&node.visibility);
    hasher.update_unordered_map(&node.metadata);
}

/// Fingerprint an `EdgeData` deterministically. All fields are
/// order-deterministic; `weight: f32` is hashed via its raw bit pattern so
/// that NaN-vs-NaN (which compares unequal) still produces a stable digest.
fn hash_edge_data(hasher: &mut FingerprintHasher, edge: &EdgeData) {
    hasher.update(&edge.kind);
    hasher.update(&edge.source_span);
    hasher.update(&edge.weight.to_bits());
}

impl Fingerprintable for CodeGraph {
    /// Fingerprint a [`CodeGraph`] in a way that's independent of the order
    /// nodes and edges were inserted.
    ///
    /// Strategy: collect all nodes by [`NodeId`], sort by `NodeId`, hash in
    /// sorted order. Then collect all edges as `(from_id, to_id, edge_data)`,
    /// sort by `(from, to, kind)`, hash in sorted order. This is O(N log N + E
    /// log E) — acceptable for cache-key computation, which by definition
    /// happens at boundaries (before/after a graph edit), not in hot loops.
    fn fingerprint(&self) -> Fingerprint {
        let mut hasher = FingerprintHasher::new();

        // --- nodes: sort by NodeId, hash in sorted order -----------------
        let mut node_refs: Vec<(&NodeId, &NodeData)> = self
            .all_node_ids()
            .into_iter()
            .filter_map(|id| self.get_node(id).map(|data| (id, data)))
            .collect();
        node_refs.sort_by(|a, b| a.0.cmp(b.0));

        hasher.update(&"jfc-graph::CodeGraph::nodes");
        hasher.update(&node_refs.len());
        for (id, data) in &node_refs {
            hasher.update(*id);
            hash_node_data(&mut hasher, data);
        }

        // --- edges: collect (from, to, data) tuples and sort -------------
        // We walk every node's outgoing edges so that iteration order over
        // node_refs (which is already sorted) gives a deterministic edge
        // stream. We additionally sort within each source for full
        // order-independence: petgraph internally orders edges by insertion,
        // and we don't want that to leak.
        let mut edges: Vec<(&NodeId, &NodeId, &EdgeData)> = node_refs
            .iter()
            .flat_map(|(id, _)| {
                self.get_edges_from(id)
                    .into_iter()
                    .map(move |(target, data)| (*id, target, data))
            })
            .collect();
        edges.sort_by(|a, b| {
            a.0.cmp(b.0).then_with(|| a.1.cmp(b.1)).then_with(|| {
                // EdgeKind doesn't impl Ord; fall back to fingerprinting
                // each EdgeData and comparing those. Same source+target
                // with different kinds is rare, so this is cheap in
                // practice.
                let mut ha = FingerprintHasher::new();
                hash_edge_data(&mut ha, a.2);
                let mut hb = FingerprintHasher::new();
                hash_edge_data(&mut hb, b.2);
                ha.finish().as_u64().cmp(&hb.finish().as_u64())
            })
        });

        hasher.update(&"jfc-graph::CodeGraph::edges");
        hasher.update(&edges.len());
        for (from, to, data) in edges {
            hasher.update(from);
            hasher.update(to);
            hash_edge_data(&mut hasher, data);
        }

        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::nodes::{NodeData, NodeKind, Span, Visibility};

    fn sample_span() -> Span {
        Span {
            file: PathBuf::from("src/lib.rs"),
            start_line: 1,
            start_col: 0,
            end_line: 10,
            end_col: 1,
            byte_range: 0..100,
        }
    }

    fn make_node(name: &str, kind: NodeKind) -> NodeData {
        let id = NodeId::new("src/lib.rs", &format!("crate::{name}"), kind);
        NodeData {
            id,
            kind,
            name: name.to_string(),
            qualified_name: format!("crate::{name}"),
            file_path: PathBuf::from("src/lib.rs"),
            span: sample_span(),
            visibility: Visibility::Public,
            metadata: HashMap::new(),
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
        }
    }

    #[test]
    fn fingerprint_hasher_unordered_map_ignores_insertion_order() {
        let mut a: HashMap<&str, u32> = HashMap::new();
        a.insert("z", 26);
        a.insert("a", 1);
        a.insert("m", 13);

        let mut b: HashMap<&str, u32> = HashMap::new();
        b.insert("a", 1);
        b.insert("m", 13);
        b.insert("z", 26);

        let mut ha = FingerprintHasher::new();
        ha.update_unordered_map(&a);
        let mut hb = FingerprintHasher::new();
        hb.update_unordered_map(&b);

        assert_eq!(ha.finish(), hb.finish());
    }

    #[test]
    fn fingerprint_hasher_unordered_map_distinguishes_different_contents() {
        let mut a: HashMap<&str, u32> = HashMap::new();
        a.insert("a", 1);
        let mut b: HashMap<&str, u32> = HashMap::new();
        b.insert("a", 2);

        let mut ha = FingerprintHasher::new();
        ha.update_unordered_map(&a);
        let mut hb = FingerprintHasher::new();
        hb.update_unordered_map(&b);

        assert_ne!(ha.finish(), hb.finish());
    }

    #[test]
    fn empty_graph_fingerprint_is_stable() {
        let g1 = CodeGraph::new();
        let g2 = CodeGraph::new();
        assert_eq!(g1.fingerprint(), g2.fingerprint());
    }

    #[test]
    fn fingerprint_changes_when_node_added() {
        let mut g = CodeGraph::new();
        let empty = g.fingerprint();
        g.add_node(make_node("foo", NodeKind::Function));
        assert_ne!(empty, g.fingerprint());
    }

    /// Pin the BLAKE3-derived fingerprint for a fixed input. This is the
    /// whole reason for the SipHash → BLAKE3 migration: the digest must be
    /// the same byte-for-byte on every machine and every Rust version so
    /// indices built on CI can be shipped to developer laptops. If this
    /// test ever starts failing, something perturbed the digest pipeline
    /// (hasher impl, encoding, byte order) and cross-machine cache reuse
    /// will silently break — investigate before updating the constant.
    #[test]
    fn fingerprint_is_cross_machine_stable() {
        let mut h = FingerprintHasher::new();
        h.update(&"hello world");
        assert_eq!(h.finish().as_u64(), 0x33aad8805c476b70);

        // The empty hasher's finalize is also pinned — finalize-on-empty
        // is a well-defined BLAKE3 output, not a per-process seed.
        let empty = FingerprintHasher::new();
        assert_eq!(empty.finish().as_u64(), 0xa6a1f9f5b94913af);
    }
}
