//! Minimal Rust source masker shared by the quality guards.
//!
//! Replaces the *contents* of string literals, char literals, and comments
//! with spaces while leaving code — and structural punctuation like braces —
//! intact. This stops the guards' brace-counting and substring scans from
//! being fooled by a `}` or a `ToolKind::Foo` that lives inside a string or a
//! comment (both real defects the auto-review found in `check_blocking_in_async`
//! and `guards::wiring`).
//!
//! Heuristic-grade, not a full lexer, but it covers the cases that actually
//! trip the guards: line comments, (nesting) block comments, normal strings
//! with escapes, raw strings (`r#"..."#`), and char literals — with a
//! lifetime-vs-char-literal heuristic so `'a` isn't mistaken for an unterminated
//! char literal. Line and column *counts* are preserved (every masked char maps
//! to exactly one output char, newlines pass through) so callers can still
//! report accurate line numbers.

/// Lexer state across the single forward pass.
#[derive(Clone, Copy)]
enum S {
    Code,
    /// Line comment — until the next newline.
    Line,
    /// Block comment with nesting depth (Rust allows nested `/* */`).
    Block(usize),
    /// String literal. `raw` is `Some(hash_count)` for `r#"..."#` forms.
    Str { raw: Option<usize>, esc: bool },
    /// Char literal.
    Chr { esc: bool },
}

/// Per-step outcome: how many input chars were consumed, what to emit, and the
/// next state. `emit` is always the same length as the consumed span so line
/// and column counts are preserved.
struct Step {
    consumed: usize,
    emit: &'static str,
    next: S,
}

/// A cursor over the char slice for one masking step.
struct Cur<'a> {
    chars: &'a [char],
    i: usize,
}

impl Cur<'_> {
    fn c(&self) -> char {
        self.chars[self.i]
    }
    fn peek(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.i + ahead).copied()
    }
}

/// Return `src` with string/char/comment content replaced by spaces. Code and
/// braces survive; newlines are preserved so line numbers stay aligned.
pub(crate) fn mask_source(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut st = S::Code;
    let mut i = 0;
    while i < chars.len() {
        let cur = Cur { chars: &chars, i };
        let step = match st {
            S::Code => step_code(&cur),
            S::Line => step_line(&cur),
            S::Block(d) => step_block(&cur, d),
            S::Str { raw, esc } => step_str(&cur, raw, esc),
            S::Chr { esc } => step_chr(&cur, esc),
        };
        // `emit` is a single-purpose marker; expand it to the consumed width.
        push_emit(&mut out, &cur, &step);
        st = step.next;
        i += step.consumed;
    }
    out
}

/// Expand a step's `emit` marker into output of the right width. "nl" passes a
/// newline through; "code" copies the consumed chars verbatim; "blank" masks
/// the consumed span with spaces.
fn push_emit(out: &mut String, cur: &Cur<'_>, step: &Step) {
    match step.emit {
        "nl" => out.push('\n'),
        "code" => {
            for k in 0..step.consumed {
                out.push(cur.chars[cur.i + k]);
            }
        }
        _ => {
            for _ in 0..step.consumed {
                out.push(' ');
            }
        }
    }
}

fn step_code(cur: &Cur<'_>) -> Step {
    let c = cur.c();
    let n = cur.peek(1);
    if c == '\n' {
        return Step { consumed: 1, emit: "nl", next: S::Code };
    }
    if c == '/' && n == Some('/') {
        return Step { consumed: 2, emit: "blank", next: S::Line };
    }
    if c == '/' && n == Some('*') {
        return Step { consumed: 2, emit: "blank", next: S::Block(1) };
    }
    if c == '"' {
        return Step {
            consumed: 1,
            emit: "blank",
            next: S::Str { raw: None, esc: false },
        };
    }
    if c == 'r' && (n == Some('"') || n == Some('#')) {
        // Possible raw string: r"..." or r#"..."#. Count the hashes.
        let mut k = cur.i + 1;
        let mut h = 0;
        while cur.chars.get(k) == Some(&'#') {
            h += 1;
            k += 1;
        }
        if cur.chars.get(k) == Some(&'"') {
            return Step {
                consumed: (k - cur.i) + 1,
                emit: "blank",
                next: S::Str { raw: Some(h), esc: false },
            };
        }
        return Step { consumed: 1, emit: "code", next: S::Code };
    }
    if c == '\'' {
        // Char literal vs lifetime. `'\x'` (escape) and `'x'` (char-then-quote)
        // are literals; anything else is a lifetime like `'a` and stays code.
        if n == Some('\\') {
            return Step { consumed: 1, emit: "blank", next: S::Chr { esc: true } };
        }
        if cur.peek(2) == Some('\'') {
            return Step { consumed: 1, emit: "blank", next: S::Chr { esc: false } };
        }
        return Step { consumed: 1, emit: "code", next: S::Code };
    }
    Step { consumed: 1, emit: "code", next: S::Code }
}

fn step_line(cur: &Cur<'_>) -> Step {
    if cur.c() == '\n' {
        Step { consumed: 1, emit: "nl", next: S::Code }
    } else {
        Step { consumed: 1, emit: "blank", next: S::Line }
    }
}

fn step_block(cur: &Cur<'_>, depth: usize) -> Step {
    let c = cur.c();
    let n = cur.peek(1);
    if c == '\n' {
        return Step { consumed: 1, emit: "nl", next: S::Block(depth) };
    }
    if c == '/' && n == Some('*') {
        return Step { consumed: 2, emit: "blank", next: S::Block(depth + 1) };
    }
    if c == '*' && n == Some('/') {
        let next = if depth <= 1 { S::Code } else { S::Block(depth - 1) };
        return Step { consumed: 2, emit: "blank", next };
    }
    Step { consumed: 1, emit: "blank", next: S::Block(depth) }
}

fn step_str(cur: &Cur<'_>, raw: Option<usize>, esc: bool) -> Step {
    match raw {
        None => step_str_normal(cur, esc),
        Some(h) => step_str_raw(cur, h),
    }
}

fn step_str_normal(cur: &Cur<'_>, esc: bool) -> Step {
    let c = cur.c();
    if c == '\n' {
        // Normal strings may span lines (escaped newline); preserve it.
        return Step { consumed: 1, emit: "nl", next: S::Str { raw: None, esc: false } };
    }
    if esc {
        return Step { consumed: 1, emit: "blank", next: S::Str { raw: None, esc: false } };
    }
    if c == '\\' {
        return Step { consumed: 1, emit: "blank", next: S::Str { raw: None, esc: true } };
    }
    if c == '"' {
        return Step { consumed: 1, emit: "blank", next: S::Code };
    }
    Step { consumed: 1, emit: "blank", next: S::Str { raw: None, esc: false } }
}

fn step_str_raw(cur: &Cur<'_>, hashes: usize) -> Step {
    let c = cur.c();
    if c == '\n' {
        return Step { consumed: 1, emit: "nl", next: S::Str { raw: Some(hashes), esc: false } };
    }
    if c == '"' {
        // Closes only at `"` followed by exactly `hashes` hashes.
        let mut k = cur.i + 1;
        let mut cnt = 0;
        while cnt < hashes && cur.chars.get(k) == Some(&'#') {
            cnt += 1;
            k += 1;
        }
        if cnt == hashes {
            return Step { consumed: k - cur.i, emit: "blank", next: S::Code };
        }
    }
    Step { consumed: 1, emit: "blank", next: S::Str { raw: Some(hashes), esc: false } }
}

fn step_chr(cur: &Cur<'_>, esc: bool) -> Step {
    let c = cur.c();
    if c == '\n' {
        return Step { consumed: 1, emit: "nl", next: S::Code };
    }
    if esc {
        return Step { consumed: 1, emit: "blank", next: S::Chr { esc: false } };
    }
    if c == '\\' {
        return Step { consumed: 1, emit: "blank", next: S::Chr { esc: true } };
    }
    if c == '\'' {
        return Step { consumed: 1, emit: "blank", next: S::Code };
    }
    Step { consumed: 1, emit: "blank", next: S::Chr { esc: false } }
}

#[cfg(test)]
mod tests {
    use super::mask_source;

    #[test]
    fn masks_brace_inside_string_normal() {
        let masked = mask_source(r#"let s = "}";"#);
        // The brace inside the string is gone; the code structure (= ;) stays.
        assert!(!masked.contains('}'), "string brace not masked: {masked:?}");
        assert!(masked.contains('='));
    }

    #[test]
    fn masks_token_inside_line_comment_normal() {
        let masked = mask_source("let x = 1; // ToolKind::ApplyPatch later");
        assert!(
            !masked.contains("ToolKind"),
            "comment token leaked: {masked:?}"
        );
        assert!(masked.contains("let x"));
    }

    #[test]
    fn keeps_lifetime_as_code_robust() {
        // `'a` is a lifetime, not a char literal — must stay code so the
        // following braces are still counted.
        let masked = mask_source("fn f<'a>(x: &'a str) { x }");
        assert!(masked.contains('{'));
        assert!(masked.contains('}'));
    }

    #[test]
    fn masks_nested_block_comment_robust() {
        let masked = mask_source("a /* outer /* inner } */ still } */ b {");
        // Both braces inside the (nested) comment are masked; the trailing
        // real brace survives.
        assert_eq!(masked.matches('{').count(), 1);
        assert_eq!(masked.matches('}').count(), 0);
    }

    #[test]
    fn masks_raw_string_contents_robust() {
        let masked = mask_source(r##"let s = r#"a "}" b"#; let y = 2;"##);
        assert!(!masked.contains('}'), "raw-string brace leaked: {masked:?}");
        assert!(masked.contains("let y"));
    }

    #[test]
    fn masks_char_literal_brace_robust() {
        let masked = mask_source("let c = '}'; let d = 1;");
        assert!(!masked.contains('}'), "char-literal brace leaked: {masked:?}");
        assert!(masked.contains("let d"));
    }

    #[test]
    fn preserves_line_count_normal() {
        let src = "line1 // c\n\"str\nstill str\"\ncode";
        assert_eq!(mask_source(src).lines().count(), src.lines().count());
    }
}
