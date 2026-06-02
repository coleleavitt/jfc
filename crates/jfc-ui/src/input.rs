use crossterm::event::{self, KeyCode, KeyModifiers};
use ratatui::style::Style;
use ratatui_textarea::{CursorMove, TextArea};
use std::sync::Arc;
use tokio::sync::mpsc;

mod account_commands;
mod approval;
mod automation_commands;
mod context_commands;
mod delegating_commands;
mod editing;
mod github_commands;
mod info_commands;
mod key_dispatch;
mod local_commands;
mod mcp_commands;
mod mentions;
mod modal_handlers;
mod model_picker;
mod navigation;
mod palette;
mod question;
mod session_commands;
mod session_picker;
mod slash_commands;
mod submit;
mod support;
pub(crate) mod vim;
mod task_commands;
mod theme_picker;
mod worktree_commands;

#[cfg(test)]
mod tests;

use automation_commands::{handle_dream_command, handle_loop_command, handle_schedule_command};
use editing::{
    input_has_text, move_input_cursor_visual_down, move_input_cursor_visual_up, reset_input,
    step_reasoning_effort, textarea_char_len,
};
use github_commands::{
    handle_install_github_app, handle_pr_autofix, handle_pr_view, handle_setup_github_actions,
};
use local_commands::{
    handle_bug_command, handle_cost_command, handle_doc_command, handle_dump_context_command,
    handle_fleet_command, handle_init_command, handle_output_style_command, handle_rewind_command,
    handle_status_command, handle_teleport_command, handle_theme_command,
};
use mcp_commands::handle_mcp_command;
use mentions::{apply_mention_pick, update_mention_state_after_input};
pub use model_picker::filtered_models;
use model_picker::{handle_model_picker_key, open_model_picker};
use navigation::{
    collect_recent_paths, jump_to_last_assistant, jump_to_last_error, jump_to_last_tool,
    jump_to_last_user, recall_next_prompt, recall_previous_prompt, refresh_search_matches,
    scroll_to_message,
};
pub use palette::{collect_all_models, palette_items};
use session_picker::{handle_session_picker_key, open_session_picker};
pub(crate) use theme_picker::filtered_theme_choices;
use worktree_commands::handle_worktree_command;

use crate::app::App;
use crate::runtime::{AppEvent, UiEvent};
use crate::types::*;

// Re-export the public functions from sub-modules
pub(crate) use approval::handle_remote_approval_response;
pub(crate) use question::build_pending_question;
pub use key_dispatch::handle_key;
pub(crate) use slash_commands::SLASH_COMMANDS;
pub use slash_commands::run_slash_command;
pub use submit::handle_submit_text;
