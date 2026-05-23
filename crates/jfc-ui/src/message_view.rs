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
use crate::types::*;

mod assistant_parts;
mod bash;
mod core;
mod detection;
mod formatters;
mod output_style;
mod outputs;
mod syntax;
mod terminal_output;
mod tests;
mod tool_blocks;
mod tool_height;
mod truncation;

#[allow(unused_imports)]
pub use assistant_parts::find_tool_at;
#[allow(unused_imports)]
pub use assistant_parts::pretty_model_badge;
#[allow(unused_imports)]
pub use core::{
    MessageView, PrebuiltItems, RenderCtx, RenderItem, build_render_items_ctx,
    build_render_items_pub, message_view_total_lines, warm_tool_height_cache_for_messages,
};
pub use tool_height::{load_tool_height_cache, persist_tool_height_cache};
#[allow(unused_imports)]
pub use outputs::diff_lang;
#[allow(unused_imports)]
pub use tool_blocks::{tool_kind_color, tool_status_icon_animated};
