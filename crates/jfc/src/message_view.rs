use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use crate::app::App;
use crate::markdown;
use crate::theme::Theme;
use jfc_core::*;

mod assistant_parts;
mod bash;
mod core;
mod detection;
mod formatters;
pub(crate) mod height_index;
mod output_style;
mod outputs;
mod syntax;
pub(crate) mod task_body;
mod terminal_output;
mod tests;
mod tool_blocks;
mod tool_height;
mod tool_xml_guard;
mod truncation;

pub use assistant_parts::find_tool_at;
pub use core::{
    MessageView, PrebuiltItems, RenderCtx, build_render_items_ctx, build_render_items_pub,
    build_render_items_window, message_view_total_lines,
};
pub use task_body::{TASK_VIEW_COLLAPSE_BYTES, TASK_VIEW_COLLAPSE_LINES, task_view_body_lines};
pub use tool_blocks::{border_color_for_status, tool_kind_color, tool_status_icon_animated};
pub use tool_height::tool_block_height_pub;
