//! Compression transforms — Rust ports of `headroom.transforms.*`
//! (Apache-2.0), trimmed to the deterministic subset jfc uses: the
//! adaptive sizer (scoring core), the line-importance-aware log / search /
//! diff compressors, the content-type detector that routes to them, the
//! tag protector that shields workflow markers, and the tool-pair safety
//! rules.
//!
//! The heavy ML transforms (smart_crusher JSON, magika/unidiff ONNX
//! detection, the live-zone proxy dispatcher) are intentionally NOT
//! ported — they pull tokenizers/ONNX runtimes jfc does not want as an
//! always-on dependency.

pub mod adaptive_sizer;
pub mod content_detector;
pub mod diff_compressor;
pub mod log_compressor;
pub mod safety;
pub mod search_compressor;
pub mod tag_protector;

pub use content_detector::{
    ContentType, DetectionResult, detect_content_type, is_json_array_of_dicts,
};
pub use diff_compressor::{
    DiffCompressionResult, DiffCompressor, DiffCompressorConfig, DiffCompressorStats,
};
pub use log_compressor::{
    LogCompressionResult, LogCompressor, LogCompressorConfig, LogCompressorStats, LogFormat,
    LogLevel, LogLine,
};
pub use safety::{ToolPair, tool_pair_indices};
pub use search_compressor::{
    FileMatches, SearchCompressionResult, SearchCompressor, SearchCompressorConfig,
    SearchCompressorStats, SearchMatch,
};
pub use tag_protector::{ProtectStats, is_known_html_tag, protect_tags, restore_tags};
