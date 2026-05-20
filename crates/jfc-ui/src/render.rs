pub(crate) use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Padding, Paragraph},
};

#[cfg(test)]
pub(crate) use ratatui::backend::TestBackend;
#[cfg(test)]
pub(crate) use ratatui::Terminal;

pub(crate) use crate::app::App;
pub(crate) use crate::theme::Theme;
pub(crate) use crate::types::*;

mod agents;
mod approval;
mod frame;
mod input_box;
mod messages;
mod model_picker;
mod overlays;
pub(crate) mod palette;
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
pub use frame::frame;

// Re-export utilities needed by other modules
#[cfg(test)]
pub(crate) use messages::task_view_body_lines;
pub(crate) use messages::{TASK_VIEW_COLLAPSE_BYTES, TASK_VIEW_COLLAPSE_LINES};
pub(crate) use agents::format_token_count;
#[cfg(test)]
pub(crate) use agents::format_subagent_counters;
pub(crate) use overlays::{current_slash_prefix, slash_matches};
pub(crate) use visual::{pulse_color_pub, DiffStats, collect_diff_stats, truncate_str};
pub use session_sidebar::ordered_sidebar_sessions;

// Internal cross-module helpers — visible to all render submodules via `use super::*`
pub use visual::*;

fn ease_out_cubic(t: f32) -> f32 {
    let t = t - 1.0;
    t * t * t + 1.0
}
