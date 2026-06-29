//! CCR (Compress-Cache-Retrieve) storage layer.
//!
//! When a transform compresses data with row-drop or opaque-string
//! substitution, the *original payload* is stashed here keyed by the
//! hash that ends up in the prompt. The runtime later honors retrieval
//! tool calls by looking up the hash in this store and serving back the
//! original. This is the cornerstone of CCR: lossy on the wire, lossless
//! end-to-end.
//!
//! Ported from headroom-core (`crates/headroom-core/src/ccr/`, Apache-2.0)
//! and trimmed to the in-memory backend — the persistent SQLite/Redis
//! backends and the blake3 `compute_key`/proxy-marker helpers are dropped
//! because jfc only needs an in-process put/get for the duration of a
//! session. The compressors key their stored originals with their own
//! `md5_hex_24` (parity with the Python implementation), so the store
//! never needs to compute keys itself.

pub mod backends;

use std::time::Duration;

pub use backends::InMemoryCcrStore;

/// Pluggable CCR storage backend. `Send + Sync` so it can sit behind an
/// `Arc` and be shared across threads.
pub trait CcrStore: Send + Sync {
    /// Stash `payload` under `hash`. If the hash already exists, the
    /// new payload overwrites — same hash should mean same content, so
    /// re-storing is idempotent.
    fn put(&self, hash: &str, payload: &str);

    /// Look up `hash`. Returns `None` if missing or expired.
    fn get(&self, hash: &str) -> Option<String>;

    /// Number of live entries. Informational; used by tests + telemetry.
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Default capacity — matches Python's `CompressionStore` default.
pub const DEFAULT_CAPACITY: usize = 1000;

/// Default TTL — 5 minutes, matching Python.
pub const DEFAULT_TTL: Duration = Duration::from_secs(300);
