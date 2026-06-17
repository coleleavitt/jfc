use super::assistant_parts::sanitize_terminal_text;
use super::truncation::{push_wrapped_diff_data_line, push_wrapped_styled_line};
use super::*;

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
#[cfg(test)]
pub fn diff_lang(diff: &DiffView) -> Option<String> {
    let p = std::path::Path::new(&diff.file_path);
    if let Some(ext) = p.extension().and_then(|e| e.to_str())
        && !ext.is_empty()
    {
        return Some(ext.to_string());
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
    let ui_tokens = t.claude_ui_tokens();

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

        for dl in hunk.lines.iter().take(max_dl) {
            let (bg_color, fg_color, sigil) = match dl.kind {
                DiffLineKind::Added => (t.code_bg, ui_tokens.diff_added, "+"),
                DiffLineKind::Removed => (t.code_bg, ui_tokens.diff_removed, "-"),
                DiffLineKind::Context => (t.bg, t.text_secondary, " "),
            };
            // Line-number column matches v126's diff style — show
            // the `new_line` for added/context (the post-edit location)
            // and `old_line` for removed (the source location).
            let lineno = match dl.kind {
                DiffLineKind::Removed => dl.old_line,
                _ => dl.new_line,
            };

            let mut style = Style::default().fg(fg_color).bg(bg_color);
            if matches!(dl.kind, DiffLineKind::Removed) {
                style = style.add_modifier(Modifier::DIM);
            }
            let content_spans = vec![Span::styled(sanitize_terminal_text(&dl.content), style)];

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

pub(super) fn diff_view_line_count(diff: &DiffView, expanded: bool, width: usize) -> usize {
    let mut rows = 0usize;

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
        rows += terminal_output::wrapped_text_row_count(&summary, width);
    }

    for hunk in &diff.hunks {
        rows +=
            terminal_output::wrapped_text_row_count(&sanitize_terminal_text(&hunk.header), width);

        let hunk_cap = if expanded { 500 } else { 50 };
        let max_dl = hunk.lines.len().min(hunk_cap);
        let content_w = width.saturating_sub(8).max(1);
        for dl in hunk.lines.iter().take(max_dl) {
            rows += terminal_output::wrapped_text_row_count(
                &sanitize_terminal_text(&dl.content),
                content_w,
            );
        }

        if hunk.lines.len() > hunk_cap {
            rows += terminal_output::wrapped_text_row_count(
                &format!("… {} more lines", hunk.lines.len() - hunk_cap),
                width,
            );
        }
    }

    rows
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
