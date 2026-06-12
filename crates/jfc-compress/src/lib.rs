//! jfc-compress — content-aware context compression for jfc.
//!
//! Ported from headroom-core (https://github.com/chopratejas/headroom,
//! Apache-2.0). jfc's context compaction previously truncated large tool
//! results to a blind head + tail window, which can elide a critical
//! error line sitting in the middle of a 10k-line build log. This crate
//! replaces that with importance-ranked, content-type-aware compression:
//!
//! - [`transforms::adaptive_sizer`] — information-theoretic "how many
//!   lines to keep" (simhash dedup → bigram-knee → zlib validation).
//! - [`signals`] — per-line importance scoring (Error/Warn/Security/…).
//! - [`transforms::log_compressor`] — build/test output (10-50×).
//! - [`transforms::search_compressor`] — grep/ripgrep output (5-10×).
//! - [`transforms::diff_compressor`] — unified diffs.
//! - [`transforms::content_detector`] — routes a blob to the right
//!   compressor.
//! - [`transforms::tag_protector`] — shields `<system-reminder>`-style
//!   workflow markers from being stripped as noise.
//! - [`volatile`] — flags timestamps/UUIDs/trace-ids that silently bust
//!   provider prompt-cache hits.
//! - [`ccr`] — in-memory original-payload store for reversible retrieval.

pub mod ccr;
pub mod content_aware;
pub mod signals;
pub mod transforms;
pub mod volatile;

pub use ccr::{CcrStore, InMemoryCcrStore};
pub use content_aware::{CompressionMethod, CompressionOutput, compress_tool_output};
pub use transforms::{
    ContentType, DetectionResult, DiffCompressor, DiffCompressorConfig, LogCompressor,
    LogCompressorConfig, SearchCompressor, SearchCompressorConfig, detect_content_type,
};
