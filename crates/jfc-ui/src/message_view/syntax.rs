use super::*;

pub(super) fn produce_highlighted_with_line_numbers_lines(
    lang: &str,
    text: &str,
    content_w: usize,
    t: Theme,
    expanded: bool,
    diag_lines: &std::collections::HashMap<usize, crate::diagnostics::Severity>,
) -> Vec<Line<'static>> {
    let (line_numbers, code) = split_line_numbers(text);
    let code_ref = code.as_deref().unwrap_or(text);

    let gutter_width = line_numbers
        .as_ref()
        .map(|nums| nums.iter().map(|n| n.len()).max().unwrap_or(0))
        .unwrap_or(0);

    // When we have any diagnostics for this file, reserve one column
    // for the severity glyph between the line number and separator
    // (` 12 ✘ │ `). When no diagnostics, the gutter stays at the
    // existing width so unaffected reads don't shift.
    let has_diags = !diag_lines.is_empty();
    let glyph_w: usize = if has_diags { 2 } else { 0 };
    let gutter_cols = if gutter_width > 0 {
        gutter_width + 3 + glyph_w
    } else {
        2
    };
    let code_w = content_w.saturating_sub(gutter_cols).max(10);

    let max_lines = if expanded { 500usize } else { 80usize };
    let highlighted = markdown::highlight_code_raw(lang, code_ref, code_w, &t);
    let total = highlighted.len();
    let truncated = total > max_lines;
    let take_n = total.min(max_lines);

    let gutter_style = Style::default().fg(t.text_muted);
    let separator_style = Style::default().fg(t.border);

    let mut lines: Vec<Line<'static>> = highlighted
        .into_iter()
        .take(take_n)
        .enumerate()
        .map(|(i, mut hl_line)| {
            let mut spans = if let Some(nums) = &line_numbers {
                let num_str = nums.get(i).map(|s| s.as_str()).unwrap_or("");
                let mut spans_init = vec![Span::styled(
                    format!("{:>width$}", num_str, width = gutter_width),
                    gutter_style,
                )];
                if has_diags {
                    let lineno: usize = num_str.parse().unwrap_or(0);
                    let (glyph, color) = match diag_lines.get(&lineno) {
                        Some(crate::diagnostics::Severity::Error) => ("✘", t.error),
                        Some(crate::diagnostics::Severity::Warning) => ("⚠", t.warning),
                        Some(crate::diagnostics::Severity::Info) => ("ℹ", t.accent),
                        Some(crate::diagnostics::Severity::Hint) => ("★", t.text_secondary),
                        None => (" ", t.text_muted),
                    };
                    spans_init.push(Span::styled(
                        format!(" {glyph}"),
                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                    ));
                }
                spans_init.push(Span::styled(" │ ", separator_style));
                spans_init
            } else {
                vec![Span::styled("│ ", separator_style)]
            };
            spans.extend(hl_line.spans.drain(..));
            Line::from(spans)
        })
        .collect();

    if truncated {
        lines.push(Line::from(Span::styled(
            format!(
                "… {} more lines · click or press o to expand",
                total - take_n
            ),
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        )));
    }
    lines
}

pub(super) fn split_line_numbers(text: &str) -> (Option<Vec<String>>, Option<String>) {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return (None, None);
    }
    let mut numbers = Vec::with_capacity(lines.len());
    let mut code_lines = Vec::with_capacity(lines.len());

    for line in &lines {
        if line.is_empty() {
            numbers.push(String::new());
            code_lines.push("");
            continue;
        }
        match line.find(": ") {
            Some(pos) if line[..pos].bytes().all(|b| b.is_ascii_digit()) => {
                numbers.push(line[..pos].to_string());
                code_lines.push(&line[pos + 2..]);
            }
            _ => return (None, None),
        }
    }
    (Some(numbers), Some(code_lines.join("\n")))
}

pub(super) fn infer_lang_from_tool(tool: &ToolCall) -> Option<String> {
    let path: &str = match &tool.input {
        ToolInput::Read { file_path, .. } => file_path.as_str(),
        ToolInput::Edit { file_path, .. } => file_path.as_str(),
        ToolInput::Write { file_path, .. } => file_path.as_str(),
        // Bash: when the user runs `cat path/file.ext`, `head -N file`,
        // or `tail file`, the stdout *is* the file content. Sniff
        // the command for one of those shapes and pull out the path
        // so the output gets the same language treatment as a Read.
        // Mirrors v126's bash → file-content highlighting heuristic.
        ToolInput::Bash { command, .. } => {
            return infer_lang_from_bash(command);
        }
        _ => return None,
    };
    lang_from_path(path)
}

pub(super) fn lang_from_path(path: &str) -> Option<String> {
    let p = std::path::Path::new(path);
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_string())
        .or_else(|| {
            p.file_name()
                .and_then(|f| f.to_str())
                .map(|f| f.to_string())
        })
}

/// Quote-aware tokenizer. Splits `cmd` on whitespace except inside
/// matched single- or double-quoted segments, which are emitted as
/// a single token. `awk '{print $1}' file` → `["awk", "'{print $1}'",
/// "file"]`. Backslashes escape the next char outside quotes. We
/// keep the quote characters in the returned token so callers can
/// still detect "this token was quoted" by its leading char.
pub(super) fn quote_aware_tokens(cmd: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut chars = cmd.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        match (quote, c) {
            (None, ws) if ws.is_whitespace() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            (None, '\'') | (None, '"') => {
                cur.push(c);
                quote = Some(c);
            }
            (Some(q), c2) if c2 == q => {
                cur.push(c2);
                quote = None;
            }
            (None, '\\') => {
                cur.push('\\');
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Replace the contents of every single- and double-quoted segment
/// in `cmd` with spaces, preserving the surrounding quotes and the
/// original length. Used to make the dangerous-meta-character checks
/// (`$`, `;`, etc.) quote-aware: `sed -n '1,$p' file` is a perfectly
/// safe sed call but the `$` lives inside `'…'` so we shouldn't
/// reject it. Without this, the canonical sed/awk idiom defeats the
/// language-inference path and the file falls back to plain rendering.
pub(super) fn redact_quoted(cmd: &str) -> String {
    let mut out = String::with_capacity(cmd.len());
    let mut chars = cmd.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        match (quote, c) {
            (None, '\'') | (None, '"') => {
                out.push(c);
                quote = Some(c);
            }
            (Some(q), c2) if c2 == q => {
                out.push(c2);
                quote = None;
            }
            (Some(_), _) => out.push(' '),
            (None, '\\') => {
                // Skip the next char so an escaped quote doesn't
                // start a fake quoted segment.
                out.push('\\');
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            (None, _) => out.push(c),
        }
    }
    out
}

/// Recognise `cat <file>` / `head <file>` / `tail <file>` commands
/// (with or without flags) and return the inferred language. Skips
/// when the command does anything fancier (pipes, redirects, multi-
/// file cats) — those need their own treatment, and over-applying
/// syntax highlighting to e.g. piped output breaks readability.
pub(super) fn infer_lang_from_bash(command: &str) -> Option<String> {
    // Pipeline + chain aware. `cmd1 || cmd2` takes cmd1; `cmd | less`
    // takes cmd; `cd X && cat README.md` takes the LAST segment
    // (the cat). Same logic as `classify_bash_cmd` so the two
    // dispatch paths agree.
    let primary_alt = command
        .split("||")
        .next()
        .unwrap_or(command)
        .split('|')
        .next()
        .unwrap_or(command);
    let primary = primary_alt
        .split("&&")
        .filter(|s| !s.trim().is_empty())
        .last()
        .unwrap_or(primary_alt);
    let trimmed = primary.trim();

    // Reject command-substitution / backticks / lone `&` / `;` —
    // those still indicate the cat is wrapped in something funky
    // and the file-path sniff would lie. `&&` was already split
    // out so any `&` here is the lone-background form. Check
    // *outside* quoted strings so `sed -n '1,$p' file.md` (the
    // canonical "print all lines" idiom) doesn't get rejected for
    // its quoted `$`.
    let probe = redact_quoted(trimmed);
    if probe.contains('$') || probe.contains('`') || probe.contains('&') || probe.contains(';') {
        return None;
    }
    // Strip stderr-redirect tokens like `2>/dev/null` or `2>&1`
    // so the file-path sniff works on the cat side. We tokenize
    // *quote-aware* so awk's `'{print $1}'` (which contains a
    // whitespace) stays a single token instead of fragmenting and
    // confusing the file-path sniff.
    let toks: Vec<String> = quote_aware_tokens(trimmed)
        .into_iter()
        .filter(|t| !t.starts_with("2>") && !t.starts_with('>'))
        .collect();
    let mut it = toks.iter().map(|s| s.as_str());
    let verb = it.next()?;
    if !matches!(
        verb,
        "cat"
            | "head"
            | "tail"
            | "bat"
            | "less"
            | "more"
            | "sed"
            | "awk"
            | "perl"
            | "jq"
            | "yq"
            | "python"
            | "python3"
            | "node"
    ) {
        return None;
    }

    // jq/yq always output JSON/YAML
    if matches!(verb, "jq") {
        return Some("json".to_string());
    }
    if matches!(verb, "yq") {
        return Some("yaml".to_string());
    }
    // python/node inline scripts — highlight as that language
    if matches!(verb, "python" | "python3") {
        return Some("python".to_string());
    }
    if matches!(verb, "node") {
        return Some("javascript".to_string());
    }
    // Pick the file-path argument. For most verbs the first
    // non-flag/non-numeric token is the file. For sed/awk/perl the
    // FIRST positional is the script (`'1,$p'`, `'{print}'`, ...);
    // the file is the next positional. Detect a script positional
    // by its leading quote character (the tokenizer kept quotes
    // because we split on whitespace, not via a real shell parser).
    let script_verb = matches!(verb, "sed" | "awk" | "perl");
    let mut seen_positional = false;
    let mut file: Option<&str> = None;
    for arg in it {
        if arg.starts_with('-') {
            continue;
        }
        if arg.parse::<i64>().is_ok() {
            continue;
        }
        // For sed/awk/perl: skip the first positional iff it looks
        // like a script (starts with a quote). A bare path with no
        // surrounding quotes still wins, so `awk file.txt` works
        // (degenerate but harmless).
        if script_verb && !seen_positional && (arg.starts_with('\'') || arg.starts_with('"')) {
            seen_positional = true;
            continue;
        }
        file = Some(arg);
        break;
    }
    let path = file?;
    lang_from_path(path)
}

/// Heuristic: does this text look like markdown content? Used when
/// the file path didn't tell us (e.g. `.sisyphus`, `README` with no
/// extension, hidden dotfile that happens to be MD). Counts the
/// most distinctive markers in the first 2KB so a long file's
/// detection is cheap.
pub(super) fn looks_like_markdown(text: &str) -> bool {
    let prefix: &str = if text.len() > 2048 {
        &text[..2048]
    } else {
        text
    };
    let mut score = 0;
    // Header lines are the strongest signal — `# ` / `## ` at start
    // of any line is rare in non-markdown text.
    for line in prefix.lines().take(60) {
        let l = line.trim_start();
        if l.starts_with("# ") || l.starts_with("## ") || l.starts_with("### ") {
            score += 2;
        }
        if l.starts_with("- ") || l.starts_with("* ") {
            score += 1;
        }
        if l.starts_with("```") {
            score += 2;
        }
        if l.contains("**") {
            score += 1;
        }
        if l.contains("|") && l.contains("---") {
            // Table separator row.
            score += 2;
        }
    }
    score >= 4
}

pub(super) fn produce_highlighted_block_lines(
    lang: &str,
    code: &str,
    content_w: usize,
    t: Theme,
    expanded: bool,
) -> Vec<Line<'static>> {
    let inner_w = content_w.saturating_sub(2);
    let max_lines = if expanded { 500usize } else { 80usize };
    let mut lines = markdown::highlight_code(lang, code, inner_w, &t);
    let total = lines.len();
    if total > max_lines {
        lines.truncate(max_lines);
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
