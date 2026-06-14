use crossterm::event::{self, KeyCode, KeyModifiers};
use ratatui::style::Style;
use ratatui_textarea::{CursorMove, TextArea};
use tokio::sync::mpsc;

mod approval;
mod editing;
mod elicitation;
mod key_dispatch;
mod mentions;
mod modal_handlers;
mod model_picker;
mod navigation;
mod palette;
mod prompt_rewrite;
mod question;
mod session_picker;
mod slash_commands;
mod submit;
mod theme_picker;
mod view_commands;
pub(crate) mod vim;

#[cfg(test)]
mod tests;

use editing::{
    input_has_text, move_input_cursor_visual_down, move_input_cursor_visual_up, reset_input,
    step_reasoning_effort, textarea_char_len,
};
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

use crate::app::App;
use jfc_core::*;

// Re-export the public functions from sub-modules
pub use key_dispatch::handle_key;
pub use slash_commands::run_slash_command;
pub(crate) use slash_commands::slash_commands_table;
pub use slash_commands::url_encode;
pub use submit::handle_submit_text;
