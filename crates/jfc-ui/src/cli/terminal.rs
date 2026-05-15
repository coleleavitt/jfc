use std::io;

use crossterm::{
    event::{
        DisableBracketedPaste, DisableMouseCapture, KeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{LeaveAlternateScreen, disable_raw_mode},
};

pub(super) fn install_terminal_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            LeaveAlternateScreen,
            DisableMouseCapture,
            DisableBracketedPaste
        );
        previous(info);
    }));
}

/// Push kitty keyboard enhancement flags so Ctrl+M is distinguishable from Enter
/// (and Ctrl+J / Shift+Enter from one another). Returns true if flags were pushed
/// and need to be popped on exit.
pub(super) fn enable_keyboard_enhancement(stdout: &mut io::Stdout) -> bool {
    if !matches!(
        crossterm::terminal::supports_keyboard_enhancement(),
        Ok(true)
    ) {
        return false;
    }
    execute!(
        stdout,
        PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
        )
    )
    .is_ok()
}
