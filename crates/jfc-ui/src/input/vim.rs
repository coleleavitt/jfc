//! Modal (vim) editing for the prompt input, gated behind `/vim`.
//!
//! This is a focused port of the canonical `ratatui-textarea` vim example
//! (`examples/vim.rs`) adapted to operate on jfc's `App::textarea`. It covers
//! the editing core: Normal / Insert / Visual / Operator / Replace modes with
//! the usual motions (`h j k l w e b ^ $ gg G`), operators (`d c y` + `dd cc
//! yy`, `D C`), inserts (`i a A I o O`), `x`, `p`, `u` / Ctrl-r, `r` / `R`, and
//! visual `v` / `V`. Scrolling and `:`-commands are intentionally omitted — the
//! prompt is a small buffer, and jfc keeps Enter=submit and its Ctrl-shortcuts
//! at the app level regardless of mode.
//!
//! Integration: when `app.vim` is `Some`, the default text-input path routes
//! the key here instead of `textarea.input`, and Esc is mode-aware
//! (Insert→Normal first). Enabling `/vim` starts in Normal mode.

use ratatui_textarea::{CursorMove, Input, Key, TextArea};

/// Current vim sub-mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VimMode {
    Normal,
    Insert,
    Visual,
    /// A pending operator (`d` / `c` / `y`) awaiting a motion.
    Operator(char),
    /// `r` (replace-one = true) or `R` (overtype = false).
    Replace(bool),
}

impl VimMode {
    /// Short uppercase tag for the input-box mode badge.
    pub fn tag(self) -> &'static str {
        match self {
            VimMode::Normal => "NORMAL",
            VimMode::Insert => "INSERT",
            VimMode::Visual => "VISUAL",
            VimMode::Operator(_) => "OP",
            VimMode::Replace(_) => "REPLACE",
        }
    }
}

/// Persistent vim state stored on `App` while vim mode is enabled.
#[derive(Clone, Debug)]
pub struct VimState {
    pub mode: VimMode,
    /// Pending input for two-key sequences (e.g. the first `g` of `gg`).
    pub pending: Input,
}

impl Default for VimState {
    fn default() -> Self {
        // Enabling /vim drops you in Normal — that's the vim contract.
        Self {
            mode: VimMode::Normal,
            pending: Input::default(),
        }
    }
}

enum Transition {
    Nop,
    Mode(VimMode),
    Pending(Input),
}

fn is_before_line_end(ta: &TextArea<'_>) -> bool {
    let cursor = ta.cursor();
    cursor.1 < ta.lines()[cursor.0].chars().count().saturating_sub(1)
}

fn transition(state: &VimState, input: Input, ta: &mut TextArea<'_>) -> Transition {
    if input.key == Key::Null {
        return Transition::Nop;
    }
    match state.mode {
        VimMode::Normal | VimMode::Visual | VimMode::Operator(_) => {
            match input {
                Input {
                    key: Key::Char('h') | Key::Left,
                    ..
                } => ta.move_cursor(CursorMove::Back),
                Input {
                    key: Key::Char('j') | Key::Down,
                    ..
                } => ta.move_cursor(CursorMove::Down),
                Input {
                    key: Key::Char('k') | Key::Up,
                    ..
                } => ta.move_cursor(CursorMove::Up),
                Input {
                    key: Key::Char('l') | Key::Right,
                    ..
                } => ta.move_cursor(CursorMove::Forward),
                Input {
                    key: Key::Char('w'),
                    ..
                } => ta.move_cursor(CursorMove::WordForward),
                Input {
                    key: Key::Char('e'),
                    ctrl: false,
                    ..
                } => {
                    ta.move_cursor(CursorMove::WordEnd);
                    if matches!(state.mode, VimMode::Operator(_)) {
                        ta.move_cursor(CursorMove::Forward);
                    }
                }
                Input {
                    key: Key::Char('b'),
                    ctrl: false,
                    ..
                } => ta.move_cursor(CursorMove::WordBack),
                Input {
                    key: Key::Char('^' | '0'),
                    ..
                } => ta.move_cursor(CursorMove::Head),
                Input {
                    key: Key::Char('$'),
                    ..
                } => ta.move_cursor(CursorMove::End),
                Input {
                    key: Key::Char('D'),
                    ..
                } => {
                    ta.delete_line_by_end();
                    return Transition::Mode(VimMode::Normal);
                }
                Input {
                    key: Key::Char('C'),
                    ..
                } => {
                    ta.delete_line_by_end();
                    ta.cancel_selection();
                    return Transition::Mode(VimMode::Insert);
                }
                Input {
                    key: Key::Char('p'),
                    ..
                } => {
                    ta.paste();
                    return Transition::Mode(VimMode::Normal);
                }
                Input {
                    key: Key::Char('u'),
                    ctrl: false,
                    ..
                } => {
                    ta.undo();
                    return Transition::Mode(VimMode::Normal);
                }
                Input {
                    key: Key::Char('r'),
                    ctrl: true,
                    ..
                } => {
                    ta.redo();
                    return Transition::Mode(VimMode::Normal);
                }
                Input {
                    key: Key::Char('x'),
                    ..
                } if is_before_line_end(ta) || ta.lines()[ta.cursor().0].is_empty() => {
                    ta.delete_next_char();
                    return Transition::Mode(VimMode::Normal);
                }
                Input {
                    key: Key::Char('i'),
                    ..
                } => {
                    ta.cancel_selection();
                    return Transition::Mode(VimMode::Insert);
                }
                Input {
                    key: Key::Char('a'),
                    ..
                } => {
                    ta.cancel_selection();
                    if is_before_line_end(ta) {
                        ta.move_cursor(CursorMove::Forward);
                    }
                    return Transition::Mode(VimMode::Insert);
                }
                Input {
                    key: Key::Char('A'),
                    ..
                } => {
                    ta.cancel_selection();
                    ta.move_cursor(CursorMove::End);
                    return Transition::Mode(VimMode::Insert);
                }
                Input {
                    key: Key::Char('o'),
                    ..
                } => {
                    ta.move_cursor(CursorMove::End);
                    ta.insert_newline();
                    return Transition::Mode(VimMode::Insert);
                }
                Input {
                    key: Key::Char('O'),
                    ..
                } => {
                    ta.move_cursor(CursorMove::Head);
                    ta.insert_newline();
                    ta.move_cursor(CursorMove::Up);
                    return Transition::Mode(VimMode::Insert);
                }
                Input {
                    key: Key::Char('I'),
                    ..
                } => {
                    ta.cancel_selection();
                    ta.move_cursor(CursorMove::Head);
                    return Transition::Mode(VimMode::Insert);
                }
                Input {
                    key: Key::Char('r'),
                    ctrl: false,
                    ..
                } if state.mode == VimMode::Normal => {
                    return Transition::Mode(VimMode::Replace(true));
                }
                Input {
                    key: Key::Char('R'),
                    ctrl: false,
                    ..
                } if state.mode == VimMode::Normal => {
                    return Transition::Mode(VimMode::Replace(false));
                }
                Input {
                    key: Key::Char('v'),
                    ctrl: false,
                    ..
                } if state.mode == VimMode::Normal => {
                    ta.start_selection();
                    return Transition::Mode(VimMode::Visual);
                }
                Input {
                    key: Key::Char('V'),
                    ctrl: false,
                    ..
                } if state.mode == VimMode::Normal => {
                    ta.move_cursor(CursorMove::Head);
                    ta.start_selection();
                    ta.move_cursor(CursorMove::End);
                    return Transition::Mode(VimMode::Visual);
                }
                Input { key: Key::Esc, .. }
                | Input {
                    key: Key::Char('v'),
                    ctrl: false,
                    ..
                } if state.mode == VimMode::Visual => {
                    ta.cancel_selection();
                    return Transition::Mode(VimMode::Normal);
                }
                Input {
                    key: Key::Char('g'),
                    ctrl: false,
                    ..
                } if matches!(
                    state.pending,
                    Input {
                        key: Key::Char('g'),
                        ctrl: false,
                        ..
                    }
                ) =>
                {
                    ta.move_cursor(CursorMove::Top)
                }
                Input {
                    key: Key::Char('G'),
                    ctrl: false,
                    ..
                } => ta.move_cursor(CursorMove::Bottom),
                // Doubled operator (dd / cc / yy): select the whole line.
                Input {
                    key: Key::Char(c),
                    ctrl: false,
                    ..
                } if state.mode == VimMode::Operator(c) => {
                    ta.move_cursor(CursorMove::Head);
                    ta.start_selection();
                    let cursor = ta.cursor();
                    ta.move_cursor(CursorMove::Down);
                    if cursor == ta.cursor() {
                        ta.move_cursor(CursorMove::End);
                    }
                }
                Input {
                    key: Key::Char(op @ ('y' | 'd' | 'c')),
                    ctrl: false,
                    ..
                } if state.mode == VimMode::Normal => {
                    ta.start_selection();
                    return Transition::Mode(VimMode::Operator(op));
                }
                Input {
                    key: Key::Char('y'),
                    ctrl: false,
                    ..
                } if state.mode == VimMode::Visual => {
                    ta.move_cursor(CursorMove::Forward);
                    ta.copy();
                    return Transition::Mode(VimMode::Normal);
                }
                Input {
                    key: Key::Char('d'),
                    ctrl: false,
                    ..
                } if state.mode == VimMode::Visual => {
                    ta.move_cursor(CursorMove::Forward);
                    ta.cut();
                    return Transition::Mode(VimMode::Normal);
                }
                Input {
                    key: Key::Char('c'),
                    ctrl: false,
                    ..
                } if state.mode == VimMode::Visual => {
                    ta.move_cursor(CursorMove::Forward);
                    ta.cut();
                    return Transition::Mode(VimMode::Insert);
                }
                input => return Transition::Pending(input),
            }

            // Apply a pending operator now that its motion has run.
            match state.mode {
                VimMode::Operator('y') => {
                    ta.copy();
                    Transition::Mode(VimMode::Normal)
                }
                VimMode::Operator('d') => {
                    ta.cut();
                    Transition::Mode(VimMode::Normal)
                }
                VimMode::Operator('c') => {
                    ta.cut();
                    Transition::Mode(VimMode::Insert)
                }
                _ => Transition::Nop,
            }
        }
        VimMode::Insert => match input {
            Input { key: Key::Esc, .. }
            | Input {
                key: Key::Char('['),
                ctrl: true,
                ..
            } => Transition::Mode(VimMode::Normal),
            input => {
                ta.input(input);
                Transition::Mode(VimMode::Insert)
            }
        },
        VimMode::Replace(once) => match input {
            Input { key: Key::Esc, .. } => Transition::Mode(VimMode::Normal),
            Input {
                key: Key::Char(c),
                ctrl: false,
                alt: false,
                ..
            } => {
                if is_before_line_end(ta)
                    || ta.lines()[ta.cursor().0].chars().count() == ta.cursor().1
                {
                    ta.delete_next_char();
                    ta.insert_char(c);
                }
                if once {
                    Transition::Mode(VimMode::Normal)
                } else {
                    Transition::Mode(VimMode::Replace(false))
                }
            }
            _ => Transition::Mode(if once {
                VimMode::Normal
            } else {
                VimMode::Replace(false)
            }),
        },
    }
}

/// Feed one key to the vim engine, mutating `state` and `textarea`. Returns
/// the (possibly unchanged) mode so the caller can refresh the badge.
pub fn handle_key(
    state: &mut VimState,
    textarea: &mut TextArea<'_>,
    key: crossterm::event::KeyEvent,
) {
    let input: Input = key.into();
    match transition(state, input, textarea) {
        Transition::Mode(mode) => {
            state.mode = mode;
            state.pending = Input::default();
        }
        Transition::Nop => {
            state.pending = Input::default();
        }
        Transition::Pending(p) => {
            state.pending = p;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ta_with(text: &str) -> TextArea<'static> {
        TextArea::from(text.lines().map(|l| l.to_string()).collect::<Vec<_>>())
    }

    fn key(c: char) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(c),
            crossterm::event::KeyModifiers::NONE,
        )
    }

    #[test]
    fn normal_starts_in_normal_and_i_enters_insert() {
        let mut st = VimState::default();
        assert_eq!(st.mode, VimMode::Normal);
        let mut ta = ta_with("hello");
        handle_key(&mut st, &mut ta, key('i'));
        assert_eq!(st.mode, VimMode::Insert);
    }

    #[test]
    fn insert_typing_inserts_and_esc_returns_to_normal() {
        let mut st = VimState {
            mode: VimMode::Insert,
            pending: Input::default(),
        };
        let mut ta = ta_with("");
        handle_key(&mut st, &mut ta, key('x'));
        assert_eq!(ta.lines()[0], "x");
        handle_key(
            &mut st,
            &mut ta,
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Esc,
                crossterm::event::KeyModifiers::NONE,
            ),
        );
        assert_eq!(st.mode, VimMode::Normal);
    }

    #[test]
    fn dd_deletes_the_line() {
        let mut st = VimState::default();
        let mut ta = ta_with("one\ntwo");
        handle_key(&mut st, &mut ta, key('d')); // operator pending
        assert_eq!(st.mode, VimMode::Operator('d'));
        handle_key(&mut st, &mut ta, key('d')); // dd → cut line
        assert_eq!(st.mode, VimMode::Normal);
        // First line removed; "two" remains.
        assert!(ta.lines().iter().any(|l| l == "two"));
        assert!(!ta.lines().iter().any(|l| l == "one"));
    }

    #[test]
    fn x_deletes_char_under_cursor() {
        let mut st = VimState::default();
        let mut ta = ta_with("abc");
        handle_key(&mut st, &mut ta, key('x'));
        assert_eq!(ta.lines()[0], "bc");
    }
}
