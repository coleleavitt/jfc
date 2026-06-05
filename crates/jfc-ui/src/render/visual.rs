use super::*;
pub fn input_line_to_spans(line: &str, t: Theme) -> Vec<Span<'static>> {
    if line.is_empty() {
        return vec![Span::raw("")];
    }
    let trimmed_start = line.trim_start();
    let leading_ws = line.len() - trimmed_start.len();
    let starts_with_slash = trimmed_start.starts_with('/');
    let mut spans: Vec<Span<'static>> = Vec::new();

    if leading_ws > 0 {
        spans.push(Span::raw(line[..leading_ws].to_string()));
    }

    if starts_with_slash {
        // Find end of the slash-command token (next whitespace).
        let token_end = trimmed_start
            .find(char::is_whitespace)
            .unwrap_or(trimmed_start.len());
        let token = &trimmed_start[..token_end];
        // Slash commands get one honest accent color, bold — enough to
        // mark the token as special without the per-char rainbow sweep
        // (which animated off `phase` for no informational reason).
        spans.push(Span::styled(
            token.to_string(),
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ));
        let rest = &trimmed_start[token_end..];
        if !rest.is_empty() {
            spans.extend(highlight_mentions_in(rest, t));
        }
    } else {
        spans.extend(highlight_mentions_in(trimmed_start, t));
    }
    spans
}

/// Tokenize prose and color any `@token` (mention) in the accent color,
/// bold — so a file/agent mention reads as a distinct reference rather
/// than plain text. One flat color, no animated gradient.
pub fn highlight_mentions_in(s: &str, t: Theme) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '@' && (i == 0 || chars[i - 1].is_whitespace()) {
            if !buf.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut buf), t.style_text_primary));
            }
            // Consume the `@` and the following non-whitespace token.
            let mut token = String::from('@');
            i += 1;
            while i < chars.len() && !chars[i].is_whitespace() {
                token.push(chars[i]);
                i += 1;
            }
            // One honest accent color for the whole `@mention`, bold —
            // marks it as a reference without the animated rainbow gradient.
            spans.push(Span::styled(
                token,
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ));
        } else {
            buf.push(c);
            i += 1;
        }
    }
    if !buf.is_empty() {
        spans.push(Span::styled(buf, t.style_text_primary));
    }
    spans
}

/// Prompt-character animation mode. Selects which glyph (or glyph
/// cycle) appears at the start of the input. Picked by parsing the
/// `JFC_PROMPT_CHAR` env var: a leading `:` denotes a named animation
/// preset; anything else is treated as a literal char.
#[derive(Clone, Debug)]
pub enum PromptMode {
    Comet,
    Moon,
    Dice,
    Notes,
    Hourglass,
    Atom,
    /// Static literal — user picked a single char (e.g. `JFC_PROMPT_CHAR=⌬`).
    Static(String),
}

pub fn parse_prompt_mode(raw: &str) -> PromptMode {
    let trimmed = raw.trim();
    match trimmed {
        ":comet" => PromptMode::Comet,
        ":moon" | ":moons" | ":moon_phases" => PromptMode::Moon,
        ":dice" | ":die" => PromptMode::Dice,
        ":notes" | ":music" => PromptMode::Notes,
        ":hourglass" | ":time" => PromptMode::Hourglass,
        ":atom" => PromptMode::Atom,
        s if !s.is_empty() && s.chars().count() <= 2 => PromptMode::Static(s.to_owned()),
        _ => PromptMode::Comet,
    }
}

/// Pick the glyph for this frame given the mode + wall-clock + state.
pub fn prompt_mode_frame(mode: &PromptMode, streaming: bool, ms: u128) -> &'static str {
    match mode {
        PromptMode::Comet => "☄",
        PromptMode::Atom => "⚛",
        PromptMode::Moon => {
            // 8-frame waxing/waning cycle that mirrors actual moon
            // phase order. Uses 1-cell symbolic glyphs (not emoji)
            // so ratatui's column tracking stays accurate. Idle
            // settles on full moon (`●`) — most "present" looking.
            if !streaming {
                return "●";
            }
            const FRAMES: &[&str] = &["○", "◐", "●", "◑"];
            FRAMES[((ms / 250) as usize) % FRAMES.len()]
        }
        PromptMode::Dice => {
            // Dice rolling at 120ms/face for a fast shuffle that
            // reads as "the model is thinking, anything could come
            // out". Idle lands on ⚀ so the prompt is visually
            // stable when nothing is happening.
            if !streaming {
                return "⚀";
            }
            const FACES: &[&str] = &["⚀", "⚁", "⚂", "⚃", "⚄", "⚅"];
            FACES[((ms / 120) as usize) % FACES.len()]
        }
        PromptMode::Notes => {
            // Music-note cycle at 280ms/note — slightly slower so
            // each glyph reads. Idle settles on ♪ (eighth note) as
            // the most "musical" looking single character.
            if !streaming {
                return "♪";
            }
            const NOTES: &[&str] = &["♩", "♪", "♫", "♬"];
            NOTES[((ms / 280) as usize) % NOTES.len()]
        }
        PromptMode::Hourglass => {
            // Flip every 800ms — `⌛` (sand running) → `⌚` (drained
            // / time face). Slow enough to read each state. Idle
            // shows the full hourglass.
            if !streaming {
                return "⌛";
            }
            if (ms / 800).is_multiple_of(2) {
                "⌛"
            } else {
                "⌚"
            }
        }
        PromptMode::Static(_) => {
            // Static is handled via fallback below (returns the
            // user-supplied char). Sentinel here for the type to
            // line up; the input renderer reads
            // `prompt_mode_frame_static` for this branch.
            ""
        }
    }
}

/// Linear-interpolate between two ratatui Colors at `t ∈ [0, 1]`.
/// Falls back to the start color when either endpoint isn't an RGB
/// triple (named ANSI colors don't have a useful midpoint). Used by
/// the spinner pulse to blend the lead glyph between accent and
/// warning across each animation cycle.
pub fn pulse_color(c1: Color, c2: Color, t: f32) -> Color {
    let (r1, g1, b1) = match c1 {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => return c1,
    };
    let (r2, g2, b2) = match c2 {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => return c1,
    };
    let (r, g, b) = crate::spinner::interpolate_rgb((r1, g1, b1), (r2, g2, b2), t);
    Color::Rgb(r, g, b)
}

pub fn gauge_color(pct: f64, t: crate::theme::Theme) -> Color {
    if pct >= 85.0 {
        t.error
    } else if pct >= 60.0 {
        t.warning
    } else {
        t.success
    }
}

pub fn fmt_number(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        let s = n.to_string();
        let mut out = String::with_capacity(s.len() + s.len() / 3);
        for (i, c) in s.chars().rev().enumerate() {
            if i > 0 && i % 3 == 0 {
                out.push(',');
            }
            out.push(c);
        }
        out.chars().rev().collect()
    } else {
        n.to_string()
    }
}

/// Aggregate edit/write diff stats across the whole conversation for the
/// sidebar "Changes" section and the footer `+N/−M` indicator. Walks every
/// Tool message part, picks up `ToolOutput::Diff(_)` payloads (Edit/Write
/// tools convert their result into a unified diff at parse time — see
/// `types.rs::ToolOutput::Diff`), and **sums** every edit's additions /
/// deletions per file. Each `DiffView` is a per-edit-local delta, so the
/// total is a session activity counter (lines churned this session) — CC
/// 2.1.154 parity (cli.js:266415 `linesAdded += z`). `total_files` still
/// dedups by path. Files appear in *most-recent-first* order to match how
/// the chat scrolls.
#[derive(Clone)]
pub struct DiffStats {
    pub total_files: usize,
    pub additions: usize,
    pub deletions: usize,
    pub files: Vec<String>,
}

/// Cached wrapper around the full diff-stats walk.
///
/// Complexity reduction: O(N_messages × N_parts) → O(1) cache hit on
/// unchanged state. Invalidates when `messages.len()` or the total
/// number of message parts changes (new message appended, or a tool
/// result added to an in-flight assistant message). The key computation
/// is O(N_messages) but touches only `.parts.len()` — no content
/// inspection — so it's negligible compared to the full HashMap walk.
pub fn collect_diff_stats(app: &App) -> DiffStats {
    let msg_count = app.messages.len();
    let total_parts: usize = app.messages.iter().map(|m| m.parts.len()).sum();

    {
        let cache = app.diff_stats_cache.borrow();
        if let Some((cached_msgs, cached_parts, ref stats)) = *cache
            && cached_msgs == msg_count
            && cached_parts == total_parts
        {
            return stats.clone();
        }
    }

    let stats = compute_diff_stats(app);
    *app.diff_stats_cache.borrow_mut() = Some((msg_count, total_parts, stats.clone()));
    stats
}

/// Inner computation for `collect_diff_stats`. Walks all messages and
/// parts to build the de-duplicated diff summary.
pub fn compute_diff_stats(app: &App) -> DiffStats {
    let mut by_file: std::collections::HashMap<String, (usize, usize)> =
        std::collections::HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for msg in &app.messages {
        for part in &msg.parts {
            if let MessagePart::Tool(call) = part
                && let ToolOutput::Diff(view) = &call.output
            {
                // Sum every edit's lines per file (CC 2.1.154 parity —
                // cli.js:266415 `linesAdded += z`). Each `DiffView` is
                // computed locally against the file's state right before
                // *that* edit (filesystem.rs build_edit_diff_view), so the
                // footer is a session ACTIVITY counter: total lines churned.
                // The previous `*entry = (...)` kept only the last edit per
                // file, silently hiding every earlier edit's lines on any
                // file touched more than once.
                let entry = by_file.entry(view.file_path.clone()).or_insert((0, 0));
                entry.0 += view.additions;
                entry.1 += view.deletions;
                if !order.contains(&view.file_path) {
                    order.push(view.file_path.clone());
                }
            }
        }
    }
    // Reverse so most-recently-touched files appear first.
    order.reverse();
    let (additions, deletions) = by_file
        .values()
        .fold((0usize, 0usize), |(a, d), (na, nd)| (a + na, d + nd));
    DiffStats {
        total_files: by_file.len(),
        additions,
        deletions,
        files: order,
    }
}

pub fn mcp_status_color(status: McpStatus, theme: Theme) -> Color {
    match status {
        McpStatus::Connected => theme.success,
        McpStatus::Disabled => theme.text_muted,
        McpStatus::Error => theme.error,
    }
}

pub fn truncate_str(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_owned()
    } else {
        let trunc: String = chars[..max.saturating_sub(1)].iter().collect();
        format!("{}…", trunc)
    }
}

/// Display width of a string in terminal cells. Unlike `.len()` (bytes)
/// or `.chars().count()` (codepoints), this counts the columns the text
/// actually occupies — CJK / fullwidth / emoji glyphs are 2 cells, the
/// multibyte box/bullet glyphs (`▶ ● ○ ✓ ✗`) are 1 cell each. Use this
/// for ALL layout math (padding, right-alignment, budget) so columns
/// line up regardless of content. `unicode_width` is already a crate
/// dep (see `input_box.rs`).
pub fn cell_width(s: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(s)
}

/// Truncate `s` so its display width fits in `max` cells, appending `…`
/// (itself 1 cell) when clipped. Cell-accurate counterpart to
/// `truncate_str`, which counts codepoints and so over/undershoots on
/// multi-cell text.
pub fn truncate_cells(s: &str, max: usize) -> String {
    use unicode_width::UnicodeWidthChar;
    if max == 0 {
        return String::new();
    }
    if cell_width(s) <= max {
        return s.to_owned();
    }
    // Reserve the last cell for the ellipsis.
    let budget = max.saturating_sub(1);
    let mut acc = 0usize;
    let mut out = String::new();
    for ch in s.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if acc + w > budget {
            break;
        }
        acc += w;
        out.push(ch);
    }
    out.push('…');
    out
}

/// Like `truncate_str` but clips from the *front*, prepending `…/`
/// so the meaningful tail (project name in a path, identifier in a
/// long namespace) survives. Used by the sidebar's cwd display so
/// the user sees `…/active/jfc` on a narrow column rather than the
/// useless `~/RustProjec…` head.
pub fn tail_truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_owned();
    }
    // Reserve 2 cells for the leading "…/" indicator. If the column
    // is too narrow even for that, fall back to head truncation.
    if max < 4 {
        return truncate_str(s, max);
    }
    let tail_len = max.saturating_sub(2);
    let start = chars.len() - tail_len;
    let tail: String = chars[start..].iter().collect();
    format!("…/{}", tail.trim_start_matches('/'))
}

/// Word-wrap a short prose string to a column-width. Used by the
/// info-sidebar's empty-state hints (e.g. "LSPs will activate as
/// files are read") that the parent Paragraph doesn't auto-wrap. A
/// hard ratatui clip would chop mid-word at the right edge; this
/// breaks on whitespace so each row is a complete fragment. Returns
/// at least one row even for empty input so callers can always
/// `.push(Line::from(row))`.
pub fn wrap_text_to_width(s: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in s.split_whitespace() {
        let word_len = word.chars().count();
        if word_len >= width {
            // Single-word overflow: hard-truncate that word with an
            // ellipsis. Better than letting it bleed off the edge.
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            out.push(truncate_str(word, width));
            continue;
        }
        let projected = if current.is_empty() {
            word_len
        } else {
            current.chars().count() + 1 + word_len
        };
        if projected > width {
            out.push(std::mem::take(&mut current));
            current.push_str(word);
        } else {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}
