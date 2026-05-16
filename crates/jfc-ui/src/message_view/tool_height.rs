use super::tool_blocks::{bash_continuation_lines, tool_body_lines};
use super::*;

/// Single-source-of-truth for a tool's rendered rows. **One producer,
/// one truth.** Both `tool_block_height` and `render_tool_block` walk
/// the body of a tool by calling `tool_body_lines` — so any change
/// to what the renderer emits automatically updates the height the
/// scroll math thinks the tool occupies.
///
/// We hit this drift class repeatedly: the renderer dispatches by
/// `BashCmdKind` to one of ~9 different `render_*_skip` functions
/// (each with their own caps, headers, footers, stderr handling),
/// while the predictor used a single `wrapped_line_count`-based
/// formula that was wrong for nearly every dispatched path. Concrete
/// drift cases pre-fix:
///
/// - **Cap mismatch**: predictor 80/500 universally; actual renderers
///   use 80/500 (most), 100/500 (tabular_list), 200/1000 (hex_dump),
///   300/1500 (compiler_output), 500-always (cat_markdown).
/// - **Exit-code header**: the predictor unconditionally added 1 for
///   the exit-code row, but `render_grep_output_skip` only emits it
///   for `code > 1`, `render_git_diff_output_skip` for `code > 1`,
///   and many others only for `code != 0` — so a successful command
///   was over-counted by 1 row.
/// - **Stderr divider**: predictor added 1 for `↳ stderr` between
///   stdout and stderr — but only `render_command_output_skip` and
///   `render_cat_output_skip` actually emit that divider. The other
///   structured renderers (grep/path_list/git_diff/git_log/hex_dump/
///   tabular_list) emit a blank line + raw stderr lines (no `↳`).
/// - **Word-wrap mismatch**: predictor used `wrapped_line_count` (cell-
///   width div_ceil), but most structured renderers don't pre-wrap —
///   they let `Paragraph` clip lines to area.width. So a long line
///   was 1 row in the renderer, multiple in the predictor.
/// - **Markdown / syntect line counts**: `cat README.md` flows through
///   `markdown::to_lines` which produces a different row count than
///   raw text-line counting. The predictor's `wrapped_line_count`
///   was off by a wide margin for any markdown-rendered content.
/// - **Highlighted file content**: `render_highlighted_block_skip`
///   uses `markdown::highlight_code` (with width subtraction), which
///   the predictor approximated with a flat `content_w - 2` raw
///   line count.
///
/// The fix mirrors the rustc query-system "feed the ground truth into
/// the query" pattern — produce the exact `Vec<Line>` once, count it
/// for height, render it for paint. The diff path stays special-cased
/// because its per-row bg tinting requires direct buffer painting
/// that doesn't fit `Paragraph`'s model; for that arm we share a
/// `diff_row_count` helper between the producer and the renderer.
/// Bounded memo of `tool_block_height` for terminal-state tools.
///
/// Why: `RenderItem::height` is summed across every render item every frame
/// (twice — `message_view_total_lines` from `render::messages`, and again
/// inside `MessageView::render`). For tool blocks the only way to know the
/// height is to ask `tool_body_lines_themed` for the full Vec<Line> and take
/// `.len()`. With the markdown highlight cache in place those Vecs are
/// produced fast, but each one is a deep-clone of N lines × M spans × String
/// per call — gdb sampling showed the main thread burning ~80% CPU in
/// `Vec<Span>::clone` while idle.
///
/// Once a tool transitions to a terminal state (Completed / Failed /
/// Cancelled) its content is immutable, so the height for a given (id, width,
/// display state) is stable forever. Caching just the integer means height
/// queries are a hash lookup with zero allocation; only the per-frame *paint*
/// of visible tools still constructs Vec<Line>.
struct ToolHeightEntry {
    height: usize,
    generation: u64,
}

const TOOL_HEIGHT_CACHE_MAX: usize = 1024;

static TOOL_HEIGHT_CACHE: std::sync::LazyLock<std::sync::Mutex<ToolHeightCache>> =
    std::sync::LazyLock::new(|| {
        std::sync::Mutex::new(ToolHeightCache {
            map: std::collections::HashMap::with_capacity(256),
            generation: 0,
        })
    });

struct ToolHeightCache {
    map: std::collections::HashMap<u64, ToolHeightEntry>,
    generation: u64,
}

/// Drop every memoized tool height. Called when something changes that the
/// per-tool fingerprint cannot encode — e.g. a layout/cap constant in this
/// module is altered. Width and display-state changes are already covered by
/// the key, so they don't need explicit invalidation.
pub fn clear_tool_height_cache() {
    let mut c = TOOL_HEIGHT_CACHE
        .lock()
        .expect("tool height cache poisoned");
    c.map.clear();
    c.generation = 0;
}

fn tool_height_fingerprint(tool: &ToolCall, inner_w: usize) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    tool.id.as_str().hash(&mut h);
    inner_w.hash(&mut h);
    tool.kind.label().hash(&mut h);
    tool.status.label().hash(&mut h);
    // ToolDisplayState is small — hash by discriminant + payload bool so we
    // don't have to add `Hash` to its derive in types.rs.
    match tool.display {
        ToolDisplayState::Default { pinned } => {
            0u8.hash(&mut h);
            pinned.hash(&mut h);
        }
        ToolDisplayState::Collapsed => {
            1u8.hash(&mut h);
        }
        ToolDisplayState::Expanded { pinned } => {
            2u8.hash(&mut h);
            pinned.hash(&mut h);
        }
    }
    // Height also depends on immutable terminal content. Tool ids should be
    // unique in normal transcripts, but tests and imported sessions can reuse
    // ids; do not let one terminal tool poison the cached height of another.
    hash_tool_input_height_fields(&tool.input, &mut h);
    hash_tool_output_height_fields(&tool.output, &mut h);
    h.finish()
}

fn hash_tool_input_height_fields(input: &ToolInput, h: &mut impl std::hash::Hasher) {
    use std::hash::Hash;
    std::mem::discriminant(input).hash(h);
    if let ToolInput::Bash { command, .. } = input {
        command.hash(h);
    }
}

fn hash_tool_output_height_fields(output: &ToolOutput, h: &mut impl std::hash::Hasher) {
    use std::hash::Hash;
    std::mem::discriminant(output).hash(h);
    match output {
        ToolOutput::Text(text) => text.hash(h),
        ToolOutput::LargeText(text) => {
            text.line_count.hash(h);
            text.byte_count.hash(h);
            text.content.hash(h);
        }
        ToolOutput::Diff(diff) => {
            diff.file_path.hash(h);
            diff.additions.hash(h);
            diff.deletions.hash(h);
            for hunk in &diff.hunks {
                hunk.old_start.hash(h);
                hunk.new_start.hash(h);
                for line in &hunk.lines {
                    std::mem::discriminant(&line.kind).hash(h);
                    line.content.hash(h);
                }
            }
        }
        ToolOutput::FileContent {
            path,
            content,
            language,
        } => {
            path.hash(h);
            content.hash(h);
            language.hash(h);
        }
        ToolOutput::Command {
            stdout,
            stderr,
            exit_code,
        } => {
            stdout.hash(h);
            stderr.hash(h);
            exit_code.hash(h);
        }
        ToolOutput::FileList(files) => files.hash(h),
        ToolOutput::ServerToolResult { tool_kind, content } => {
            tool_kind.wire_type().hash(h);
            // Stringify the JSON for hashing — the rendered text is
            // derived from this content via
            // `format_server_tool_result_text_public`, so hashing the
            // raw JSON keeps the cache key tight to what the
            // renderer actually outputs.
            if let Ok(s) = serde_json::to_string(content) {
                s.hash(h);
            }
        }
        ToolOutput::Empty => {}
    }
}

pub(super) fn tool_block_height(tool: &ToolCall, inner_w: usize) -> usize {
    if tool.display.is_collapsed() {
        return 1;
    }
    // Cache only terminal-state tools — Running/Pending/Idle tools have
    // mutable content (output streams in chunk-by-chunk) and would serve
    // stale heights. `elapsed_ms` is not part of the key because it's set
    // at the same transition that freezes content, so it's already implied
    // by the status check.
    let cacheable = matches!(
        tool.status,
        ExecutionStatus::Completed | ExecutionStatus::Failed | ExecutionStatus::Cancelled
    );

    if cacheable {
        let key = tool_height_fingerprint(tool, inner_w);
        let mut cache = TOOL_HEIGHT_CACHE
            .lock()
            .expect("tool height cache poisoned");
        cache.generation = cache.generation.wrapping_add(1);
        let gen_now = cache.generation;
        if let Some(entry) = cache.map.get_mut(&key) {
            entry.generation = gen_now;
            return entry.height;
        }
        // Drop the lock while we compute so re-entrant calls (the predictor
        // recursing into another tool inside this one's body — none today,
        // but cheap insurance) don't deadlock.
        drop(cache);

        let cont = bash_continuation_lines(tool).len();
        let content_w = inner_w.saturating_sub(2);
        let height = 1 + cont + tool_content_height_with_tool(tool, content_w);

        let mut cache = TOOL_HEIGHT_CACHE
            .lock()
            .expect("tool height cache poisoned");
        if cache.map.len() >= TOOL_HEIGHT_CACHE_MAX {
            let target = TOOL_HEIGHT_CACHE_MAX * 3 / 4;
            let mut gens: Vec<u64> = cache.map.values().map(|e| e.generation).collect();
            gens.sort_unstable();
            let cutoff_idx = cache.map.len().saturating_sub(target);
            let cutoff = gens.get(cutoff_idx).copied().unwrap_or(u64::MAX);
            cache.map.retain(|_, e| e.generation > cutoff);
        }
        cache.map.insert(
            key,
            ToolHeightEntry {
                height,
                generation: gen_now,
            },
        );
        return height;
    }

    let cont = bash_continuation_lines(tool).len();
    let content_w = inner_w.saturating_sub(2);
    1 + cont + tool_content_height_with_tool(tool, content_w)
}

pub fn tool_block_height_pub(tool: &ToolCall, inner_w: usize) -> usize {
    tool_block_height(tool, inner_w)
}

/// Body-only row count for a tool — drives both the height-math
/// predictor and the actual draw via `tool_body_lines` /
/// `tool_body_diff_rows`.
pub(super) fn tool_content_height_with_tool(tool: &ToolCall, content_w: usize) -> usize {
    match &tool.output {
        ToolOutput::Diff(diff) => diff_row_count(diff, tool.display.is_expanded(), content_w),
        // Empty + text + command + file-content + file-list all
        // produce a flat `Vec<Line>` — count it directly.
        _ => tool_body_lines(tool, content_w).len(),
    }
}

/// Count rows for a diff render. Mirrors `render_diff_skip` exactly:
/// summary line (when there are non-zero additions/deletions) +
/// per-hunk (1 header + min(lines.len, hunk_cap) data + overflow
/// footer when truncated). Both paths read this; if you change the
/// diff renderer's row layout you change this.
pub(super) fn diff_row_count(diff: &DiffView, expanded: bool, content_w: usize) -> usize {
    super::outputs::produce_diff_view_lines(diff, crate::theme::Theme::dark(), expanded, content_w)
        .len()
}
