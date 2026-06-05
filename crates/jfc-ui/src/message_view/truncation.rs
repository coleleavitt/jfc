use super::syntax::quote_aware_tokens;
use super::*;

pub(super) enum GrepLine<'a> {
    Match {
        path: &'a str,
        lineno: Option<&'a str>,
        col: Option<&'a str>,
        body: &'a str,
        is_context: bool,
    },
    HeadingPath(&'a str),
}

/// Parse a single grep / rg result line into its components.
/// Tries the structured forms in order: column-form
/// (`path:line:col:body`), match (`path:line:body`), file-only
/// (`path:body`), single-file `<line>:<body>` (no path prefix),
/// context with `-` separators, then bare-path heading.
pub(super) fn parse_grep_line<'a>(raw: &'a str) -> Option<GrepLine<'a>> {
    // Try `:` separator first (most common).
    if let Some(parsed) = parse_grep_with_sep(raw, ':', false) {
        return Some(parsed);
    }
    // Then `-` for context lines.
    if let Some(parsed) = parse_grep_with_sep(raw, '-', true) {
        return Some(parsed);
    }
    // No path prefix: `grep -n pat single-file` emits `<lineno>:<body>`.
    // Also rg `--no-filename`. Detect by leading digits + `:`.
    if let Some(parsed) = parse_grep_no_path(raw, ':', false) {
        return Some(parsed);
    }
    // No-path context (grep `-A`/`-B`/`-C` against single file):
    // `<lineno>-<body>`.
    if let Some(parsed) = parse_grep_no_path(raw, '-', true) {
        return Some(parsed);
    }
    // Fall back to bare-path detection: a line that *looks like* a
    // file path (has slash or extension) and contains no `:` or
    // `-` markers is probably a heading.
    let trimmed = raw.trim();
    if !trimmed.is_empty()
        && (trimmed.contains('/') || std::path::Path::new(trimmed).extension().is_some())
        && !trimmed.contains(':')
    {
        return Some(GrepLine::HeadingPath(trimmed));
    }
    None
}

/// Parse the path-less `<lineno><sep><body>` form. Used by single-
/// file grep invocations where the filename isn't repeated on each
/// line. Returns `Match` with `path = ""` so the renderer skips
/// the path span entirely.
pub(super) fn parse_grep_no_path<'a>(
    raw: &'a str,
    sep: char,
    is_context: bool,
) -> Option<GrepLine<'a>> {
    let bytes = raw.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_digit() {
        return None;
    }
    let mut j = 0;
    while j < bytes.len() && bytes[j].is_ascii_digit() {
        j += 1;
    }
    // After the digit run, expect the separator. Reject if the
    // digit run is the whole line (just a number, no body).
    if j >= bytes.len() || bytes[j] != sep as u8 {
        return None;
    }
    let lineno = &raw[..j];
    let body = &raw[j + 1..];
    // Reasonable line numbers are 1..=10M. Anything wildly larger
    // is probably a different format (a hex offset, a hash) we
    // shouldn't false-match.
    if lineno.parse::<u32>().is_err() {
        return None;
    }
    Some(GrepLine::Match {
        path: "",
        lineno: Some(lineno),
        col: None,
        body,
        is_context,
    })
}

/// Look for `path<sep>lineno<sep>[col<sep>]body` in `raw`.
/// Returns None if the structure doesn't match — caller falls
/// through to the next separator or the heading-path fallback.
pub(super) fn parse_grep_with_sep<'a>(
    raw: &'a str,
    sep: char,
    is_context: bool,
) -> Option<GrepLine<'a>> {
    // Walk the string finding `<sep><digits><sep>` — that
    // anchors the "this is a (path, lineno) prefix" claim. Without
    // the digit-bracketed pattern, a path like
    // `src/foo:bar.rs:10:hi` would mis-parse.
    let bytes = raw.as_bytes();
    let sep_b = sep as u8;
    let mut i = 0;
    let mut path_end: Option<usize> = None;
    while i < bytes.len() {
        if bytes[i] == sep_b {
            // Tentative path ends at i. After i+1, we want digits
            // then another sep.
            let after = i + 1;
            let mut j = after;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > after && j < bytes.len() && bytes[j] == sep_b {
                path_end = Some(i);
                break;
            }
        }
        i += 1;
    }
    let p_end = path_end?;
    let path = &raw[..p_end];
    if path.is_empty() {
        return None;
    }
    let after_path = p_end + 1;
    let mut lineno_end = after_path;
    while lineno_end < bytes.len() && bytes[lineno_end].is_ascii_digit() {
        lineno_end += 1;
    }
    if lineno_end == after_path || lineno_end >= bytes.len() || bytes[lineno_end] != sep_b {
        return None;
    }
    let lineno = &raw[after_path..lineno_end];
    let after_lineno = lineno_end + 1;
    // Optional column: another `<digits><sep>` block.
    let mut col: Option<&str> = None;
    let body_start;
    let mut col_end = after_lineno;
    while col_end < bytes.len() && bytes[col_end].is_ascii_digit() {
        col_end += 1;
    }
    if col_end > after_lineno && col_end < bytes.len() && bytes[col_end] == sep_b {
        col = Some(&raw[after_lineno..col_end]);
        body_start = col_end + 1;
    } else {
        body_start = after_lineno;
    }
    let body = &raw[body_start..];
    Some(GrepLine::Match {
        path,
        lineno: Some(lineno),
        col,
        body,
        is_context,
    })
}

/// Extract the search pattern from a grep/rg command string.
/// Mirrors `grep_target_file` but returns the *pattern* positional
/// instead of the file positional. For `-e PAT` / `--regexp=PAT`
/// the flag value is the pattern; otherwise the first non-flag
/// positional is the pattern.
pub(super) fn grep_search_pattern(cmd: &str) -> Option<String> {
    let toks = quote_aware_tokens(cmd);
    let mut it = toks.into_iter();
    let verb = it.next()?;
    if !matches!(verb.as_str(), "grep" | "rg" | "ack" | "ag" | "ripgrep") {
        return None;
    }
    const VALUE_FLAGS: &[&str] = &[
        "-A",
        "-B",
        "-C",
        "-m",
        "--max-count",
        "--type",
        "-t",
        "--type-not",
        "-T",
        "--color",
        "--colour",
        "-g",
        "--glob",
        "--iglob",
        "--include",
        "--exclude",
        "--exclude-dir",
        "--threads",
        "-j",
    ];
    const PATTERN_FLAGS: &[&str] = &["-e", "--regexp"];
    // `-f FILE` supplies pattern from a *file* — we can't extract a
    // literal pattern from that, so skip it as a value flag only.
    const PATTERN_FILE_FLAGS: &[&str] = &["-f", "--file"];
    let mut explicit_pattern: Option<String> = None;
    let mut first_positional: Option<String> = None;
    while let Some(tok) = it.next() {
        if tok.starts_with("--") {
            let (key, inline_val) = match tok.split_once('=') {
                Some((k, v)) => (k, Some(v.to_string())),
                None => (tok.as_str(), None),
            };
            if PATTERN_FLAGS.contains(&key) {
                let val = inline_val.or_else(|| it.next());
                if let Some(v) = val {
                    explicit_pattern = Some(unquote(&v));
                }
                continue;
            }
            if PATTERN_FILE_FLAGS.contains(&key) {
                if inline_val.is_none() {
                    it.next();
                }
                continue;
            }
            if inline_val.is_none() && VALUE_FLAGS.contains(&tok.as_str()) {
                it.next();
            }
            continue;
        }
        if tok.starts_with('-') && tok.len() > 1 && !tok.chars().all(|c| c == '-') {
            if PATTERN_FLAGS.contains(&tok.as_str()) {
                if let Some(v) = it.next() {
                    explicit_pattern = Some(unquote(&v));
                }
                continue;
            }
            if PATTERN_FILE_FLAGS.contains(&tok.as_str()) {
                it.next();
                continue;
            }
            if VALUE_FLAGS.contains(&tok.as_str()) {
                it.next();
            }
            continue;
        }
        // First positional = pattern (unless `-e` already gave us one).
        if first_positional.is_none() {
            first_positional = Some(unquote(&tok));
        }
        // Don't break — keep scanning for explicit `-e` which wins.
    }
    explicit_pattern.or(first_positional)
}

/// Check whether the grep invocation is case-insensitive (`-i` /
/// `--ignore-case`). Used to decide whether pattern matching
/// against the body should be case-folded.
pub(super) fn grep_is_case_insensitive(cmd: &str) -> bool {
    let toks = quote_aware_tokens(cmd);
    for tok in &toks {
        if tok == "-i" || tok == "--ignore-case" {
            return true;
        }
        // Bundled short flags: `-inr`, `-rni`, etc.
        if tok.starts_with('-') && !tok.starts_with("--") && tok.len() > 1 && tok.contains('i') {
            return true;
        }
    }
    false
}

/// Strip surrounding single or double quotes from a token.
fn unquote(s: &str) -> String {
    s.strip_prefix('\'')
        .and_then(|inner| inner.strip_suffix('\''))
        .or_else(|| {
            s.strip_prefix('"')
                .and_then(|inner| inner.strip_suffix('"'))
        })
        .unwrap_or(s)
        .to_string()
}

pub(super) fn push_wrapped_styled_line(
    out: &mut Vec<Line<'static>>,
    spans: Vec<Span<'static>>,
    width: usize,
    bg: Color,
) {
    let line = Line::from(spans).style(Style::default().bg(bg));
    for wrapped in terminal_output::wrap_styled_line(&line, width) {
        out.push(wrapped.style(Style::default().bg(bg)));
    }
}

pub(super) fn push_wrapped_diff_data_line(
    out: &mut Vec<Line<'static>>,
    lineno: Option<usize>,
    sigil: &'static str,
    fg_color: Color,
    bg_color: Color,
    text_muted: Color,
    content_spans: Vec<Span<'static>>,
    width: usize,
) {
    let prefix_w = 8usize;
    let content_w = width.saturating_sub(prefix_w).max(1);
    let chunks = terminal_output::wrap_styled_line(&Line::from(content_spans), content_w);
    let lineno_str = match lineno {
        Some(n) => format!("{n:>5} "),
        None => "      ".into(),
    };
    for (idx, chunk) in chunks.into_iter().enumerate() {
        let mut spans = Vec::with_capacity(chunk.spans.len() + 2);
        if idx == 0 {
            spans.push(Span::styled(
                lineno_str.clone(),
                Style::default().fg(text_muted).bg(bg_color),
            ));
            spans.push(Span::styled(
                format!("{sigil} "),
                Style::default().fg(fg_color).bg(bg_color),
            ));
        } else {
            spans.push(Span::styled(
                "        ",
                Style::default().fg(text_muted).bg(bg_color),
            ));
        }
        spans.extend(chunk.spans);
        out.push(Line::from(spans).style(Style::default().bg(bg_color)));
    }
}

/// Walk the original command and return the first positional that
/// looks like a file/directory the user grep'd against. Used by
/// `render_grep_output_skip` to surface a heading line when grep
/// emitted path-less `<lineno>:<body>` rows (single-file mode), so
/// the user can see *which* file is being searched.
///
/// Heuristic: skip the verb (`grep`/`rg`/`ack`/`ag`), skip flags
/// (`-X`, `--long`), skip the value of flag pairs that take an
/// argument (`-e PAT`, `-f FILE`, `--type rust`), skip what looks
/// like the regex pattern (the first un-quoted positional). The
/// next positional is the target file/path. Quote-aware so a
/// pattern like `"foo("` doesn't get mistaken for a path. Returns
/// the path with surrounding quotes stripped.
pub(super) fn grep_target_file(cmd: &str) -> Option<String> {
    let toks = quote_aware_tokens(cmd);
    let mut it = toks.into_iter();
    let verb = it.next()?;
    if !matches!(verb.as_str(), "grep" | "rg" | "ack" | "ag" | "ripgrep") {
        return None;
    }
    // Flags whose value lives in the *next* token. Skip both.
    const VALUE_FLAGS: &[&str] = &[
        "-e",
        "-f",
        "-A",
        "-B",
        "-C",
        "-m",
        "--max-count",
        "--type",
        "-t",
        "--type-not",
        "-T",
        "--color",
        "--colour",
        "-g",
        "--glob",
        "--iglob",
        "--include",
        "--exclude",
        "--exclude-dir",
        "--threads",
        "-j",
    ];
    // `-e PAT` and `-f FILE` (regex source file) supply the pattern
    // via a flag value rather than a positional. When we see one of
    // those we absorb the value AND mark seen_pattern so the next
    // positional is treated as the target file.
    const PATTERN_FLAGS: &[&str] = &["-e", "--regexp", "-f", "--file"];
    let mut seen_pattern = false;
    while let Some(tok) = it.next() {
        if tok.starts_with("--") {
            let key = tok.split('=').next().unwrap_or(&tok);
            if PATTERN_FLAGS.contains(&key) {
                if !tok.contains('=') {
                    let _ = it.next();
                }
                seen_pattern = true;
                continue;
            }
            if !tok.contains('=') && VALUE_FLAGS.contains(&tok.as_str()) {
                let _ = it.next();
            }
            continue;
        }
        if tok.starts_with('-') && tok.len() > 1 && !tok.chars().all(|c| c == '-') {
            if PATTERN_FLAGS.contains(&tok.as_str()) {
                let _ = it.next();
                seen_pattern = true;
                continue;
            }
            if VALUE_FLAGS.contains(&tok.as_str()) {
                let _ = it.next();
            }
            continue;
        }
        if !seen_pattern {
            seen_pattern = true;
            continue;
        }
        // First positional after the pattern → target.
        let unquoted = tok
            .strip_prefix('\'')
            .and_then(|s| s.strip_suffix('\''))
            .or_else(|| tok.strip_prefix('"').and_then(|s| s.strip_suffix('"')))
            .map(|s| s.to_string())
            .unwrap_or(tok);
        return Some(unquoted);
    }
    None
}
