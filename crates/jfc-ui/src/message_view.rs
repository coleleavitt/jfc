use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget, Wrap},
};

use crate::app::App;
use crate::markdown;
use crate::theme::Theme;
use crate::types::*;

mod assistant_parts;
mod bash;
mod core;
mod detection;
mod output_style;
mod outputs;
mod syntax;
mod terminal_output;
mod tests;
mod tool_blocks;
mod tool_height;

#[allow(unused_imports)]
pub use assistant_parts::find_tool_at;
#[allow(unused_imports)]
pub use core::{
    MessageView, PrebuiltItems, RenderItem, build_render_items_for_messages,
    build_render_items_pub, message_view_total_lines, warm_tool_height_cache_for_messages,
};
#[allow(unused_imports)]
pub use outputs::diff_lang;
#[allow(unused_imports)]
pub use tool_blocks::{tool_kind_color, tool_status_icon_animated};
