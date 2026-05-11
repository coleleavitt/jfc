use super::assistant_parts::sanitize_terminal_text;
use super::output_style::path_color;
use super::syntax::{lang_from_path, quote_aware_tokens};
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

fn push_wrapped_styled_line(
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

fn push_wrapped_diff_data_line(
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
            if PATTERN_FLAGS.iter().any(|f| *f == key) {
                if !tok.contains('=') {
                    let _ = it.next();
                }
                seen_pattern = true;
                continue;
            }
            if !tok.contains('=') && VALUE_FLAGS.iter().any(|f| *f == tok.as_str()) {
                let _ = it.next();
            }
            continue;
        }
        if tok.starts_with('-') && tok.len() > 1 && !tok.chars().all(|c| c == '-') {
            if PATTERN_FLAGS.iter().any(|f| *f == tok.as_str()) {
                let _ = it.next();
                seen_pattern = true;
                continue;
            }
            if VALUE_FLAGS.iter().any(|f| *f == tok.as_str()) {
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

/// Render `grep -rn` / `rg` / `ack` output. Handles all the
/// formats those tools emit (verified against ripgrep's
/// `crates/printer/src/standard.rs` and GNU grep's `print_sep`):
///
/// - `path:line:col:match`   (rg with `--column`)
/// - `path:line:match`       (default rg / `grep -n`)
/// - `path:match`            (no line numbers, e.g. `grep -h`)
/// - `path-line-context`     (grep `-A`/`-B`/`-C`, context uses `-`)
/// - `--`                    (group separator between matches)
/// - bare path on its own line (rg `--heading` mode)
///
/// Path gets its language-tinted color, line number warning-yellow
/// (matches grep's default), `:` separators muted, match body in
/// surface text color. Context lines (`-` separator) dim their
/// body to differentiate from matches.
pub(super) fn produce_grep_output_lines(
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
    t: Theme,
    expanded: bool,
    cmd: &str,
) -> Vec<Line<'static>> {
    let max_lines = if expanded { 500usize } else { 80usize };
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(code) = exit_code {
        // grep returns 1 for "no matches found" — that's not a
        // failure visually, just an empty result. Only color the
        // exit code red for truly weird codes (>1).
        if code > 1 {
            lines.push(Line::from(Span::styled(
                format!("[exit {code}]"),
                Style::default().fg(t.error),
            )));
        }
    }
    // Single-file grep (`grep -n PAT file.rs`) emits `<lineno>:body`
    // with no path prefix on each line. Without a heading the user
    // can't tell which file they searched — surface the file path
    // we extracted from the command so each match has context.
    let first_data = stdout.lines().find(|l| !l.is_empty());
    let pathless = first_data
        .map(|l| matches!(parse_grep_line(l), Some(GrepLine::Match { path: "", .. })))
        .unwrap_or(false);
    if pathless && let Some(target) = grep_target_file(cmd) {
        lines.push(Line::from(Span::styled(
            target,
            Style::default()
                .fg(t.accent)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )));
    }
    let mut total = 0usize;
    for raw in stdout.lines() {
        total += 1;
        if lines.len() >= max_lines {
            continue;
        }
        let clean = sanitize_terminal_text(raw);

        if clean == "--" {
            lines.push(Line::from(Span::styled(
                "──".to_string(),
                Style::default().fg(t.text_muted),
            )));
            continue;
        }

        let parsed = parse_grep_line(&clean);
        match parsed {
            Some(GrepLine::Match {
                path,
                lineno,
                col,
                body,
                is_context,
            }) => {
                let sep_color = t.text_muted;
                let body_color = if is_context {
                    t.text_muted
                } else {
                    t.text_secondary
                };
                let lineno_color = if is_context { t.text_muted } else { t.warning };
                let sep_str = if is_context { "-" } else { ":" };
                let mut spans: Vec<Span<'static>> = Vec::new();
                if !path.is_empty() {
                    spans.push(Span::styled(
                        path.to_owned(),
                        Style::default().fg(path_color(path, t)),
                    ));
                }
                if let Some(n) = lineno {
                    if !path.is_empty() {
                        spans.push(Span::styled(
                            sep_str.to_owned(),
                            Style::default().fg(sep_color),
                        ));
                    }
                    spans.push(Span::styled(
                        n.to_owned(),
                        Style::default().fg(lineno_color),
                    ));
                }
                if let Some(c) = col {
                    spans.push(Span::styled(
                        sep_str.to_owned(),
                        Style::default().fg(sep_color),
                    ));
                    spans.push(Span::styled(
                        c.to_owned(),
                        Style::default().fg(t.text_muted),
                    ));
                }
                spans.push(Span::styled(
                    sep_str.to_owned(),
                    Style::default().fg(sep_color),
                ));
                spans.push(Span::styled(
                    body.to_owned(),
                    Style::default().fg(body_color),
                ));
                lines.push(Line::from(spans));
            }
            Some(GrepLine::HeadingPath(path)) => {
                lines.push(Line::from(Span::styled(
                    path.to_owned(),
                    Style::default()
                        .fg(path_color(path, t))
                        .add_modifier(Modifier::BOLD),
                )));
            }
            None => {
                lines.push(Line::from(Span::styled(
                    clean,
                    Style::default().fg(t.text_secondary),
                )));
            }
        }
    }
    if total > max_lines {
        lines.push(Line::from(Span::styled(
            format!(
                "… {} more lines · click or press o to expand",
                total - max_lines
            ),
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        )));
    }
    if !stderr.is_empty() {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        for sl in stderr.lines() {
            lines.push(Line::from(Span::styled(
                sanitize_terminal_text(sl),
                Style::default().fg(t.error),
            )));
        }
    }
    lines
}

/// Render `find` / `ls` / `tree` / `fd` output as a list of paths
/// colored by file extension. Multi-column `ls` output (no flags)
/// is split on whitespace and each entry gets its own colored
/// span; `ls -l` lines get split by column with file mode in muted,
/// size right-aligned, name colored.
pub(super) fn produce_path_list_output_lines(
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
    t: Theme,
    expanded: bool,
) -> Vec<Line<'static>> {
    let max_lines = if expanded { 500usize } else { 80usize };
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(code) = exit_code {
        if code != 0 {
            lines.push(Line::from(Span::styled(
                format!("[exit {code}]"),
                Style::default().fg(t.error),
            )));
        }
    }
    let mut total = 0usize;
    for raw in stdout.lines() {
        total += 1;
        if lines.len() >= max_lines {
            continue;
        }
        // `ls -l` long format: `<perms> <links> <user> <group> <size> <date> <name>`
        // — first char is a file-type indicator (`-`, `d`, `l`, etc.).
        let is_ls_long = raw
            .chars()
            .next()
            .map(|c| matches!(c, '-' | 'd' | 'l' | 'c' | 'b' | 'p' | 's'))
            .unwrap_or(false)
            && raw.split_whitespace().count() >= 7;
        if is_ls_long {
            let cols: Vec<&str> = raw.splitn(9, char::is_whitespace).collect();
            // Re-split smarter: we want file mode, ..., name (which
            // may contain spaces in `ls -lQ` etc.).
            let parts: Vec<&str> = raw.split_whitespace().collect();
            if parts.len() >= 8 {
                let perms = parts[0];
                // Find the size column (5th non-empty token after links)
                let name_start = parts[..parts.len() - 1]
                    .iter()
                    .map(|s| s.len())
                    .sum::<usize>()
                    + parts.len()
                    - 2; // approximation
                let name = parts.last().copied().unwrap_or("");
                let _ = name_start;
                let _ = cols;
                let mut spans: Vec<Span<'static>> = Vec::new();
                spans.push(Span::styled(
                    perms.to_owned(),
                    Style::default().fg(t.text_muted),
                ));
                spans.push(Span::raw(" "));
                // Middle columns rendered muted as one block.
                let middle = parts[1..parts.len() - 1].join(" ");
                spans.push(Span::styled(middle, Style::default().fg(t.text_muted)));
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    name.to_owned(),
                    Style::default().fg(path_color(name, t)),
                ));
                lines.push(Line::from(spans));
                continue;
            }
        }
        // Simple path-per-line: tint by extension.
        let trimmed = raw.trim_end();
        if trimmed.is_empty() {
            lines.push(Line::from(""));
        } else {
            lines.push(Line::from(Span::styled(
                raw.to_owned(),
                Style::default().fg(path_color(trimmed, t)),
            )));
        }
    }
    if total > max_lines {
        lines.push(Line::from(Span::styled(
            format!(
                "… {} more lines · click or press o to expand",
                total - max_lines
            ),
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        )));
    }
    if !stderr.is_empty() {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        for sl in stderr.lines() {
            lines.push(Line::from(Span::styled(
                sanitize_terminal_text(sl),
                Style::default().fg(t.error),
            )));
        }
    }
    lines
}

/// Produce `git diff` / `git show` output as colored unified diff
/// lines. Each line gets a per-prefix color: `+` green, `-` red, `@@`
/// cyan, file headers bold, index/`diff --git` lines muted.
/// Parse the substring after `diff --git ` into `(a_path, b_path)`.
/// Handles quoted paths (Git emits these when `core.quotePath` triggers
/// on non-ASCII or whitespace) but skips C-string unescaping — only the
/// extension is needed downstream for syntect.
fn parse_diff_git_paths(rest: &str) -> Option<(String, String)> {
    let trimmed = rest.trim();
    let (a_quoted, after_a) = if let Some(stripped) = trimmed.strip_prefix('"') {
        let end = stripped.find('"')?;
        (&stripped[..end], stripped[end + 1..].trim_start())
    } else {
        let end = trimmed.find(' ')?;
        (&trimmed[..end], trimmed[end + 1..].trim_start())
    };
    let b_quoted = if let Some(stripped) = after_a.strip_prefix('"') {
        let end = stripped.find('"')?;
        &stripped[..end]
    } else {
        after_a.split_whitespace().next()?
    };
    let strip_prefix = |p: &str| -> String {
        p.strip_prefix("a/")
            .or_else(|| p.strip_prefix("b/"))
            .unwrap_or(p)
            .to_owned()
    };
    Some((strip_prefix(a_quoted), strip_prefix(b_quoted)))
}

pub(super) fn produce_git_diff_output_lines(
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
    t: Theme,
    expanded: bool,
) -> Vec<Line<'static>> {
    let max_lines = if expanded { 1000usize } else { 200usize };
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(code) = exit_code {
        // git diff exits 1 when there are differences (with --exit-code).
        // 0 = no diffs, 1 = diffs found, >1 = real error.
        if code > 1 {
            lines.push(Line::from(Span::styled(
                format!("[exit {code}]"),
                Style::default().fg(t.error),
            )));
        }
    }

    // Pre-pass: strip ANSI from every line so prefix matching, hunk
    // batching, and syntect highlighting all see the original source
    // text. `git diff --color=always` (and `git -c color.ui=always`)
    // wraps every line in SGR escapes (`\u{1b}[1m…\u{1b}[m`); without
    // this strip the prefix checks below never fire and the raw
    // escapes leak into the rendered Paragraph.
    let cleaned: Vec<String> = stdout.lines().map(sanitize_terminal_text).collect();
    let total = cleaned.len();

    let mut current_lang: Option<String> = None;
    let mut hunk: Vec<(DiffLineKind, String)> = Vec::new();

    let flush = |hunk: &mut Vec<(DiffLineKind, String)>,
                 lang: &Option<String>,
                 lines: &mut Vec<Line<'static>>,
                 max_lines: usize,
                 t: Theme| {
        if hunk.is_empty() {
            return;
        }
        let body: String = hunk
            .iter()
            .map(|(_, content)| content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let highlighted = lang.as_deref().and_then(|l| {
            // `wrap_w=0` → 1:1 line mapping from `highlight_code_raw`
            // (no wrapping inside syntect). The `hl.len() == hunk.len()`
            // invariant below relies on this contract.
            let hl = markdown::highlight_code_raw(l, &body, 0, &t);
            (hl.len() == hunk.len()).then_some(hl)
        });
        for (idx, (kind, content)) in hunk.drain(..).enumerate() {
            if lines.len() >= max_lines {
                break;
            }
            let (bg_color, fg_color, sigil) = match kind {
                DiffLineKind::Added => (t.code_bg, t.success, '+'),
                DiffLineKind::Removed => (t.code_bg, t.error, '-'),
                DiffLineKind::Context => (t.bg, t.text_secondary, ' '),
            };
            let mut spans: Vec<Span<'static>> = vec![Span::styled(
                format!("{sigil} "),
                Style::default().fg(fg_color).bg(bg_color),
            )];
            match highlighted.as_ref().and_then(|h| h.get(idx)) {
                Some(hl) => {
                    let extra_mod = matches!(kind, DiffLineKind::Removed).then_some(Modifier::DIM);
                    for sp in &hl.spans {
                        let mut style = sp.style;
                        style.bg = Some(bg_color);
                        if let Some(m) = extra_mod {
                            style = style.add_modifier(m);
                        }
                        spans.push(Span::styled(sp.content.clone().into_owned(), style));
                    }
                }
                None => {
                    spans.push(Span::styled(
                        content,
                        Style::default().fg(fg_color).bg(bg_color),
                    ));
                }
            }
            lines.push(Line::from(spans).style(Style::default().bg(bg_color)));
        }
    };

    for clean in &cleaned {
        if lines.len() >= max_lines && hunk.is_empty() {
            continue;
        }
        // Pull language from the b-side (post-edit) path so renames
        // pick up the destination's extension, not the source's.
        if let Some(rest) = clean.strip_prefix("diff --git ") {
            flush(&mut hunk, &current_lang, &mut lines, max_lines, t);
            current_lang = parse_diff_git_paths(rest).and_then(|(_, b)| lang_from_path(&b));
            if lines.len() < max_lines {
                lines.push(Line::from(Span::styled(
                    clean.clone(),
                    Style::default().fg(t.text_muted),
                )));
            }
            continue;
        }
        if clean.starts_with("index ")
            || clean.starts_with("new file mode ")
            || clean.starts_with("deleted file mode ")
            || clean.starts_with("old mode ")
            || clean.starts_with("new mode ")
            || clean.starts_with("similarity index ")
            || clean.starts_with("rename from ")
            || clean.starts_with("rename to ")
            || clean.starts_with("copy from ")
            || clean.starts_with("copy to ")
        {
            flush(&mut hunk, &current_lang, &mut lines, max_lines, t);
            if lines.len() < max_lines {
                lines.push(Line::from(Span::styled(
                    clean.clone(),
                    Style::default().fg(t.text_muted),
                )));
            }
            continue;
        }
        if clean.starts_with("--- ") || clean.starts_with("+++ ") {
            flush(&mut hunk, &current_lang, &mut lines, max_lines, t);
            if lines.len() < max_lines {
                lines.push(Line::from(Span::styled(
                    clean.clone(),
                    Style::default()
                        .fg(t.text_primary)
                        .add_modifier(Modifier::BOLD),
                )));
            }
            continue;
        }
        if clean.starts_with("@@") {
            flush(&mut hunk, &current_lang, &mut lines, max_lines, t);
            if lines.len() < max_lines {
                lines.push(Line::from(Span::styled(
                    clean.clone(),
                    Style::default().fg(t.accent),
                )));
            }
            continue;
        }
        if let Some(spans) = terminal_output::colorize_diffstat_line(clean, t.text_secondary, t) {
            flush(&mut hunk, &current_lang, &mut lines, max_lines, t);
            if lines.len() < max_lines {
                lines.push(Line::from(spans));
            }
            continue;
        }
        if let Some(content) = clean.strip_prefix('+') {
            hunk.push((DiffLineKind::Added, content.to_owned()));
        } else if let Some(content) = clean.strip_prefix('-') {
            hunk.push((DiffLineKind::Removed, content.to_owned()));
        } else if let Some(content) = clean.strip_prefix(' ') {
            hunk.push((DiffLineKind::Context, content.to_owned()));
        } else {
            flush(&mut hunk, &current_lang, &mut lines, max_lines, t);
            if lines.len() < max_lines {
                lines.push(Line::from(Span::styled(
                    clean.clone(),
                    Style::default().fg(t.text_muted),
                )));
            }
        }
    }
    flush(&mut hunk, &current_lang, &mut lines, max_lines, t);
    if total > max_lines {
        lines.push(Line::from(Span::styled(
            format!(
                "… {} more lines · click or press o to expand",
                total - max_lines
            ),
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        )));
    }
    if !stderr.is_empty() {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        for sl in stderr.lines() {
            lines.push(Line::from(Span::styled(
                sanitize_terminal_text(sl),
                Style::default().fg(t.error),
            )));
        }
    }
    lines
}

/// Produce `git log` output lines. Detects two formats:
///   - `--oneline`: `SHA message` — SHA in accent, rest plain
///   - default: `commit SHA\nAuthor: ...\nDate: ...\n\n    body\n`
///     — `commit` line in accent, Author/Date muted, body italic.
pub(super) fn produce_git_log_output_lines(
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
    t: Theme,
    expanded: bool,
) -> Vec<Line<'static>> {
    let max_lines = if expanded { 500usize } else { 100usize };
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(code) = exit_code {
        if code != 0 {
            lines.push(Line::from(Span::styled(
                format!("[exit {code}]"),
                Style::default().fg(t.error),
            )));
        }
    }
    let mut total = 0usize;
    for raw in stdout.lines() {
        total += 1;
        if lines.len() >= max_lines {
            continue;
        }
        // Default format heuristic: lines starting with `commit `
        // followed by a hex SHA; `Author:` / `Date:` headers; body
        // indented with 4 spaces; everything else default.
        if let Some(rest) = raw.strip_prefix("commit ") {
            // Split SHA from any trailing decorations like
            // `(HEAD -> main, origin/main)`.
            let (sha, decoration) = rest
                .split_once(' ')
                .map(|(s, d)| (s, Some(d)))
                .unwrap_or((rest, None));
            let mut spans = vec![
                Span::styled("commit ", Style::default().fg(t.text_muted)),
                Span::styled(
                    sha.to_owned(),
                    Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
                ),
            ];
            if let Some(d) = decoration {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(d.to_owned(), Style::default().fg(t.warning)));
            }
            lines.push(Line::from(spans));
        } else if raw.starts_with("Author:") || raw.starts_with("Date:") {
            lines.push(Line::from(Span::styled(
                raw.to_owned(),
                Style::default().fg(t.text_muted),
            )));
        } else if raw.starts_with("    ") {
            // 4-space-indented body line.
            lines.push(Line::from(Span::styled(
                raw.to_owned(),
                Style::default().fg(t.text_secondary),
            )));
        } else {
            // `--oneline` format: <SHA> <msg>. Sniff a short hex
            // SHA at the start.
            if let Some(space) = raw.find(' ') {
                let (head, tail) = raw.split_at(space);
                let head_clean = head.trim_end_matches(|c: char| !c.is_ascii_alphanumeric());
                if !head_clean.is_empty()
                    && head_clean.len() >= 6
                    && head_clean.len() <= 40
                    && head_clean.chars().all(|c| c.is_ascii_hexdigit())
                {
                    lines.push(Line::from(vec![
                        Span::styled(
                            head.to_owned(),
                            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(tail.to_owned(), Style::default().fg(t.text_secondary)),
                    ]));
                    continue;
                }
            }
            lines.push(Line::from(Span::styled(
                raw.to_owned(),
                Style::default().fg(t.text_secondary),
            )));
        }
    }
    if total > max_lines {
        lines.push(Line::from(Span::styled(
            format!(
                "… {} more lines · click or press o to expand",
                total - max_lines
            ),
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        )));
    }
    if !stderr.is_empty() {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        for sl in stderr.lines() {
            lines.push(Line::from(Span::styled(
                sanitize_terminal_text(sl),
                Style::default().fg(t.error),
            )));
        }
    }
    lines
}

/// Render `cat <markdown-file>` output as actual rendered markdown
/// (formatted headers, tables, code fences) instead of syntax-
/// highlighted source. The user expects `cat README.md` to show
/// the document the way the model's prose is shown — not the raw
/// `# Header` characters with syntax coloring. Mirrors v126's
/// markdown rendering for tool output.
pub(super) fn produce_cat_markdown_output_lines(
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
    content_w: usize,
    t: Theme,
) -> Vec<Line<'static>> {
    const MAX_LINES: usize = 500;
    let inner_w = content_w.saturating_sub(2);
    let mut lines: Vec<Line<'static>> = Vec::new();

    if let Some(code) = exit_code {
        if code != 0 {
            lines.push(Line::from(Span::styled(
                format!("[exit {code}]"),
                Style::default().fg(t.warning),
            )));
        }
    }

    let body = markdown::to_lines(stdout, &t, inner_w.max(1));
    lines.extend(body);

    if lines.len() > MAX_LINES {
        let total = lines.len();
        lines.truncate(MAX_LINES);
        lines.push(Line::from(Span::styled(
            format!(
                "… {} more lines · click or press o to expand",
                total - MAX_LINES
            ),
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    if !stderr.is_empty() {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        for sl in stderr.lines() {
            lines.push(Line::from(Span::styled(
                sanitize_terminal_text(sl),
                Style::default().fg(t.error),
            )));
        }
    }

    lines
}

/// Render `xxd` / `hexyl` / `od` hex-dump output. Each input line
/// has the canonical shape `OFFSET: BYTES  ASCII` (xxd) or hexyl's
/// boxed table form. We split on the first colon (offset/bytes) and
/// the doubled-space separator before the ASCII column, color each
/// region distinctly, and pass everything else through unstyled so
/// hexyl's box-drawing characters survive intact.
pub(super) fn produce_hex_dump_output_lines(
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
    t: Theme,
    expanded: bool,
) -> Vec<Line<'static>> {
    let max_lines = if expanded { 1000usize } else { 200usize };
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(code) = exit_code
        && code != 0
    {
        lines.push(Line::from(Span::styled(
            format!("[exit {code}]"),
            Style::default().fg(t.error),
        )));
    }
    let mut total = 0usize;
    for raw in stdout.lines() {
        total += 1;
        if lines.len() >= max_lines {
            continue;
        }
        // xxd canonical form: `00000000: 4865 6c6c 6f0a                           Hello.`
        // hexyl decorates with │ │ box separators — let those
        // pass through styled neutrally.
        if let Some((offset, rest)) = raw.split_once(':') {
            // Heuristic for the hex/ASCII split: xxd uses two
            // consecutive spaces, hexyl uses ` │ ` separators.
            let (bytes, ascii) = if let Some(idx) = rest.find("  ") {
                let (a, b) = rest.split_at(idx);
                (a, b.trim_start())
            } else if let Some(idx) = rest.find(" │ ") {
                let (a, b) = rest.split_at(idx);
                (a, &b[3..])
            } else {
                (rest, "")
            };
            // Sanity check: real offsets are mostly hex digits.
            // A non-hex prefix means we're looking at unrelated
            // output (stderr-style line) — fall back to plain.
            let looks_offset =
                !offset.is_empty() && offset.trim_start().chars().all(|c| c.is_ascii_hexdigit());
            if looks_offset {
                let mut spans = vec![
                    Span::styled(format!("{offset}:"), Style::default().fg(t.text_muted)),
                    Span::styled(bytes.to_owned(), Style::default().fg(t.accent)),
                ];
                if !ascii.is_empty() {
                    spans.push(Span::raw("  "));
                    spans.push(Span::styled(
                        ascii.to_owned(),
                        Style::default().fg(t.text_secondary),
                    ));
                }
                lines.push(Line::from(spans));
                continue;
            }
        }
        // hexyl header / footer / unknown line — keep raw.
        lines.push(Line::from(Span::styled(
            raw.to_owned(),
            Style::default().fg(t.text_muted),
        )));
    }
    if total > max_lines {
        lines.push(Line::from(Span::styled(
            format!(
                "… {} more lines · click or press o to expand",
                total - max_lines
            ),
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        )));
    }
    if !stderr.is_empty() {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        for sl in stderr.lines() {
            lines.push(Line::from(Span::styled(
                sanitize_terminal_text(sl),
                Style::default().fg(t.error),
            )));
        }
    }
    lines
}

/// Render `docker ps` / `docker images` / `kubectl get …` and
/// similar fixed-width tables. The first non-empty stdout line is
/// the column header (uppercase column names) — bold it and use the
/// accent color so it pops; body rows alternate between primary and
/// muted text so wide tables remain scannable. Container/pod state
/// columns get an extra tint when we recognise the value (`Running`,
/// `Up …`, `Exited`, `Error`, `CrashLoopBackOff`).
pub(super) fn produce_tabular_list_output_lines(
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
    t: Theme,
    expanded: bool,
) -> Vec<Line<'static>> {
    let max_lines = if expanded { 500usize } else { 100usize };
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(code) = exit_code
        && code != 0
    {
        lines.push(Line::from(Span::styled(
            format!("[exit {code}]"),
            Style::default().fg(t.error),
        )));
    }
    let mut header_drawn = false;
    let mut total = 0usize;
    for raw in stdout.lines() {
        total += 1;
        if lines.len() >= max_lines {
            continue;
        }
        if !header_drawn && !raw.trim().is_empty() {
            lines.push(Line::from(Span::styled(
                raw.to_owned(),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            )));
            header_drawn = true;
            continue;
        }
        // Tint a status word if we can spot one. We don't try to
        // parse columns — just look at the line for known tokens.
        let style = if raw.contains("CrashLoopBackOff")
            || raw.contains("Error")
            || raw.contains("Exited")
        {
            Style::default().fg(t.error)
        } else if raw.contains("Running") || raw.starts_with("Up ") || raw.contains(" Up ") {
            Style::default().fg(t.success)
        } else if raw.contains("Pending") || raw.contains("ContainerCreating") {
            Style::default().fg(t.warning)
        } else {
            Style::default().fg(t.text_primary)
        };
        lines.push(Line::from(Span::styled(raw.to_owned(), style)));
    }
    if total > max_lines {
        lines.push(Line::from(Span::styled(
            format!(
                "… {} more lines · click or press o to expand",
                total - max_lines
            ),
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        )));
    }
    if !stderr.is_empty() {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        for sl in stderr.lines() {
            lines.push(Line::from(Span::styled(
                sanitize_terminal_text(sl),
                Style::default().fg(t.error),
            )));
        }
    }
    lines
}

/// Render `cargo build` / `cargo test` / `cargo check` / `make` /
/// `npm run build` output. Routes recognised line shapes to colored
/// styles so the user can scan a long compile log at a glance:
///
///   * `Compiling foo v1.2.3` → muted (info, lots of these scroll by)
///   * `Finished … in N.NNs` / `Finished` → success green, bold
///   * `Building [...]` progress bars → accent
///   * `error[E0123]:` / `error: …` → error red, bold prefix
///   * `warning:` → warning yellow, bold prefix
///   * `note:` / `help:` → accent muted
///   * `--> path:line:col` location markers → accent
///   * `running N tests` / `test result: ok. N passed` → success
///   * `test foo::bar ... ok` → success; `... FAILED` → error
///   * `failures:` block headers → error
///   * Everything else → text_secondary
pub(super) fn produce_compiler_output_lines(
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
    t: Theme,
    expanded: bool,
) -> Vec<Line<'static>> {
    let max_lines = if expanded { 1500usize } else { 300usize };
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(code) = exit_code
        && code != 0
    {
        let badge_color = if code == 101 || code == 1 {
            t.error
        } else {
            t.warning
        };
        lines.push(Line::from(Span::styled(
            format!("[exit {code}]"),
            Style::default()
                .fg(badge_color)
                .add_modifier(Modifier::BOLD),
        )));
    }
    let mut total = 0usize;
    // `cargo` writes status to stderr (Compiling/Finished/warning),
    // diagnostics to stderr too, and final binary output to stdout.
    // Walk both streams in order — stderr first (the build log),
    // then stdout (test output, run output).
    for raw in stderr.lines().chain(stdout.lines()) {
        total += 1;
        if lines.len() >= max_lines {
            continue;
        }
        let trimmed = raw.trim_start();
        let leading_ws_len = raw.len() - trimmed.len();
        let leading = if leading_ws_len > 0 {
            &raw[..leading_ws_len]
        } else {
            ""
        };

        // Build progress: `Compiling foo v1.2.3` / `Building […]`
        // / `Downloading foo v1`. Use muted color so the dozens of
        // these don't dominate the log visually.
        if let Some(pkg) = trimmed.strip_prefix("Compiling ") {
            let mut spans = vec![
                Span::raw(leading.to_owned()),
                Span::styled(
                    "Compiling ".to_string(),
                    Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled(pkg.to_owned(), Style::default().fg(t.text_secondary)),
            ];
            // Trim line so spans length matches trimmed length
            let _ = &mut spans;
            lines.push(Line::from(spans));
            continue;
        }
        for prefix in &[
            "Checking ",
            "Building ",
            "Downloading ",
            "Updating ",
            "Verifying ",
            "Installing ",
            "Removing ",
            "Fresh ",
            "Documenting ",
        ] {
            if let Some(rest) = trimmed.strip_prefix(prefix) {
                lines.push(Line::from(vec![
                    Span::raw(leading.to_owned()),
                    Span::styled((*prefix).to_string(), Style::default().fg(t.text_muted)),
                    Span::styled(rest.to_owned(), Style::default().fg(t.text_muted)),
                ]));
                continue;
            }
        }

        // `Finished` (build success) / `Compiled` etc. — bold green.
        if trimmed.starts_with("Finished ")
            || trimmed.starts_with("Compiled ")
            || trimmed.starts_with("Built ")
        {
            lines.push(Line::from(Span::styled(
                raw.to_owned(),
                Style::default().fg(t.success).add_modifier(Modifier::BOLD),
            )));
            continue;
        }

        // Errors: `error[E0123]: …` and `error: …`. Color the
        // prefix red+bold and let the rest of the line read in
        // primary text so the message is legible.
        if let Some(rest) = trimmed.strip_prefix("error") {
            // Match `error[…]:` or `error:` — anything else is text.
            let after =
                rest.trim_start_matches(|c: char| c == '[' || c == ']' || c.is_alphanumeric());
            if rest.is_empty()
                || rest.starts_with(':')
                || rest.starts_with('[')
                || after.starts_with(':')
            {
                lines.push(Line::from(vec![
                    Span::raw(leading.to_owned()),
                    Span::styled(
                        format!("error{}", rest.split(':').next().unwrap_or("")),
                        Style::default().fg(t.error).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        rest.split_once(':')
                            .map(|(_, after)| format!(":{after}"))
                            .unwrap_or_default(),
                        Style::default().fg(t.text_primary),
                    ),
                ]));
                continue;
            }
        }
        if let Some(rest) = trimmed.strip_prefix("warning") {
            let after =
                rest.trim_start_matches(|c: char| c == '[' || c == ']' || c.is_alphanumeric());
            if rest.is_empty()
                || rest.starts_with(':')
                || rest.starts_with('[')
                || after.starts_with(':')
            {
                lines.push(Line::from(vec![
                    Span::raw(leading.to_owned()),
                    Span::styled(
                        format!("warning{}", rest.split(':').next().unwrap_or("")),
                        Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        rest.split_once(':')
                            .map(|(_, after)| format!(":{after}"))
                            .unwrap_or_default(),
                        Style::default().fg(t.text_primary),
                    ),
                ]));
                continue;
            }
        }
        // Diagnostic detail: `note:`, `help:` — softer color.
        if trimmed.starts_with("note:") || trimmed.starts_with("help:") {
            lines.push(Line::from(Span::styled(
                raw.to_owned(),
                Style::default().fg(t.accent),
            )));
            continue;
        }
        // Location pointer: `   --> src/foo.rs:12:5`. Pick out the
        // arrow and color the path/lineno region.
        if let Some(idx) = raw.find("--> ") {
            let (before, after) = raw.split_at(idx + 4);
            lines.push(Line::from(vec![
                Span::styled(before.to_owned(), Style::default().fg(t.text_muted)),
                Span::styled(after.to_owned(), Style::default().fg(t.accent)),
            ]));
            continue;
        }

        // `cargo test` results.
        if trimmed.starts_with("running ")
            && trimmed.ends_with(" tests")
            && !trimmed.contains("0 tests")
        {
            lines.push(Line::from(Span::styled(
                raw.to_owned(),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            )));
            continue;
        }
        if trimmed.starts_with("test ") {
            // `test foo::bar ... ok` / `... FAILED` / `... ignored`
            let style = if trimmed.contains(" ... ok") || trimmed.contains(" ... bench:") {
                Style::default().fg(t.success)
            } else if trimmed.contains(" ... FAILED") || trimmed.contains(" ... fail") {
                Style::default().fg(t.error).add_modifier(Modifier::BOLD)
            } else if trimmed.contains(" ... ignored") {
                Style::default().fg(t.text_muted)
            } else {
                Style::default().fg(t.text_secondary)
            };
            lines.push(Line::from(Span::styled(raw.to_owned(), style)));
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("test result:") {
            // `test result: ok. N passed; M failed; …`
            let body_color = if rest.contains(" FAILED") || rest.contains("failed; 0") {
                if rest.contains("0 failed") {
                    t.success
                } else {
                    t.error
                }
            } else if rest.contains(" ok") {
                t.success
            } else {
                t.warning
            };
            lines.push(Line::from(vec![
                Span::raw(leading.to_owned()),
                Span::styled(
                    "test result:".to_owned(),
                    Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    rest.to_owned(),
                    Style::default().fg(body_color).add_modifier(Modifier::BOLD),
                ),
            ]));
            continue;
        }
        if trimmed.starts_with("failures:") {
            lines.push(Line::from(Span::styled(
                raw.to_owned(),
                Style::default().fg(t.error).add_modifier(Modifier::BOLD),
            )));
            continue;
        }

        // Carat / pipe gutters from the rust diagnostic format —
        // they hint at code so let them inherit accent.
        if trimmed.starts_with('|') || trimmed.starts_with('=') || trimmed.starts_with("^") {
            lines.push(Line::from(Span::styled(
                raw.to_owned(),
                Style::default().fg(t.accent),
            )));
            continue;
        }

        lines.push(Line::from(Span::styled(
            raw.to_owned(),
            Style::default().fg(t.text_secondary),
        )));
    }
    if total > max_lines {
        lines.push(Line::from(Span::styled(
            format!(
                "… {} more lines · click or press o to expand",
                total - max_lines
            ),
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        )));
    }
    lines
}

/// Produce Bash output lines where stdout is the contents of a single
/// file (cat / head / tail). Top row is the exit-code badge, then
/// stdout flows through syntect highlighting (no line numbers — the
/// `cat` user opted out of those), then any stderr in red.
pub(super) fn produce_cat_output_lines(
    lang: &str,
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
    content_w: usize,
    t: Theme,
    expanded: bool,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let (code_str, code_style) = match exit_code {
        Some(0) => ("exit 0".to_owned(), Style::default().fg(t.success)),
        Some(n) => (format!("exit {n}"), Style::default().fg(t.error)),
        None => ("running…".to_owned(), Style::default().fg(t.text_muted)),
    };
    lines.push(Line::from(Span::styled(code_str, code_style)));

    let max_lines = if expanded { 500usize } else { 80usize };
    let mut highlighted = markdown::highlight_code_raw(lang, stdout, content_w, &t);
    let total = highlighted.len();
    let truncated = total > max_lines;
    if truncated {
        highlighted.truncate(max_lines);
    }
    lines.extend(highlighted);
    if truncated {
        lines.push(Line::from(Span::styled(
            format!(
                "… {} more lines · click or press o to expand",
                total - max_lines
            ),
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    if !stderr.is_empty() {
        lines.push(Line::from(Span::styled(
            "↳ stderr",
            Style::default()
                .fg(t.error)
                .add_modifier(Modifier::ITALIC | Modifier::BOLD),
        )));
        for line in stderr.lines().take(40) {
            lines.push(Line::from(Span::styled(
                line.to_owned(),
                Style::default().fg(t.error),
            )));
        }
    }
    lines
}

pub(super) fn produce_command_output_lines(
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
    content_w: usize,
    t: Theme,
    expanded: bool,
) -> Vec<Line<'static>> {
    use ansi_to_tui::IntoText;

    let mut lines: Vec<Line<'static>> = Vec::new();
    let w = content_w;

    let (code_str, code_style) = match exit_code {
        Some(0) => ("exit 0".to_owned(), Style::default().fg(t.success)),
        Some(n) => (format!("exit {n}"), Style::default().fg(t.error)),
        None => ("running…".to_owned(), Style::default().fg(t.text_muted)),
    };
    lines.push(Line::from(Span::styled(code_str, code_style)));

    let max_lines = if expanded { 500usize } else { 80usize };
    let mut body_lines: Vec<Line<'static>> = Vec::new();

    let push_styled = |raw: &str, fallback_style: Style, lines: &mut Vec<Line<'static>>| {
        let parsed = raw.into_text().ok();
        let source_lines: Vec<Line<'static>> = match parsed {
            Some(text) => text.lines.into_iter().collect(),
            None => raw
                .lines()
                .map(|l| Line::from(Span::styled(sanitize_terminal_text(l), fallback_style)))
                .collect(),
        };
        for line in source_lines {
            for wrapped in terminal_output::wrap_styled_line(&line, w.max(1)) {
                lines.push(wrapped);
            }
        }
    };

    push_styled(
        stdout,
        Style::default().fg(t.text_secondary),
        &mut body_lines,
    );
    if !stdout.is_empty() && !stderr.is_empty() {
        body_lines.push(Line::from(Span::styled(
            "↳ stderr",
            Style::default()
                .fg(t.error)
                .add_modifier(Modifier::ITALIC | Modifier::BOLD),
        )));
    }
    push_styled(stderr, Style::default().fg(t.error), &mut body_lines);

    lines.extend(terminal_output::truncate_lines_middle(
        body_lines,
        max_lines,
        Style::default().fg(t.text_muted),
    ));
    lines
}

/// Best-effort language detection for a diff view. Returns a token suitable
/// for `markdown::highlight_code_raw` (typically the file extension, falling
/// back to the filename for ext-less files like `Makefile`/`Dockerfile`).
/// Returns `None` for empty paths or paths with no recognizable token.
///
/// The returned string is *not* guaranteed to map to a real syntect syntax —
/// `highlight_code_raw` will fall back to plain text for unknowns. Keeping
/// this lossy on purpose: matching the syntect set up front would couple this
/// helper to syntect's loaded syntaxes, but the highlighter already does that
/// resolution downstream and degrades gracefully.
pub fn diff_lang(diff: &DiffView) -> Option<String> {
    let p = std::path::Path::new(&diff.file_path);
    if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
        if !ext.is_empty() {
            return Some(ext.to_string());
        }
    }
    // No extension — fall back to the filename (lowercased) so things like
    // `Makefile` / `Dockerfile` / `Rakefile` get a chance to resolve via
    // syntect's by-name / by-token lookup.
    p.file_name()
        .and_then(|f| f.to_str())
        .map(|f| f.to_lowercase())
        .filter(|s| !s.is_empty())
}

pub(super) fn produce_diff_view_lines(
    diff: &DiffView,
    t: Theme,
    expanded: bool,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let lang = diff_lang(diff);

    // Sub-status row: `□ Added N lines, removed M` matching v126's
    // `□ Added 3 lines` summary line under the Update title (cli.js
    // diff renderer). Skipped when both counts are zero (e.g. a
    // metadata-only edit).
    if diff.additions > 0 || diff.deletions > 0 {
        let mut parts: Vec<String> = Vec::new();
        if diff.additions > 0 {
            parts.push(format!(
                "Added {} {}",
                diff.additions,
                if diff.additions == 1 { "line" } else { "lines" }
            ));
        }
        if diff.deletions > 0 {
            parts.push(format!(
                "removed {} {}",
                diff.deletions,
                if diff.deletions == 1 { "line" } else { "lines" }
            ));
        }
        let summary = format!("□ {}", parts.join(", "));
        push_wrapped_styled_line(
            &mut lines,
            vec![Span::styled(summary, Style::default().fg(t.text_muted))],
            width,
            t.bg,
        );
    }

    for hunk in &diff.hunks {
        push_wrapped_styled_line(
            &mut lines,
            vec![Span::styled(
                sanitize_terminal_text(&hunk.header),
                Style::default().fg(t.text_muted),
            )],
            width,
            t.bg,
        );

        let hunk_cap = if expanded { 500 } else { 50 };
        let max_dl = hunk.lines.len().min(hunk_cap);

        // Per-hunk syntax highlighting. Build a single string containing all
        // line bodies (sigils stripped) joined by `\n`, then run syntect over
        // it once so multi-line constructs (block comments, raw strings,
        // here-docs) tokenize correctly across +/-/context boundaries. We
        // pass `wrap_w = 0` to disable hard-wrapping, guaranteeing a 1:1 map
        // from input lines to output lines that we can index into by row.
        // Mirrors codex's diff_render approach (codex-rs/tui/src/diff_render
        // .rs around the `hunk_syntax_lines` block).
        let highlighted: Option<Vec<Line<'static>>> = lang.as_deref().and_then(|l| {
            let visible = &hunk.lines[..max_dl];
            let hunk_text: String = visible
                .iter()
                .map(|dl| sanitize_terminal_text(&dl.content))
                .collect::<Vec<_>>()
                .join("\n");
            let lines = markdown::highlight_code_raw(l, &hunk_text, 0, &t);
            // Defensive: if line counts don't agree (shouldn't happen with
            // wrap_w=0, but syntect can occasionally produce extra rows on
            // pathological inputs), bail and let the unhighlighted branch
            // render. Better plain than misaligned.
            (lines.len() == visible.len()).then_some(lines)
        });

        for (idx, dl) in hunk.lines.iter().take(max_dl).enumerate() {
            let (bg_color, fg_color, sigil) = match dl.kind {
                DiffLineKind::Added => (t.code_bg, t.success, "+"),
                DiffLineKind::Removed => (t.code_bg, t.error, "-"),
                DiffLineKind::Context => (t.bg, t.text_secondary, " "),
            };
            // Line-number column matches v126's diff style — show
            // the `new_line` for added/context (the post-edit location)
            // and `old_line` for removed (the source location).
            let lineno = match dl.kind {
                DiffLineKind::Removed => dl.old_line,
                _ => dl.new_line,
            };

            let mut content_spans: Vec<Span<'static>> = Vec::new();
            match highlighted.as_ref().and_then(|h| h.get(idx)) {
                Some(hl) => {
                    // Span composition: keep syntect's foreground, force
                    // the diff bg tint over it, and dim removed lines so
                    // deletions read as fading out.
                    let extra_mod =
                        matches!(dl.kind, DiffLineKind::Removed).then_some(Modifier::DIM);
                    for sp in &hl.spans {
                        let mut style = sp.style;
                        style.bg = Some(bg_color);
                        if let Some(m) = extra_mod {
                            style = style.add_modifier(m);
                        }
                        content_spans.push(Span::styled(sp.content.clone().into_owned(), style));
                    }
                }
                None => {
                    content_spans.push(Span::styled(
                        sanitize_terminal_text(&dl.content),
                        Style::default().fg(fg_color).bg(bg_color),
                    ));
                }
            }

            push_wrapped_diff_data_line(
                &mut lines,
                lineno,
                sigil,
                fg_color,
                bg_color,
                t.text_muted,
                content_spans,
                width,
            );
        }

        if hunk.lines.len() > hunk_cap {
            push_wrapped_styled_line(
                &mut lines,
                vec![Span::styled(
                    format!("… {} more lines", hunk.lines.len() - hunk_cap),
                    Style::default().fg(t.text_muted),
                )],
                width,
                t.bg,
            );
        }
    }
    lines
}

pub(super) fn render_diff_skip(
    diff: &DiffView,
    area: Rect,
    t: Theme,
    buf: &mut Buffer,
    skip: usize,
    expanded: bool,
) {
    let lines = produce_diff_view_lines(diff, t, expanded, area.width as usize);
    let bottom = area.y + area.height;
    for (row_idx, line) in lines.into_iter().enumerate().skip(skip) {
        let screen_y = area.y + (row_idx - skip) as u16;
        if screen_y >= bottom {
            break;
        }
        let row = Rect {
            x: area.x,
            y: screen_y,
            width: area.width,
            height: 1,
        };
        let row_bg = line.style.bg.unwrap_or(t.bg);
        Paragraph::new(line)
            .style(Style::default().bg(row_bg))
            .render(row, buf);
    }
}

pub(super) fn produce_file_list_lines(files: &[String], t: Theme) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    for f in files.iter().take(20) {
        lines.push(Line::from(Span::styled(
            sanitize_terminal_text(f),
            Style::default().fg(t.text_secondary),
        )));
    }
    if files.len() > 20 {
        lines.push(Line::from(Span::styled(
            format!("… {} more", files.len() - 20),
            Style::default().fg(t.text_muted),
        )));
    }
    lines
}
