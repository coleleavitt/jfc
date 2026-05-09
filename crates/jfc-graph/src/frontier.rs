//! Hybrid sparse/dense vertex frontier for BFS-style traversals.
//!
//! ## Rationale (Ligra/Yang 2018)
//!
//! Direction-optimised BFS picks **push** when the frontier is small
//! (iterate frontier, expand to neighbours) and **pull** when the
//! frontier is large (iterate every vertex, check if any predecessor
//! is in frontier). The push representation wants a compact
//! list-of-active-vertices; the pull representation wants O(1)
//! membership checks across the whole vertex set. Neither is good at
//! both.
//!
//! `Frontier` switches between two backings dynamically:
//!
//! - `Sparse` — `Vec<u32>` for tiny frontiers (<= 1/16 of n). O(|F|)
//!   iteration, O(|F|) membership (linear scan, but |F| is small).
//! - `Dense` — `RoaringBitmap` for large frontiers. O(n/64) iteration
//!   with bitmask SIMD, O(1) membership.
//!
//! Switching between representations is O(|F|) on a state change. The
//! threshold (`DENSE_THRESHOLD_NUMER` / `DENOM`) is tunable and
//! defaults to 1/16 — the empirical inflection point reported in
//! Yang 2018 for code-shaped graphs.
//!
//! ## Why not just use HashSet<u32> everywhere
//!
//! `HashSet` is allocation-heavy (bucket-based) and has poor spatial
//! locality. The push path (sparse) wants an array we can iterate
//! linearly; the pull path (dense) wants a bitvector we can scan with
//! popcnt. `HashSet` is the worst of both.

use roaring::RoaringBitmap;

/// Density threshold (numerator over `DENOM`) at which the frontier
/// auto-promotes from sparse to dense. `1/16` is the empirical
/// inflection point reported in Yang et al. 2018 for code-shaped
/// graphs; tunable via the constants below.
const DENSE_THRESHOLD_NUMER: usize = 1;
const DENSE_THRESHOLD_DENOM: usize = 16;

/// A vertex set that switches between sparse `Vec<u32>` and dense
/// `RoaringBitmap` representations based on density.
pub struct Frontier {
    n: usize,
    inner: FrontierInner,
}

enum FrontierInner {
    Sparse(Vec<u32>),
    Dense(RoaringBitmap),
}

impl Frontier {
    /// Empty frontier sized for a graph of `n` vertices. Starts sparse.
    pub fn new(n: usize) -> Self {
        Self {
            n,
            inner: FrontierInner::Sparse(Vec::new()),
        }
    }

    /// Frontier with one initial vertex.
    pub fn singleton(n: usize, v: u32) -> Self {
        Self {
            n,
            inner: FrontierInner::Sparse(vec![v]),
        }
    }

    /// True if `v` is in the frontier.
    pub fn contains(&self, v: u32) -> bool {
        match &self.inner {
            FrontierInner::Sparse(s) => s.contains(&v),
            FrontierInner::Dense(b) => b.contains(v),
        }
    }

    /// Insert `v`. May trigger a sparse → dense promotion.
    pub fn insert(&mut self, v: u32) {
        match &mut self.inner {
            FrontierInner::Sparse(s) => {
                if !s.contains(&v) {
                    s.push(v);
                }
            }
            FrontierInner::Dense(b) => {
                b.insert(v);
            }
        }
        self.maybe_promote();
    }

    /// Number of vertices in the frontier.
    pub fn len(&self) -> usize {
        match &self.inner {
            FrontierInner::Sparse(s) => s.len(),
            FrontierInner::Dense(b) => b.len() as usize,
        }
    }

    /// True if frontier has no vertices.
    pub fn is_empty(&self) -> bool {
        match &self.inner {
            FrontierInner::Sparse(s) => s.is_empty(),
            FrontierInner::Dense(b) => b.is_empty(),
        }
    }

    /// True if the frontier is currently dense.
    pub fn is_dense(&self) -> bool {
        matches!(self.inner, FrontierInner::Dense(_))
    }

    /// Iterate the frontier's vertex indices.
    pub fn iter(&self) -> FrontierIter<'_> {
        match &self.inner {
            FrontierInner::Sparse(s) => FrontierIter::Sparse(s.iter()),
            FrontierInner::Dense(b) => FrontierIter::Dense(b.iter()),
        }
    }

    /// Estimate of the total work for a push-based expansion: sum of
    /// frontier degrees. Caller passes a closure to look up degrees.
    pub fn push_workload(&self, mut deg_of: impl FnMut(u32) -> usize) -> usize {
        let mut total = 0usize;
        for v in self.iter() {
            total = total.saturating_add(deg_of(v));
        }
        total
    }

    /// Drop-in clear preserving capacity.
    pub fn clear(&mut self) {
        match &mut self.inner {
            FrontierInner::Sparse(s) => s.clear(),
            FrontierInner::Dense(b) => b.clear(),
        }
    }

    /// Force-promote to dense. Useful for analyses that know they want
    /// dense up front (pull-only workloads).
    pub fn promote_to_dense(&mut self) {
        if let FrontierInner::Sparse(s) = &mut self.inner {
            let mut bm = RoaringBitmap::new();
            for &v in s.iter() {
                bm.insert(v);
            }
            self.inner = FrontierInner::Dense(bm);
        }
    }

    fn maybe_promote(&mut self) {
        // Promote when |F| > n/16 (Ligra inflection point).
        let threshold = (self.n.saturating_mul(DENSE_THRESHOLD_NUMER)) / DENSE_THRESHOLD_DENOM;
        if let FrontierInner::Sparse(s) = &self.inner {
            if s.len() > threshold && self.n > 0 {
                self.promote_to_dense();
            }
        }
    }
}

/// Iterator over a `Frontier`.
pub enum FrontierIter<'a> {
    Sparse(std::slice::Iter<'a, u32>),
    Dense(roaring::bitmap::Iter<'a>),
}

impl Iterator for FrontierIter<'_> {
    type Item = u32;
    fn next(&mut self) -> Option<u32> {
        match self {
            FrontierIter::Sparse(it) => it.next().copied(),
            FrontierIter::Dense(it) => it.next(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_frontier_starts_sparse() {
        let f = Frontier::new(100);
        assert!(f.is_empty());
        assert!(!f.is_dense());
        assert_eq!(f.len(), 0);
    }

    #[test]
    fn singleton_contains() {
        let f = Frontier::singleton(100, 42);
        assert_eq!(f.len(), 1);
        assert!(f.contains(42));
        assert!(!f.contains(0));
    }

    #[test]
    fn insert_idempotent() {
        let mut f = Frontier::new(100);
        f.insert(7);
        f.insert(7);
        assert_eq!(f.len(), 1);
    }

    #[test]
    fn auto_promotes_to_dense_at_threshold() {
        // n=160, threshold = 160/16 = 10. Insert 11 → promote.
        let mut f = Frontier::new(160);
        for i in 0..11u32 {
            f.insert(i);
        }
        assert!(f.is_dense());
        for i in 0..11u32 {
            assert!(f.contains(i));
        }
    }

    #[test]
    fn small_frontier_stays_sparse() {
        let mut f = Frontier::new(10_000);
        for i in 0..5u32 {
            f.insert(i);
        }
        assert!(!f.is_dense());
    }

    #[test]
    fn iter_yields_all_inserted() {
        let mut f = Frontier::new(10);
        f.insert(1);
        f.insert(2);
        f.insert(3);
        let mut out: Vec<u32> = f.iter().collect();
        out.sort();
        assert_eq!(out, vec![1, 2, 3]);
    }

    #[test]
    fn force_promote() {
        let mut f = Frontier::new(1000);
        f.insert(5);
        f.promote_to_dense();
        assert!(f.is_dense());
        assert!(f.contains(5));
    }

    #[test]
    fn clear_preserves_dense_state() {
        let mut f = Frontier::new(10);
        f.promote_to_dense();
        f.insert(3);
        f.clear();
        assert!(f.is_empty());
        assert!(f.is_dense());
    }

    #[test]
    fn push_workload_sums_degrees() {
        let mut f = Frontier::new(100);
        f.insert(1);
        f.insert(2);
        f.insert(3);
        let work = f.push_workload(|v| v as usize * 2);
        assert_eq!(work, 12);
    }

    #[test]
    fn dense_iter_after_promotion() {
        let mut f = Frontier::new(160);
        for i in 0..11u32 {
            f.insert(i);
        }
        let collected: Vec<u32> = f.iter().collect();
        assert_eq!(collected.len(), 11);
    }
}
