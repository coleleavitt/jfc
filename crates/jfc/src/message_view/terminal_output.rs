use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthChar;

use crate::theme::Theme;

/// Wrap a styled terminal line to `width` display cells while preserving span
/// styles. This is for preformatted command output, not markdown paragraphs.
pub(super) fn wrap_styled_line(line: &Line<'static>, width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return vec![line.clone()];
    }

    let total_width: usize = line
        .spans
        .iter()
        .flat_map(|s| s.content.chars())
        .map(char_width)
        .sum();
    if total_width <= width {
        return vec![line.clone()];
    }

    let mut out: Vec<Line<'static>> = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut current_w = 0usize;

    for span in &line.spans {
        let mut buf = String::new();
        for ch in span.content.chars() {
            let ch_w = char_width(ch);
            if current_w > 0 && current_w + ch_w > width {
                if !buf.is_empty() {
                    current.push(Span::styled(std::mem::take(&mut buf), span.style));
                }
                out.push(Line::from(std::mem::take(&mut current)).style(line.style));
                current_w = 0;
            }
            buf.push(ch);
            current_w += ch_w;
        }
        if !buf.is_empty() {
            current.push(Span::styled(buf, span.style));
        }
    }

    if !current.is_empty() {
        out.push(Line::from(current).style(line.style));
    }
    if out.is_empty() {
        out.push(line.clone());
    }
    out
}

/// Count rows `wrap_styled_line` would produce without allocating wrapped
/// `Line`s. Keep this logic in lockstep with `wrap_styled_line`: a zero-width
/// line still occupies one row, and a wide character at column zero is allowed
/// to overflow that row instead of creating an empty row first.
pub(super) fn styled_line_row_count(line: &Line<'static>, width: usize) -> usize {
    if width == 0 {
        return 1;
    }

    let mut rows = 1usize;
    let mut current_w = 0usize;
    let mut saw_char = false;

    for ch in line.spans.iter().flat_map(|s| s.content.chars()) {
        saw_char = true;
        let ch_w = char_width(ch);
        if current_w > 0 && current_w + ch_w > width {
            rows += 1;
            current_w = 0;
        }
        current_w += ch_w;
    }

    if saw_char { rows } else { 1 }
}

pub(super) fn wrapped_text_row_count(text: &str, width: usize) -> usize {
    styled_line_row_count(&Line::from(Span::raw(text.to_owned())), width)
}

pub(super) fn truncate_lines_middle_row_count(line_count: usize, max_lines: usize) -> usize {
    if max_lines == 0 || line_count <= max_lines {
        line_count
    } else {
        max_lines
    }
}

pub(super) fn expand_hint_text(count: usize, unit: &str) -> String {
    let plural = if count == 1 {
        unit.to_owned()
    } else {
        format!("{unit}s")
    };
    format!("… +{count} {plural} (ctrl+o to expand)")
}

pub(super) fn expand_hint_line(count: usize, unit: &str, style: Style) -> Line<'static> {
    Line::from(Span::styled(expand_hint_text(count, unit), style))
}

/// Keep the head and tail of already-wrapped terminal output, inserting a
/// visible omission marker in the middle. This preserves the final error lines
/// that are usually more useful than the middle of a long log.
pub(super) fn truncate_lines_middle(
    lines: Vec<Line<'static>>,
    max_lines: usize,
    marker_style: Style,
) -> Vec<Line<'static>> {
    if max_lines == 0 || lines.len() <= max_lines {
        return lines;
    }
    if max_lines == 1 {
        return vec![omitted_line(lines.len(), marker_style)];
    }

    let marker_rows = 1;
    let keep = max_lines.saturating_sub(marker_rows);
    let head = keep.div_ceil(2);
    let tail = keep.saturating_sub(head);
    let omitted = lines.len().saturating_sub(head + tail);

    let mut out = Vec::with_capacity(max_lines);
    out.extend(lines.iter().take(head).cloned());
    out.push(omitted_line(omitted, marker_style));
    if tail > 0 {
        out.extend(lines.iter().skip(lines.len() - tail).cloned());
    }
    out
}

fn omitted_line(omitted: usize, style: Style) -> Line<'static> {
    expand_hint_line(omitted, "line", style)
}

fn char_width(ch: char) -> usize {
    UnicodeWidthChar::width(ch).unwrap_or(0)
}

/// Detect git-diffstat-style lines (` path | NN +++---`) and return styled
/// spans for the path/count/graph. Returns `None` when the line does not match
/// the diffstat shape.
pub(super) fn colorize_diffstat_line(
    line: &str,
    fallback: Color,
    t: Theme,
) -> Option<Vec<Span<'static>>> {
    let sep_idx = line.find(" | ")?;
    let (prefix, rest) = line.split_at(sep_idx);
    let rest = &rest[3..];

    let bars_start = rest
        .char_indices()
        .find(|(_, c)| *c == '+' || *c == '-')
        .map(|(i, _)| i);
    let (head, bars) = match bars_start {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, ""),
    };

    if !head
        .chars()
        .all(|c| c.is_ascii_digit() || c.is_whitespace())
    {
        return None;
    }
    if !bars.is_empty() && !bars.chars().all(|c| c == '+' || c == '-') {
        return None;
    }
    if bars.is_empty() && !head.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }

    let mut spans: Vec<Span<'static>> = vec![
        Span::styled(
            prefix.to_owned(),
            Style::default().fg(path_color(prefix, t)),
        ),
        Span::styled(" | ", Style::default().fg(t.text_muted)),
        Span::styled(head.to_owned(), Style::default().fg(fallback)),
    ];

    let mut buf = String::new();
    let mut current_kind: Option<char> = None;
    for c in bars.chars() {
        match current_kind {
            Some(k) if k == c => buf.push(c),
            Some(k) => {
                push_diffstat_bar(&mut spans, &mut buf, k, t);
                buf.push(c);
                current_kind = Some(c);
            }
            None => {
                buf.push(c);
                current_kind = Some(c);
            }
        }
    }
    if let Some(k) = current_kind {
        push_diffstat_bar(&mut spans, &mut buf, k, t);
    }
    Some(spans)
}

fn push_diffstat_bar(spans: &mut Vec<Span<'static>>, buf: &mut String, kind: char, t: Theme) {
    let ui_tokens = t.claude_ui_tokens();
    let color = if kind == '+' {
        ui_tokens.diff_added
    } else {
        ui_tokens.diff_removed
    };
    let modifier = if kind == '+' {
        Modifier::BOLD
    } else {
        Modifier::BOLD | Modifier::DIM
    };
    spans.push(Span::styled(
        std::mem::take(buf),
        Style::default().fg(color).add_modifier(modifier),
    ));
}

fn path_color(path: &str, t: Theme) -> Color {
    let ext = std::path::Path::new(path.trim())
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "rs" | "go" | "py" | "js" | "ts" | "tsx" | "jsx" | "rb" | "java" | "c" | "cpp" | "h"
        | "hpp" | "swift" | "kt" | "lua" | "zig" | "ml" | "hs" | "ex" | "exs" => t.accent,
        "toml" | "yaml" | "yml" | "json" | "ini" | "cfg" | "conf" | "env" | "lock" => {
            t.text_secondary
        }
        "md" | "mdx" | "rst" | "txt" | "adoc" => t.text_primary,
        "html" | "css" | "scss" | "sass" | "less" | "vue" | "svelte" => t.success,
        "sh" | "bash" | "zsh" | "fish" => t.warning,
        _ => t.text_muted,
    }
}
