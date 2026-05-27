use super::tool_blocks::{bash_continuation_lines, tool_body_line_count};
use super::*;

/// Exact visual height for one tool block at `inner_w`.
///
/// This is deliberately a pure layout query. The old implementation kept a
/// whole-tool height LRU keyed by a full hash of the input/output payload; that
/// avoided some repeated work, but it also made every frame hash large tool
/// output just to ask for height. Row counting now mirrors the body producers
/// directly and delegates only the truly expensive primitive, syntax
/// highlighting, to `jfc_markdown::highlight_code_line_count`.
pub(super) fn tool_block_height(tool: &ToolCall, inner_w: usize) -> usize {
    if tool.display.is_collapsed() {
        return 1;
    }

    let cont = bash_continuation_lines(tool).len();
    let content_w = inner_w.saturating_sub(2);
    1 + cont + tool_content_height_with_tool(tool, content_w)
}

#[allow(dead_code)]
pub fn tool_block_height_pub(tool: &ToolCall, inner_w: usize) -> usize {
    tool_block_height(tool, inner_w)
}

/// Body-only row count for a tool. Title and Bash continuation rows are handled
/// by `tool_block_height`; this function mirrors `tool_body_lines_themed` for
/// the body itself.
pub(super) fn tool_content_height_with_tool(tool: &ToolCall, content_w: usize) -> usize {
    tool_body_line_count(tool, content_w)
}
