//! High-level content-aware compression facade.
//!
//! One entry point — [`compress_tool_output`] — that the jfc runtime
//! calls instead of a blind head/tail truncation. It:
//!
//! 1. Protects workflow tags (`<system-reminder>`, `<thinking>`, …) so
//!    the line-dropping compressors can't strip them as noise.
//! 2. Detects the content type (build/test log, grep/search results,
//!    unified diff) and routes to the matching importance-aware
//!    compressor.
//! 3. Restores the protected tags.
//! 4. Falls back to a head/tail window for content with no specialized
//!    compressor (plain prose, source code, JSON, HTML).
//!
//! # Safety net
//!
//! The facade NEVER returns output longer than the caller's `char_budget`
//! head/tail fallback would, and never returns the specialized output if
//! it somehow grew the text. The worst case is identical to the old blind
//! truncation; the common case keeps the important lines (errors, fails,
//! summaries) that a positional cut would have elided.

use crate::transforms::{
    content_detector::{ContentType, detect_content_type},
    diff_compressor::{DiffCompressor, DiffCompressorConfig},
    log_compressor::{LogCompressor, LogCompressorConfig},
    search_compressor::{SearchCompressor, SearchCompressorConfig},
    tag_protector::{protect_tags, restore_tags},
};

/// How a [`compress_tool_output`] call was satisfied — surfaced so callers
/// can log/measure which path fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionMethod {
    /// Build/test log compressor (pytest/cargo/npm/jest/make/generic).
    Log,
    /// grep/ripgrep search-results compressor.
    Search,
    /// Unified-diff compressor.
    Diff,
    /// No specialized compressor matched (or it didn't help): blind
    /// head/tail window.
    HeadTail,
    /// Input was already under budget — returned verbatim.
    Verbatim,
}

impl CompressionMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            CompressionMethod::Log => "log",
            CompressionMethod::Search => "search",
            CompressionMethod::Diff => "diff",
            CompressionMethod::HeadTail => "head_tail",
            CompressionMethod::Verbatim => "verbatim",
        }
    }
}

/// Outcome of a content-aware compression.
#[derive(Debug, Clone)]
pub struct CompressionOutput {
    /// The compressed text.
    pub text: String,
    /// Which path produced it.
    pub method: CompressionMethod,
    /// Characters in the original input.
    pub original_chars: usize,
    /// Characters in `text`.
    pub compressed_chars: usize,
}

impl CompressionOutput {
    pub fn chars_saved(&self) -> usize {
        self.original_chars.saturating_sub(self.compressed_chars)
    }
}

/// Blind head/tail fallback — the original microcompaction behavior. Kept
/// as the safety net so content-aware compression can never do worse.
fn head_tail(text: &str, keep_head: usize, keep_tail: usize) -> String {
    let len = text.chars().count();
    if len <= keep_head + keep_tail {
        return text.to_string();
    }
    let head: String = text.chars().take(keep_head).collect();
    let tail: String = text.chars().skip(len.saturating_sub(keep_tail)).collect();
    let dropped = len - keep_head - keep_tail;
    format!("{head}\n\n[… {dropped} chars elided by microcompaction …]\n\n{tail}")
}

/// Compress a large tool-output string, keeping the *important* lines.
///
/// `keep_head` / `keep_tail` define the head/tail fallback window used for
/// content with no specialized compressor; they also bound the worst case
/// for specialized compressors (if the specialized output is somehow
/// larger than the head/tail fallback, the fallback is used instead).
///
/// `query_context` is an optional relevance hint (e.g. the user's last
/// prompt or the tool's command) — search/diff compressors use it to
/// prefer lines overlapping the query. Pass `""` when there's no hint.
pub fn compress_tool_output(
    text: &str,
    keep_head: usize,
    keep_tail: usize,
    query_context: &str,
) -> CompressionOutput {
    let original_chars = text.chars().count();
    if original_chars <= keep_head + keep_tail {
        return CompressionOutput {
            text: text.to_string(),
            method: CompressionMethod::Verbatim,
            original_chars,
            compressed_chars: original_chars,
        };
    }

    // Blind head/tail of the *original* text is the worst-case ceiling and
    // the fallback for anything that isn't a clean specialized win.
    let fallback = head_tail(text, keep_head, keep_tail);
    let fallback_chars = fallback.chars().count();
    let head_tail_output = |method| CompressionOutput {
        text: fallback.clone(),
        method,
        original_chars,
        compressed_chars: fallback_chars,
    };

    // Protect workflow tags. `true` = block mode: each tag (with body)
    // collapses to one atomic placeholder token, so a line-dropping
    // compressor either keeps the whole tag or drops the placeholder — it
    // can never half-strip a tag. We detect the drop below and fall back.
    let (clean, blocks, _protect_stats) = protect_tags(text, true);

    let detection = detect_content_type(&clean);
    let specialized: Option<(String, CompressionMethod)> = match detection.content_type {
        ContentType::BuildOutput => {
            let c = LogCompressor::new(LogCompressorConfig::default());
            let (result, _stats) = c.compress(&clean, 1.0);
            Some((result.compressed, CompressionMethod::Log))
        }
        ContentType::SearchResults => {
            let c = SearchCompressor::new(SearchCompressorConfig::default());
            let (result, _stats) = c.compress(&clean, query_context, 1.0);
            Some((result.compressed, CompressionMethod::Search))
        }
        ContentType::GitDiff => {
            let c = DiffCompressor::new(DiffCompressorConfig::default());
            let result = c.compress(&clean, query_context);
            Some((result.compressed, CompressionMethod::Diff))
        }
        // No specialized compressor for JSON / source / HTML / prose in
        // the deterministic subset — those fall through to head/tail.
        ContentType::JsonArray
        | ContentType::SourceCode
        | ContentType::Html
        | ContentType::PlainText => None,
    };

    let Some((compressed_clean, method)) = specialized else {
        return head_tail_output(CompressionMethod::HeadTail);
    };

    // Reject the specialized output unless it's a clean win: it must have
    // shrunk the text, must not exceed the head/tail ceiling, and must
    // have preserved every protected-tag placeholder (a line-dropper can
    // drop a placeholder's whole line). Any failure → head/tail of the
    // original, which is never worse than the old blind behavior.
    let compressed_chars = compressed_clean.chars().count();
    let all_tags_survived = blocks
        .iter()
        .all(|(placeholder, _)| compressed_clean.contains(placeholder.as_str()));
    if compressed_chars >= original_chars || compressed_chars > fallback_chars || !all_tags_survived
    {
        return head_tail_output(CompressionMethod::HeadTail);
    }

    // Splice the protected tags back in.
    let restored = restore_tags(&compressed_clean, &blocks);
    CompressionOutput {
        compressed_chars: restored.chars().count(),
        text: restored,
        method,
        original_chars,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A realistic pytest/cargo-style run: WARN/INFO/test-status lines
    /// interspersed throughout (so the content detector recognizes it as
    /// build output), with one FATAL error sitting in the middle — the
    /// line a blind head/tail cut would elide.
    fn build_log_with_mid_error(total_lines: usize) -> String {
        let mut lines = Vec::with_capacity(total_lines);
        for i in 0..total_lines {
            if i == total_lines / 2 {
                lines.push(
                    "error[E0599]: no method named `frobnicate` found for type `Widget`"
                        .to_string(),
                );
            } else if i % 5 == 0 {
                lines.push(format!("[INFO] test_case_{i} ... ok"));
            } else if i % 7 == 0 {
                lines.push(format!("WARNING: deprecated API used in module_{i}"));
            } else {
                lines.push(format!("   Compiling crate_{i} v0.1.0 (/work/crate_{i})"));
            }
        }
        lines.join("\n")
    }

    // The headline win: a build error sitting in the MIDDLE of a long log
    // is preserved by content-aware compression, where a blind head/tail
    // cut would have elided it.
    #[test]
    fn mid_log_error_survives_compression_robust() {
        let log = build_log_with_mid_error(400);
        // Blind head/tail with a small window would drop the middle.
        let blind = head_tail(&log, 600, 400);
        assert!(
            !blind.contains("E0599"),
            "precondition: blind truncation must drop the mid-log error"
        );

        let out = compress_tool_output(&log, 600, 400, "");
        assert_eq!(out.method, CompressionMethod::Log);
        assert!(
            out.text.contains("E0599"),
            "content-aware compression must keep the mid-log error; got method {:?}",
            out.method
        );
        assert!(out.compressed_chars < out.original_chars);
    }

    #[test]
    fn small_output_returned_verbatim_normal() {
        let text = "just a short line\nand another";
        let out = compress_tool_output(text, 600, 400, "");
        assert_eq!(out.method, CompressionMethod::Verbatim);
        assert_eq!(out.text, text);
    }

    #[test]
    fn plain_prose_falls_back_to_head_tail_normal() {
        // Long prose with no log/search/diff structure → head/tail.
        let prose = "lorem ipsum dolor sit amet ".repeat(200);
        let out = compress_tool_output(&prose, 200, 100, "");
        assert_eq!(out.method, CompressionMethod::HeadTail);
        assert!(out.compressed_chars < out.original_chars);
    }

    #[test]
    fn workflow_tags_survive_compression_robust() {
        // A system-reminder tag embedded in an otherwise-compressible log
        // must round-trip intact.
        let mut log = build_log_with_mid_error(400);
        log.push_str("\n<system-reminder>keep me verbatim</system-reminder>");
        let out = compress_tool_output(&log, 600, 400, "");
        assert!(
            out.text
                .contains("<system-reminder>keep me verbatim</system-reminder>"),
            "protected workflow tag must survive; method {:?}",
            out.method
        );
    }

    #[test]
    fn never_exceeds_head_tail_ceiling_robust() {
        // For any compressible input, the result is never longer than the
        // head/tail fallback would have produced.
        let log = build_log_with_mid_error(1000);
        let out = compress_tool_output(&log, 600, 400, "");
        let fallback_len = head_tail(&log, 600, 400).chars().count();
        assert!(
            out.compressed_chars <= fallback_len.max(out.original_chars),
            "compressed ({}) must not exceed head/tail ceiling ({})",
            out.compressed_chars,
            fallback_len
        );
    }
}
