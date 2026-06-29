use super::assistant_parts::sanitize_terminal_text;
use super::syntax::lang_from_path;
use super::terminal_output;
use super::truncation::{push_wrapped_diff_data_line, push_wrapped_styled_line};
use super::*;

pub(super) fn file_mutation_success_body_is_redundant(tool: &ToolCall) -> bool {
    matches!(tool.status, ToolStatus::Completed)
        && is_file_mutation_tool(tool)
        && matches!(tool.output, ToolOutput::Text(_) | ToolOutput::LargeText(_))
}

pub(super) fn is_file_mutation_tool(tool: &ToolCall) -> bool {
    matches!(
        tool.kind,
        ToolKind::Write
            | ToolKind::Edit
            | ToolKind::MultiEdit
            | ToolKind::NotebookEdit
            | ToolKind::ApplyPatch
    ) || matches!(
        tool.input,
        ToolInput::Write { .. }
            | ToolInput::Edit { .. }
            | ToolInput::MultiEdit { .. }
            | ToolInput::NotebookEdit { .. }
            | ToolInput::ApplyPatch { .. }
    )
}

pub(super) fn diff_lang(diff: &DiffView) -> Option<String> {
    lang_from_path(&diff.file_path)
}

pub(super) fn produce_diff_view_lines(
    diff: &DiffView,
    t: Theme,
    expanded: bool,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if let Some(summary) = diff_summary(diff) {
        push_wrapped_styled_line(
            &mut lines,
            vec![Span::styled(summary, Style::default().fg(t.text_muted))],
            width,
            t.bg,
        );
    }

    let lang = diff_lang(diff);
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
        let syntax_lines = hunk_syntax_lines(hunk, lang.as_deref(), t);
        for (idx, dl) in hunk
            .lines
            .iter()
            .take(hunk.lines.len().min(hunk_cap))
            .enumerate()
        {
            let syntax_spans = syntax_lines
                .as_ref()
                .and_then(|hunk_lines| hunk_lines.get(idx))
                .map(Vec::as_slice);
            push_diff_line(&mut lines, dl, syntax_spans, lang.as_deref(), width, t);
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
    if let Some(summary) = diff_summary(diff) {
        rows += terminal_output::wrapped_text_row_count(&summary, width);
    }

    let content_w = width.saturating_sub(8).max(1);
    for hunk in &diff.hunks {
        rows +=
            terminal_output::wrapped_text_row_count(&sanitize_terminal_text(&hunk.header), width);
        let hunk_cap = if expanded { 500 } else { 50 };
        for dl in hunk.lines.iter().take(hunk.lines.len().min(hunk_cap)) {
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

fn diff_summary(diff: &DiffView) -> Option<String> {
    if diff.additions == 0 && diff.deletions == 0 {
        return None;
    }
    let mut parts = Vec::new();
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
    Some(format!("□ {}", parts.join(", ")))
}

fn push_diff_line(
    lines: &mut Vec<Line<'static>>,
    dl: &DiffLine,
    syntax_spans: Option<&[Span<'static>]>,
    lang: Option<&str>,
    width: usize,
    t: Theme,
) {
    let ui_tokens = t.claude_ui_tokens();
    let (bg_color, fg_color, sigil) = match dl.kind {
        DiffLineKind::Added => (ui_tokens.diff_added_background, ui_tokens.diff_added, "+"),
        DiffLineKind::Removed => (
            ui_tokens.diff_removed_background,
            ui_tokens.diff_removed,
            "-",
        ),
        DiffLineKind::Context => (t.bg, t.text_secondary, " "),
    };
    let lineno = match dl.kind {
        DiffLineKind::Removed => dl.old_line,
        _ => dl.new_line,
    };
    let dim = matches!(dl.kind, DiffLineKind::Removed);
    let content_spans =
        diff_content_spans(lang, &dl.content, syntax_spans, t, bg_color, fg_color, dim);
    push_wrapped_diff_data_line(
        lines,
        lineno,
        sigil,
        fg_color,
        bg_color,
        t.text_muted,
        content_spans,
        width,
    );
}

fn hunk_syntax_lines(
    hunk: &DiffHunk,
    lang: Option<&str>,
    t: Theme,
) -> Option<Vec<Vec<Span<'static>>>> {
    let lang = lang.filter(|lang| !lang.is_empty())?;
    let mut source = String::new();
    for (idx, dl) in hunk.lines.iter().enumerate() {
        if idx > 0 {
            source.push('\n');
        }
        source.push_str(&sanitize_terminal_text(&dl.content));
    }

    let highlighted = markdown::highlight_code_raw(lang, &source, 0, &t);
    (highlighted.len() == hunk.lines.len()).then(|| {
        highlighted
            .into_iter()
            .map(|line| line.spans.into_iter().collect())
            .collect()
    })
}

fn diff_content_spans(
    lang: Option<&str>,
    content: &str,
    syntax_spans: Option<&[Span<'static>]>,
    t: Theme,
    bg_color: Color,
    fallback_fg: Color,
    dim: bool,
) -> Vec<Span<'static>> {
    let clean = sanitize_terminal_text(content);
    if let Some(syntax_spans) = syntax_spans {
        let spans = decorate_syntax_spans(syntax_spans, bg_color, fallback_fg, dim);
        if !spans.is_empty() {
            return spans;
        }
    }

    let Some(lang) = lang.filter(|lang| !lang.is_empty()) else {
        return vec![Span::styled(
            clean,
            diff_content_style(Style::default().fg(fallback_fg), bg_color, fallback_fg, dim),
        )];
    };

    let highlighted = markdown::highlight_code_raw(lang, &clean, 0, &t);
    let mut spans: Vec<Span<'static>> = highlighted
        .into_iter()
        .flat_map(|line| line.spans)
        .filter(|span| !span.content.is_empty())
        .map(|span| {
            let style = diff_content_style(span.style, bg_color, fallback_fg, dim);
            Span::styled(span.content.into_owned(), style)
        })
        .collect();
    if spans.is_empty() {
        spans.push(Span::styled(
            clean,
            diff_content_style(Style::default().fg(fallback_fg), bg_color, fallback_fg, dim),
        ));
    }
    spans
}

fn decorate_syntax_spans(
    syntax_spans: &[Span<'static>],
    bg_color: Color,
    fallback_fg: Color,
    dim: bool,
) -> Vec<Span<'static>> {
    syntax_spans
        .iter()
        .filter(|span| !span.content.is_empty())
        .map(|span| {
            let style = diff_content_style(span.style, bg_color, fallback_fg, dim);
            Span::styled(span.content.as_ref().to_owned(), style)
        })
        .collect()
}

fn diff_content_style(mut style: Style, bg_color: Color, fallback_fg: Color, dim: bool) -> Style {
    style.bg = Some(bg_color);
    if style.fg.is_none() {
        style.fg = Some(fallback_fg);
    }
    if dim {
        style.add_modifier(Modifier::DIM)
    } else {
        style
    }
}
