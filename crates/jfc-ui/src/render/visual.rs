use super::*;
pub fn input_line_to_spans(line: &str, t: Theme, phase: f32) -> Vec<Span<'static>> {
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
        for (i, ch) in token.chars().enumerate() {
            let hue = (phase + i as f32 * 18.0) % 360.0;
            let (r, g, b) = crate::spinner::hue_to_rgb(hue);
            spans.push(Span::styled(
                ch.to_string(),
                Style::default()
                    .fg(Color::Rgb(r, g, b))
                    .add_modifier(Modifier::BOLD),
            ));
        }
        let rest = &trimmed_start[token_end..];
        if !rest.is_empty() {
            spans.extend(highlight_mentions_in(rest, t, phase));
        }
    } else {
        spans.extend(highlight_mentions_in(trimmed_start, t, phase));
    }
    spans
}

/// Tokenize prose, color any `@token` (mention) with the same rainbow
/// gradient as the leading slash command, but with a phase offset so
/// each mention reads as its own colored token rather than blending
/// in with the slash prefix.
pub fn highlight_mentions_in(s: &str, t: Theme, phase: f32) -> Vec<Span<'static>> {
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
            for (j, ch) in token.chars().enumerate() {
                let hue = (phase + 60.0 + j as f32 * 18.0) % 360.0;
                let (r, g, b) = crate::spinner::hue_to_rgb(hue);
                spans.push(Span::styled(
                    ch.to_string(),
                    Style::default()
                        .fg(Color::Rgb(r, g, b))
                        .add_modifier(Modifier::BOLD),
                ));
            }
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

/// Enumerate every cell along the border of `area` in clockwise
/// order, starting at the top-left corner. Used by the border-comet
/// painter to walk the perimeter at a steady speed regardless of
/// rect aspect ratio.
#[allow(dead_code)]
pub fn perimeter_cells(area: Rect) -> Vec<(u16, u16)> {
    // Per-frame cache: the input dock + status bar reuse the same Rect on
    // back-to-back frames. Without this, every frame allocated and filled a
    // ~2 × (w + h) Vec just to walk the same perimeter — pure waste during
    // idle/streaming where geometry is fixed. Invalidate on Rect change
    // (resize, layout shift); LRU-of-1 is enough since paint_border_comets
    // is the only non-test caller.
    thread_local! {
        static LAST: std::cell::RefCell<Option<(Rect, Vec<(u16, u16)>)>> =
            const { std::cell::RefCell::new(None) };
    }
    LAST.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some((cached_area, cached_cells)) = slot.as_ref()
            && *cached_area == area
        {
            return cached_cells.clone();
        }
        let cells = compute_perimeter_cells(area);
        *slot = Some((area, cells.clone()));
        cells
    })
}

#[allow(dead_code)]
pub fn compute_perimeter_cells(area: Rect) -> Vec<(u16, u16)> {
    let mut cells: Vec<(u16, u16)> = Vec::new();
    if area.width < 2 || area.height < 2 {
        return cells;
    }
    let right = area.x + area.width - 1;
    let bottom = area.y + area.height - 1;
    for x in area.x..=right {
        cells.push((x, area.y));
    }
    for y in (area.y + 1)..=bottom {
        cells.push((right, y));
    }
    if right > area.x {
        for x in (area.x..right).rev() {
            cells.push((x, bottom));
        }
    }
    if bottom > area.y + 1 {
        for y in ((area.y + 1)..bottom).rev() {
            cells.push((area.x, y));
        }
    }
    cells
}

/// Configuration for `paint_border_comets`. All knobs that callers
/// might want to vary at runtime live here so the painter stays
/// declarative — pass a struct, get a render.
#[allow(dead_code)]
pub struct CometConfig {
    /// Number of comets evenly spaced around the perimeter. 1..=4.
    pub(crate) count: u32,
    /// Lap duration in ms — full perimeter traversal time. Lower
    /// = faster comets. Drives by streaming velocity in the input
    /// renderer; can be hard-overridden via env.
    pub(crate) lap_ms: u128,
    /// Trail length in cells. 6 is the standard comet shape.
    pub(crate) trail_len: usize,
    /// Resting border color (the comet fades to this at the tail end).
    pub(crate) base: Color,
    /// Comet head color (the lead cell blends fully to this).
    pub(crate) head: Color,
    /// When true, comets at odd indices counter-rotate (go
    /// counter-clockwise) so a count=2 setup produces two comets
    /// going opposite directions, meeting at corners.
    pub(crate) counter_rotate: bool,
    /// Reverse the clockwise base direction. Combined with
    /// `counter_rotate`, this lets the tool-use signal flip every
    /// comet's direction at once.
    pub(crate) reverse_base: bool,
}

/// Paint N border comets traveling around the rectangle's perimeter
/// at a steady speed. Each comet is a `trail_len`-cell trail (head
/// at brightest blend toward `head` color, tail fading to `base`).
#[allow(dead_code)]
pub fn paint_border_comets(f: &mut Frame, area: Rect, cfg: &CometConfig) {
    // O(1) early exit: skip perimeter computation and buffer writes when
    // there is no animation to show (no comets configured, or zero-length
    // trail). Callers pass count=0 when neither streaming nor compaction
    // is active, so this is the common idle-frame path.
    if cfg.count == 0 || cfg.trail_len == 0 {
        return;
    }
    let perim = perimeter_cells(area);
    if perim.is_empty() {
        return;
    }
    let total = perim.len();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let head_pos_signed = ((now_ms * total as u128) / cfg.lap_ms.max(1)) as i64;

    let buf = f.buffer_mut();

    for c in 0..cfg.count {
        // Direction: even-indexed comets follow the base direction;
        // odd-indexed comets reverse if `counter_rotate` is set.
        // `reverse_base` flips the base on top of that.
        let counter = cfg.counter_rotate && c % 2 == 1;
        let direction_positive = match (cfg.reverse_base, counter) {
            (false, false) => true,
            (true, false) => false,
            (false, true) => false,
            (true, true) => true,
        };
        // Even spacing around the perimeter.
        let offset = (c as usize * total) / cfg.count.max(1) as usize;
        // Position of this comet's head this frame.
        let head_idx = if direction_positive {
            ((head_pos_signed + offset as i64).rem_euclid(total as i64)) as usize
        } else {
            ((-head_pos_signed + offset as i64).rem_euclid(total as i64)) as usize
        };
        for trail in 0..cfg.trail_len {
            // Trail cells trail "behind" the head along its
            // direction of travel.
            let pos = if direction_positive {
                (head_idx + total - trail) % total
            } else {
                (head_idx + trail) % total
            };
            let (x, y) = perim[pos];
            if x >= buf.area().right() || y >= buf.area().bottom() {
                continue;
            }
            // Squared falloff: head bright, tail dies off quickly.
            let pct = trail as f32 / cfg.trail_len as f32;
            let intensity = (1.0 - pct).powi(2);
            let blended = pulse_color(cfg.base, cfg.head, intensity);
            let cell = &mut buf[(x, y)];
            let mut style = cell.style();
            style.fg = Some(blended);
            cell.set_style(style);
        }
    }
}

/// Compute the comet config from the current app state. Centralizes
/// all the "what color, what speed, which direction" logic in one
/// place so the input renderer just calls this once.
#[allow(dead_code)]
pub fn comet_config_from_state(app: &App, t: Theme, count: u32) -> CometConfig {
    // Bash-mode detection: the user is composing a shell command
    // (input starts with `!`). Mirrors v126's bash-mode prompt
    // indicator. Color goes warning so the comets clearly signal
    // "this isn't a normal prompt".
    let bash_mode = app
        .textarea
        .lines()
        .iter()
        .next()
        .map(|line| line.trim_start().starts_with('!'))
        .unwrap_or(false);

    // Tool-use detection: any tool currently `Running` in the most
    // recent assistant turn (the streaming placeholder OR the last
    // committed message). Drives the reverse-direction +
    // warning-color override so the user sees "the model is
    // executing something" at a glance.
    let any_tool_running = app.messages.iter().rev().take(2).any(|m| {
        m.parts.iter().any(|p| {
            if let MessagePart::Tool(tc) = p {
                matches!(tc.status, ToolStatus::Running | ToolStatus::Pending)
            } else {
                false
            }
        })
    }) || !app.pending_tool_calls.is_empty();

    let head_color = if bash_mode {
        // Bash mode trumps tool-use coloring — it's the highest-
        // signal state because it's the user's explicit choice.
        t.warning
    } else if any_tool_running {
        t.warning
    } else {
        t.accent
    };

    // Speed = streaming velocity. Compute a rough tokens/sec rate
    // from the cumulative output and the turn elapsed time. Map to
    // a lap_ms with a few buckets so the speed change is
    // perceptible (smooth interpolation reads as "did it just
    // change?"). Resting (idle) sits at 3500ms.
    let now = std::time::Instant::now();
    let elapsed = app
        .turn_started_at
        .or(app.streaming_started_at)
        .map(|t0| now.duration_since(t0))
        .unwrap_or_default();
    let secs = elapsed.as_secs_f64().max(0.5);
    let live = app
        .last_usage_output
        .max((app.streaming_response_bytes / 4) as u32);
    let rate = (live as f64) / secs;
    let mut lap_ms: u128 = if !app.is_streaming {
        3500
    } else if rate > 60.0 {
        1200 // hot: fast laps
    } else if rate > 30.0 {
        2000 // warm
    } else {
        3500 // cold / first chunks
    };
    // Hard env override wins regardless.
    if let Some(forced) = std::env::var("JFC_BORDER_COMET_SPEED")
        .ok()
        .and_then(|s| s.parse::<u128>().ok())
    {
        lap_ms = forced.max(200);
    }

    let trail_len: usize = std::env::var("JFC_BORDER_COMET_TRAIL")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(6)
        .clamp(2, 12);

    // Counter-rotation is opt-in (matches v126's "two flames
    // chasing" pattern). Off by default — single-direction reads
    // calmer for an idle prompt.
    let counter_rotate = matches!(
        std::env::var("JFC_BORDER_COMET_COUNTER").as_deref(),
        Ok("1") | Ok("true")
    );

    CometConfig {
        count,
        lap_ms,
        trail_len,
        base: t.border,
        head: head_color,
        counter_rotate,
        // Tool-use reverses the base direction so the comets visibly
        // change which way they're going — strong signal that the
        // model is doing something the user can't see (running a
        // tool offscreen or in a long bash).
        reverse_base: any_tool_running && !bash_mode,
    }
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
            if (ms / 800) % 2 == 0 { "⌛" } else { "⌚" }
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

/// Public form for cross-module callers (sparkle in message_view, etc.)
/// — the private `pulse_color` is preferred inside this file for
/// brevity.
pub fn pulse_color_pub(c1: Color, c2: Color, t: f32) -> Color {
    pulse_color(c1, c2, t)
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
/// sidebar "Changes" section. Walks every Tool message part, picks up
/// `ToolOutput::Diff(_)` payloads (Edit/Write tools convert their result
/// into a unified diff at parse time — see `types.rs::ToolOutput::Diff`),
/// and de-duplicates files by their last-seen entry so the most recent
/// edit wins. Files appear in *most-recent-first* order to match how the
/// chat scrolls.
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
        if let Some((cached_msgs, cached_parts, ref stats)) = *cache {
            if cached_msgs == msg_count && cached_parts == total_parts {
                return stats.clone();
            }
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
            if let MessagePart::Tool(call) = part {
                if let ToolOutput::Diff(view) = &call.output {
                    let entry = by_file.entry(view.file_path.clone()).or_insert((0, 0));
                    *entry = (view.additions, view.deletions);
                    if !order.contains(&view.file_path) {
                        order.push(view.file_path.clone());
                    }
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

/// Like `truncate_str` but clips from the *front*, prepending `…/`
/// so the meaningful tail (project name in a path, identifier in a
/// long namespace) survives. Used by the sidebar's cwd display so
/// the user sees `…/active/jfc` on a narrow column rather than the
/// useless `~/RustProjec…` head.
#[allow(dead_code)]
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
