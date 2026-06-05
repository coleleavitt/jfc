use std::io;

use crossterm::{
    event::{
        DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, KeyboardEnhancementFlags,
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
            DisableBracketedPaste,
            DisableFocusChange
        );
        previous(info);
    }));
}

/// Push kitty keyboard enhancement flags so Ctrl+M is distinguishable from Enter
/// (and Ctrl+J / Shift+Enter from one another). Returns true if flags were pushed
/// and need to be popped on exit.
pub(super) fn enable_keyboard_enhancement(stdout: &mut io::Stdout) -> bool {
    // Do not call `crossterm::terminal::supports_keyboard_enhancement()` here:
    // on Unix it sends a terminal query and waits up to 2000ms for a reply.
    // That was directly in the startup path after entering raw mode, so
    // terminals/PTYs that do not answer the kitty query showed a blank
    // alt-screen for two seconds before the first frame.
    execute!(
        stdout,
        PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
        )
    )
    .is_ok()
}
