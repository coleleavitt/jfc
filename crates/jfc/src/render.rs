pub(crate) use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Padding, Paragraph},
};

pub(crate) use crate::app::App;
pub(crate) use crate::theme::Theme;
pub(crate) use jfc_core::*;

mod agents;
mod approval;
/// Ported streaming-render toolkit from `openai/codex` (protocol-free pieces).
/// Stage 1 of the selective TUI port — see `codex_stream/mod.rs`.
pub(crate) mod codex_stream;
pub(crate) mod elicitation;
mod frame;
mod input_box;
mod messages;
mod model_picker;
mod overlays;
pub(crate) mod palette;
mod prompt_rewrite;
mod question;
pub(crate) mod roster;
pub(crate) mod session_picker;
mod session_sidebar;
mod sidebar;
mod status;
mod task_panel;
mod teammates_panel;
mod theme_picker;
pub(crate) mod visual;

#[cfg(test)]
mod tests;

// Re-export the top-level entry point
pub use agents::format_subagent_counters;
pub use frame::frame;

// Re-export utilities needed by other modules
#[cfg(test)]
pub(crate) use crate::message_view::task_body::task_view_body_lines;
pub(crate) use crate::message_view::{TASK_VIEW_COLLAPSE_BYTES, TASK_VIEW_COLLAPSE_LINES};
pub(crate) use agents::fleet_ordered_task_ids;
pub(crate) use overlays::{current_slash_prefix, slash_matches};
pub use session_sidebar::ordered_sidebar_sessions;
// Internal cross-module helpers — visible to all render submodules via `use super::*`
pub use visual::*;

fn ease_out_cubic(t: f32) -> f32 {
    let t = t - 1.0;
    t * t * t + 1.0
}
