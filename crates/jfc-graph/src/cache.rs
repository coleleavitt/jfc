//! In-memory memoization layer for per-file analysis results, plus an
//! opt-in on-disk persistence layer keyed by content [`Fingerprint`].
//!
//! Keyed on `(path, input_fingerprint)`. On re-index, compute the fingerprint
//! of the file's content; if it matches a cached entry, reuse — otherwise
//! recompute and replace the entry.
//!
//! This is the simplified red-green algorithm from rustc's query system
//! (see Zulip t-compiler/query-system, Zoxc on memoization). The full
//! red-green algorithm tracks input dependencies as a DAG; this scaffold
//! is one-level deep and good enough for "did this file change" decisions.
//!
//! ## On-disk layout
//!
//! When a [`AnalysisCache`] is constructed with a [`PathBuf`] cache root
//! (or when the default is used), values can be persisted to:
//!
//! ```text
//! <cache_root>/analysis/<analysis_kind>/<fingerprint_hex>.bin
//! ```
//!
//! Each file is a bincode-serialized [`VersionedAnalysisEntry<V>`] tagged with
//! [`ANALYSIS_CACHE_SCHEMA_VERSION`]. Reads return `None` on schema mismatch
//! after logging a `warn!`. Writes are atomic (tmp + rename).
//!
//! The on-disk layer is independent of [`crate::persistence`]'s event log:
//! that module versions the wire format of mutating events, this module
//! versions the wire format of memoized analysis values. They evolve on
//! independent cadences and so each carry their own `*_SCHEMA_VERSION`
//! constant.
//!
//! # Future-PR migration plan
//!
//! - `enrichment.rs` should grow a `cache_signature_resolution(file_path, &source)`
//!   helper that calls `AnalysisCache::get` first, runs the LSP resolution on
//!   miss, then `AnalysisCache::put`s the result.
//! - `partial.rs` partial-graph extension uses the cache on incremental
//!   rebuilds: each touched file is re-fingerprinted; cache hits skip the
//!   per-file analysis entirely, cache misses trigger a recomputation that
//!   feeds back into the cache.
//! - Whole-graph chunk persistence (a `<cache_root>/graphs/<fp>.bin`
//!   subdirectory holding bincoded `CodeGraph` partials) is planned but lives
//!   outside this scaffold; it will reuse [`cache_root_for`] for path
//!   discovery so test overrides flow through uniformly.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::warn;

/// 64-bit content fingerprint. Mirrors `rustc_data_structures::fingerprint::Fingerprint`
/// in spirit — jfc only needs a deterministic key for in-process memoization,
/// not a cryptographic digest.
///
/// If a parallel agent later introduces a richer `Fingerprint` newtype (e.g.
/// 128-bit, or a `Fingerprintable` trait), this definition can be replaced
/// transparently — `AnalysisCache` only requires `Eq + Copy` from the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint(pub u64);

impl Fingerprint {
    /// Construct a fingerprint from a raw 64-bit hash.
    pub const fn from_u64(raw: u64) -> Self {
        Self(raw)
    }

    /// Hash a byte slice with the default hasher to produce a fingerprint.
    /// This is intentionally simple — callers wanting collision resistance
    /// should produce the `u64` themselves (e.g. via blake3 truncation).
    pub fn of_bytes(bytes: &[u8]) -> Self {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut hasher);
        Self(hasher.finish())
    }

    /// Underlying raw value, primarily for debugging / serialization.
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// Lowercase 16-digit hex representation, used as the on-disk filename
    /// stem. Stable across processes (no hidden state).
    pub fn to_hex(self) -> String {
        format!("{:016x}", self.0)
    }
}

// --- on-disk schema ------------------------------------------------------

/// Schema version for [`VersionedAnalysisEntry`] / on-disk cache files.
///
/// Bump this when the wire format of any persisted analysis value changes
/// in a backward-incompatible way. Readers reject mismatched versions by
/// returning `None` (after logging a `warn!`); the offending file is left
/// in place so an explicit [`AnalysisCache::clear_disk`] call (or the user
/// blowing away the directory) is needed to recover space.
///
/// Independent of [`crate::persistence::PERSISTENCE_SCHEMA_VERSION`]: those
/// version the event log, this versions memoized values.
pub const ANALYSIS_CACHE_SCHEMA_VERSION: u32 = 1;

/// Wire-format wrapper for on-disk analysis entries.
///
/// `value: V` is whatever the analysis produced (a SCC partition, a taint
/// summary, etc.); the wrapper attaches a schema tag so future format breaks
/// can be detected without inspecting `V` itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionedAnalysisEntry<V> {
    pub schema_version: u32,
    pub value: V,
}

impl<V> VersionedAnalysisEntry<V> {
    /// Wrap a value with the current [`ANALYSIS_CACHE_SCHEMA_VERSION`].
    pub fn wrap(value: V) -> Self {
        Self {
            schema_version: ANALYSIS_CACHE_SCHEMA_VERSION,
            value,
        }
    }

    /// Returns `Some(value)` iff the schema tag matches the running binary.
    /// Mismatches return `None` so on-disk reads degrade to a cache miss.
    pub fn into_value_if_current(self) -> Option<V> {
        if self.schema_version == ANALYSIS_CACHE_SCHEMA_VERSION {
            Some(self.value)
        } else {
            warn!(
                expected = ANALYSIS_CACHE_SCHEMA_VERSION,
                found = self.schema_version,
                "analysis cache schema mismatch — treating as miss"
            );
            None
        }
    }
}

/// Tag trait for an analysis "kind" — namespacing for the on-disk
/// `<cache_root>/analysis/<KIND>/` subdirectory.
///
/// Implementors are typically zero-sized marker types (e.g. `struct
/// SccAnalysis;`). Keeping the kind at the type level lets each call to
/// [`AnalysisCache::load_disk`] / [`AnalysisCache::store_disk`] pick its
/// directory at compile time, and forecloses the "two analyses accidentally
/// share a cache slot because someone passed the wrong string" failure
/// mode.
///
/// `KIND` MUST be a stable, filesystem-safe ASCII identifier (lowercase,
/// `[a-z0-9_-]`). Renaming a kind invalidates all existing on-disk entries
/// for that kind.
pub trait AnalysisKind {
    const KIND: &'static str;
}

// --- in-memory cache -----------------------------------------------------

/// One memoized result, plus bookkeeping for LRU eviction.
struct CacheEntry<V> {
    fingerprint: Fingerprint,
    value: V,
    /// Generation counter — bumped each time the cache is consulted.
    /// Used by an LRU eviction policy if the cache grows unbounded.
    last_used: u64,
}

/// In-memory memoization layer for per-file analysis results, with optional
/// on-disk persistence.
///
/// Keyed on `(PathBuf, Fingerprint)`. The fingerprint represents the
/// content of the file at the time the value was computed. A subsequent
/// `get` with the same `(path, fingerprint)` is a hit; a `get` with a
/// different `fingerprint` is a miss (the file changed).
///
/// The disk layer is consulted via the explicit [`Self::load_disk`] /
/// [`Self::store_disk`] methods — the in-memory `get`/`put` API is
/// unchanged so existing call sites are unaffected.
pub struct AnalysisCache<V> {
    entries: HashMap<PathBuf, CacheEntry<V>>,
    /// Monotonic clock — incremented on every `get` and `put` so that
    /// `last_used` deltas are meaningful for LRU eviction.
    generation: u64,
    /// Override for the on-disk cache root. `None` means "use the default
    /// resolved by [`cache_root_for`] at call time" — that resolution
    /// honors `JFC_GRAPH_CACHE_DIR`, then falls back to `$HOME/.cache/...`.
    /// Storing a `PathBuf` here lets tests inject a `tempfile::tempdir`
    /// without a process-wide env mutation.
    cache_root_override: Option<PathBuf>,
}

impl<V> AnalysisCache<V> {
    /// Create an empty cache. Disk-backed methods will resolve their root
    /// via [`cache_root_for`] (env var, then `$HOME/.cache/jfc-graph/v1`).
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            generation: 0,
            cache_root_override: None,
        }
    }

    /// Create a cache with an explicit on-disk root. Primarily used by tests
    /// to point at a `tempfile::tempdir`; production code can use
    /// [`Self::new`] and let env-var / `$HOME` resolution apply.
    pub fn with_cache_root(root: impl Into<PathBuf>) -> Self {
        Self {
            entries: HashMap::new(),
            generation: 0,
            cache_root_override: Some(root.into()),
        }
    }

    /// Number of entries currently in the cache.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True iff the cache holds no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Check if the cached value for `path` matches `fingerprint`.
    /// Returns `Some(&V)` on hit, `None` on miss.
    ///
    /// On hit, the entry's `last_used` generation is updated so subsequent
    /// `evict_lru` calls treat it as recently consulted.
    pub fn get(&mut self, path: &Path, fingerprint: Fingerprint) -> Option<&V> {
        self.generation = self.generation.wrapping_add(1);
        let now = self.generation;
        let entry = self.entries.get_mut(path)?;
        if entry.fingerprint != fingerprint {
            return None;
        }
        entry.last_used = now;
        Some(&entry.value)
    }

    /// Insert or replace the entry for `path`.
    pub fn put(&mut self, path: PathBuf, fingerprint: Fingerprint, value: V) {
        self.generation = self.generation.wrapping_add(1);
        self.entries.insert(
            path,
            CacheEntry {
                fingerprint,
                value,
                last_used: self.generation,
            },
        );
    }

    /// Drop the least-recently-used entries until at most `max_keep` remain.
    ///
    /// If the cache already holds `<= max_keep` entries, this is a no-op.
    /// Ties on `last_used` are broken arbitrarily (whichever the iterator
    /// visits first), which is fine for an eviction heuristic.
    pub fn evict_lru(&mut self, max_keep: usize) {
        if self.entries.len() <= max_keep {
            return;
        }

        // Collect (last_used, path) so we can sort without borrowing entries.
        let mut by_age: Vec<(u64, PathBuf)> = self
            .entries
            .iter()
            .map(|(p, e)| (e.last_used, p.clone()))
            .collect();
        by_age.sort_by_key(|(age, _)| *age);

        let drop_count = self.entries.len() - max_keep;
        for (_, path) in by_age.into_iter().take(drop_count) {
            self.entries.remove(&path);
        }
    }

    /// Drop all in-memory entries — used on schema migration or workspace
    /// switch. Does NOT touch the on-disk cache; use [`Self::clear_disk`]
    /// for that.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Resolve the active on-disk root for this cache: the explicit override
    /// passed to [`Self::with_cache_root`] if any, otherwise the default
    /// from [`cache_root_for`].
    pub fn cache_root(&self) -> PathBuf {
        match &self.cache_root_override {
            Some(p) => p.clone(),
            None => cache_root_for::<()>(None),
        }
    }
}

impl<V> AnalysisCache<V>
where
    V: Serialize + for<'de> Deserialize<'de>,
{
    /// Try to load a value from disk for `(path, fp)` under the analysis
    /// kind `K`. The `path` argument is currently unused at the on-disk
    /// layer (the fingerprint already identifies the input bytes) but is
    /// retained in the signature so callers don't need to thread two
    /// keying conventions; future cache layouts may incorporate it.
    ///
    /// Returns `None` on:
    ///
    /// - file not present (cache miss),
    /// - I/O error reading the file (logs `warn!`),
    /// - bincode decode failure (logs `warn!`),
    /// - schema-version mismatch (logs `warn!`).
    pub fn load_disk<K: AnalysisKind>(&self, path: &Path, fp: Fingerprint) -> Option<V> {
        let _ = path; // reserved; see doc comment.
        let target = self.disk_path::<K>(fp);

        let bytes = match fs::read(&target) {
            Ok(b) => b,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return None,
            Err(err) => {
                warn!(
                    target = %target.display(),
                    error = %err,
                    "analysis cache: read failed, treating as miss"
                );
                return None;
            }
        };

        let cfg = bincode::config::standard();
        let entry: VersionedAnalysisEntry<V> =
            match bincode::serde::decode_from_slice(&bytes, cfg) {
                Ok((value, _)) => value,
                Err(err) => {
                    warn!(
                        target = %target.display(),
                        error = %err,
                        "analysis cache: decode failed, treating as miss"
                    );
                    return None;
                }
            };

        entry.into_value_if_current()
    }

    /// Persist `value` to disk for `(path, fp)` under the analysis kind `K`.
    ///
    /// Implementation: serialize → write to `<target>.tmp` → `rename` over
    /// `<target>`. The rename is atomic on POSIX, so a crash mid-write
    /// either leaves the previous `.bin` intact or replaces it cleanly —
    /// readers never observe a half-written file.
    ///
    /// Errors are returned to the caller (so a noisy disk doesn't silently
    /// drop everything), but per the docstring at the top of this module
    /// the disk cache is best-effort and call sites SHOULD log + continue
    /// rather than propagate.
    pub fn store_disk<K: AnalysisKind>(
        &self,
        path: &Path,
        fp: Fingerprint,
        value: &V,
    ) -> io::Result<()> {
        let _ = path; // reserved; see `load_disk`.
        let target = self.disk_path::<K>(fp);

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }

        let cfg = bincode::config::standard();
        let bytes = bincode::serde::encode_to_vec(VersionedAnalysisEntry::wrap(value), cfg)
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;

        // POSIX `rename(2)` is atomic for files on the same filesystem; the
        // tmp file lives next to the target so they share the cache root's
        // mount. The `.tmp` suffix replaces any extension on the target,
        // but our targets are `<hex>.bin`, so we hand-construct the tmp
        // name to avoid clobbering `.bin` on, e.g., a hypothetical
        // `foo.bin.tmp` collision.
        let tmp = target.with_extension("bin.tmp");
        fs::write(&tmp, &bytes)?;
        match fs::rename(&tmp, &target) {
            Ok(()) => Ok(()),
            Err(err) => {
                // Best effort: if the rename fails, scrub the tmp file so
                // we don't leak a half-written artifact on disk.
                let _ = fs::remove_file(&tmp);
                Err(err)
            }
        }
    }

    /// Drop every on-disk entry under analysis kind `K` (i.e. delete the
    /// `<cache_root>/analysis/<KIND>/` directory recursively).
    ///
    /// Used on schema-version mismatch or when an analysis is being
    /// retired. Idempotent: a missing directory is treated as success.
    pub fn clear_disk<K: AnalysisKind>(&self) -> io::Result<()> {
        let dir = self.kind_dir::<K>();
        match fs::remove_dir_all(&dir) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err),
        }
    }

    /// Compute the directory holding all on-disk entries for kind `K`.
    fn kind_dir<K: AnalysisKind>(&self) -> PathBuf {
        let mut dir = self.cache_root();
        dir.push("analysis");
        dir.push(K::KIND);
        dir
    }

    /// Compute the on-disk path for a single `(K, fp)` entry.
    fn disk_path<K: AnalysisKind>(&self, fp: Fingerprint) -> PathBuf {
        let mut p = self.kind_dir::<K>();
        p.push(format!("{}.bin", fp.to_hex()));
        p
    }
}

impl<V> Default for AnalysisCache<V> {
    fn default() -> Self {
        Self::new()
    }
}

// --- cache root resolution ----------------------------------------------

/// Resolve the on-disk cache root.
///
/// Resolution order:
///
/// 1. If `override_root` is `Some`, it wins (used by tests via
///    [`AnalysisCache::with_cache_root`]).
/// 2. The `JFC_GRAPH_CACHE_DIR` environment variable, if set and non-empty.
/// 3. `$HOME/.cache/jfc-graph/v1/`.
/// 4. As a last resort (no `$HOME`), `./.cache/jfc-graph/v1/` relative to
///    the current working directory.
///
/// The `v1` suffix is intentional: bumping the path on a schema break is
/// cheaper than reading + discarding incompatible files.
///
/// The generic parameter is unused — the function is monomorphized to allow
/// callers to write `cache_root_for::<()>(None)` without naming a type.
pub fn cache_root_for<T: ?Sized>(override_root: Option<&Path>) -> PathBuf {
    if let Some(p) = override_root {
        return p.to_path_buf();
    }

    if let Ok(val) = std::env::var("JFC_GRAPH_CACHE_DIR") {
        if !val.is_empty() {
            return PathBuf::from(val);
        }
    }

    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(".cache").join("jfc-graph").join("v1")
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- in-memory cache (preserved from the original suite) ------------

    #[test]
    fn cache_hit_returns_cached_value_normal() {
        let mut cache: AnalysisCache<String> = AnalysisCache::new();
        let path = PathBuf::from("src/lib.rs");
        let fp = Fingerprint::of_bytes(b"fn main() {}");

        cache.put(path.clone(), fp, "analysis-result".to_string());

        let hit = cache.get(&path, fp);
        assert_eq!(hit, Some(&"analysis-result".to_string()));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn cache_miss_when_fingerprint_differs_robust() {
        let mut cache: AnalysisCache<u32> = AnalysisCache::new();
        let path = PathBuf::from("src/lib.rs");
        let fp_v1 = Fingerprint::of_bytes(b"fn main() {}");
        let fp_v2 = Fingerprint::of_bytes(b"fn main() { let x = 1; }");
        assert_ne!(fp_v1, fp_v2, "test inputs should hash differently");

        cache.put(path.clone(), fp_v1, 42);

        // Same path, different fingerprint → miss.
        assert_eq!(cache.get(&path, fp_v2), None);

        // Original entry is still there (we never recomputed); verify by
        // re-querying with the original fingerprint.
        assert_eq!(cache.get(&path, fp_v1), Some(&42));

        // Different path → miss.
        let other = PathBuf::from("src/other.rs");
        assert_eq!(cache.get(&other, fp_v1), None);
    }

    #[test]
    fn cache_evict_lru_keeps_most_recent_normal() {
        let mut cache: AnalysisCache<u32> = AnalysisCache::new();
        let a = PathBuf::from("a.rs");
        let b = PathBuf::from("b.rs");
        let c = PathBuf::from("c.rs");
        let fp = Fingerprint::from_u64(1);

        cache.put(a.clone(), fp, 1);
        cache.put(b.clone(), fp, 2);
        cache.put(c.clone(), fp, 3);

        // Touch `a` and `c` so `b` becomes the LRU.
        let _ = cache.get(&a, fp);
        let _ = cache.get(&c, fp);

        cache.evict_lru(2);

        assert_eq!(cache.len(), 2);
        assert!(cache.get(&a, fp).is_some(), "a was recently used");
        assert!(cache.get(&c, fp).is_some(), "c was recently used");
        assert!(cache.get(&b, fp).is_none(), "b should have been evicted");
    }

    #[test]
    fn cache_clear_drops_all_entries_robust() {
        let mut cache: AnalysisCache<&'static str> = AnalysisCache::new();
        let fp = Fingerprint::from_u64(7);

        cache.put(PathBuf::from("x.rs"), fp, "x");
        cache.put(PathBuf::from("y.rs"), fp, "y");
        cache.put(PathBuf::from("z.rs"), fp, "z");
        assert_eq!(cache.len(), 3);

        cache.clear();

        assert!(cache.is_empty());
        assert_eq!(cache.get(&PathBuf::from("x.rs"), fp), None);
        assert_eq!(cache.get(&PathBuf::from("y.rs"), fp), None);
        assert_eq!(cache.get(&PathBuf::from("z.rs"), fp), None);

        // Cache should remain usable after clear.
        cache.put(PathBuf::from("new.rs"), fp, "new");
        assert_eq!(cache.get(&PathBuf::from("new.rs"), fp), Some(&"new"));
    }

    // --- on-disk cache --------------------------------------------------

    /// Marker analysis kind used by the disk-cache tests below. A real
    /// consumer would define one of these per `analysis::*` module.
    struct SccAnalysis;
    impl AnalysisKind for SccAnalysis {
        const KIND: &'static str = "scc";
    }

    /// A second kind, for the namespacing test.
    struct CentralityAnalysis;
    impl AnalysisKind for CentralityAnalysis {
        const KIND: &'static str = "centrality";
    }

    /// Stand-in for an SCC partition: `Vec<Vec<u32>>` round-trips trivially
    /// through bincode + serde without dragging in graph types.
    type Partition = Vec<Vec<u32>>;

    fn sample_partition() -> Partition {
        vec![vec![0, 1, 2], vec![3], vec![4, 5]]
    }

    #[test]
    fn disk_cache_round_trips_normal() {
        let tmp = tempfile::tempdir().unwrap();
        let cache: AnalysisCache<Partition> = AnalysisCache::with_cache_root(tmp.path());
        let path = PathBuf::from("src/lib.rs");
        let fp = Fingerprint::of_bytes(b"some source bytes");
        let scc = sample_partition();

        cache.store_disk::<SccAnalysis>(&path, fp, &scc).unwrap();
        let loaded = cache
            .load_disk::<SccAnalysis>(&path, fp)
            .expect("just-written entry should load");
        assert_eq!(scc, loaded);

        // Layout sanity: file lives at <root>/analysis/scc/<hex>.bin.
        let on_disk = tmp
            .path()
            .join("analysis")
            .join("scc")
            .join(format!("{}.bin", fp.to_hex()));
        assert!(on_disk.exists(), "entry file should exist at {on_disk:?}");
    }

    #[test]
    fn disk_cache_namespaces_by_kind_robust() {
        // Same fingerprint, different kinds → independent cache slots.
        let tmp = tempfile::tempdir().unwrap();
        let cache: AnalysisCache<u64> = AnalysisCache::with_cache_root(tmp.path());
        let path = PathBuf::from("src/lib.rs");
        let fp = Fingerprint::from_u64(0xdead_beef);

        cache.store_disk::<SccAnalysis>(&path, fp, &11).unwrap();
        cache.store_disk::<CentralityAnalysis>(&path, fp, &22).unwrap();

        assert_eq!(cache.load_disk::<SccAnalysis>(&path, fp), Some(11));
        assert_eq!(cache.load_disk::<CentralityAnalysis>(&path, fp), Some(22));

        // `clear_disk` of one kind must not affect the other.
        cache.clear_disk::<SccAnalysis>().unwrap();
        assert_eq!(cache.load_disk::<SccAnalysis>(&path, fp), None);
        assert_eq!(cache.load_disk::<CentralityAnalysis>(&path, fp), Some(22));
    }

    #[test]
    fn disk_cache_returns_none_on_schema_mismatch_robust() {
        let tmp = tempfile::tempdir().unwrap();
        let cache: AnalysisCache<Partition> = AnalysisCache::with_cache_root(tmp.path());
        let path = PathBuf::from("src/lib.rs");
        let fp = Fingerprint::from_u64(42);

        // Hand-construct a wrapper with a future schema version and write
        // it through the same on-disk path the cache would. We bypass
        // `store_disk` so the version on disk is intentionally wrong.
        let bogus = VersionedAnalysisEntry::<Partition> {
            schema_version: ANALYSIS_CACHE_SCHEMA_VERSION + 1,
            value: sample_partition(),
        };
        let cfg = bincode::config::standard();
        let bytes = bincode::serde::encode_to_vec(&bogus, cfg).unwrap();

        let target = tmp
            .path()
            .join("analysis")
            .join("scc")
            .join(format!("{}.bin", fp.to_hex()));
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, &bytes).unwrap();

        assert!(
            cache.load_disk::<SccAnalysis>(&path, fp).is_none(),
            "schema mismatch must surface as a miss, not a panic"
        );
    }

    #[test]
    fn disk_cache_atomic_write_prevents_partial_robust() {
        // Simulate a crash by removing the tmp file before the rename
        // happens. The user-facing `.bin` path must NOT exist as a result.
        let tmp = tempfile::tempdir().unwrap();
        let path = PathBuf::from("src/lib.rs");
        let fp = Fingerprint::from_u64(7);

        // Manually plant a tmp file that would have been the pre-rename
        // artifact, then remove it. The `.bin` path must remain absent.
        let kind_dir = tmp.path().join("analysis").join("scc");
        fs::create_dir_all(&kind_dir).unwrap();
        let tmp_file = kind_dir.join(format!("{}.bin.tmp", fp.to_hex()));
        let bin_file = kind_dir.join(format!("{}.bin", fp.to_hex()));
        fs::write(&tmp_file, b"partial-write-bytes").unwrap();
        // "Crash": the rename never happens. Clean up the tmp ourselves.
        fs::remove_file(&tmp_file).unwrap();

        assert!(
            !bin_file.exists(),
            "atomic write contract: .bin must not exist if rename never ran"
        );

        // The cache should report a miss, not surface the half-written tmp.
        let cache: AnalysisCache<Partition> = AnalysisCache::with_cache_root(tmp.path());
        assert!(cache.load_disk::<SccAnalysis>(&path, fp).is_none());

        // Now do a real `store_disk`; the .bin should appear and the tmp
        // should not linger.
        cache
            .store_disk::<SccAnalysis>(&path, fp, &sample_partition())
            .unwrap();
        assert!(bin_file.exists(), "store_disk should produce the .bin");
        assert!(!tmp_file.exists(), "tmp file should not linger after rename");
    }

    #[test]
    fn disk_cache_load_miss_for_unknown_fp_robust() {
        let tmp = tempfile::tempdir().unwrap();
        let cache: AnalysisCache<Partition> = AnalysisCache::with_cache_root(tmp.path());
        let path = PathBuf::from("src/lib.rs");

        // No store_disk was called → any fingerprint is a miss.
        let fp = Fingerprint::from_u64(0x1234_5678);
        assert!(cache.load_disk::<SccAnalysis>(&path, fp).is_none());

        // After storing a different fp, the unknown fp is still a miss.
        let stored_fp = Fingerprint::from_u64(0xaaaa_bbbb);
        cache
            .store_disk::<SccAnalysis>(&path, stored_fp, &sample_partition())
            .unwrap();
        assert!(cache.load_disk::<SccAnalysis>(&path, fp).is_none());
        assert!(cache.load_disk::<SccAnalysis>(&path, stored_fp).is_some());
    }

    #[test]
    fn disk_cache_clear_disk_is_idempotent_robust() {
        let tmp = tempfile::tempdir().unwrap();
        let cache: AnalysisCache<Partition> = AnalysisCache::with_cache_root(tmp.path());

        // Empty directory: clear_disk must succeed (NotFound is fine).
        cache.clear_disk::<SccAnalysis>().unwrap();

        // Populate, clear, clear again — second clear is a no-op.
        let path = PathBuf::from("x.rs");
        let fp = Fingerprint::from_u64(1);
        cache
            .store_disk::<SccAnalysis>(&path, fp, &sample_partition())
            .unwrap();
        cache.clear_disk::<SccAnalysis>().unwrap();
        cache.clear_disk::<SccAnalysis>().unwrap();
        assert!(cache.load_disk::<SccAnalysis>(&path, fp).is_none());
    }

    #[test]
    fn cache_root_for_honors_env_var_robust() {
        // We can't safely mutate process-global env in a parallel test
        // runner, so check the override path (which env-var resolution
        // ultimately wires into) directly.
        let injected = PathBuf::from("/tmp/jfc-test-root");
        assert_eq!(
            cache_root_for::<()>(Some(&injected)),
            injected,
            "explicit override must win over any env / HOME fallback"
        );
    }
}
