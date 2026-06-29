use super::assistant_parts::{sanitize_terminal_text, truncate_str};
use super::bash::{BashCmdKind, classify_bash_cmd};
use super::core::{diagnostics_for_input, diagnostics_for_path};
use super::detection::detect_background_task_notification;
use super::detection::looks_like_git_diff_output;
use super::file_tool::{
    diff_view_line_count, file_mutation_success_body_is_redundant, is_file_mutation_tool,
    render_diff_skip,
};
use super::formatters::{
    produce_cat_markdown_output_line_count, produce_cat_markdown_output_lines,
    produce_cat_output_line_count, produce_cat_output_lines, produce_command_output_line_count,
    produce_command_output_lines, produce_compiler_output_line_count,
    produce_compiler_output_lines, produce_file_list_line_count, produce_file_list_lines,
    produce_git_diff_output_line_count, produce_git_diff_output_lines,
    produce_git_log_output_line_count, produce_git_log_output_lines,
    produce_grep_output_line_count, produce_grep_output_lines, produce_hex_dump_output_line_count,
    produce_hex_dump_output_lines, produce_path_list_output_line_count,
    produce_path_list_output_lines, produce_tabular_list_output_line_count,
    produce_tabular_list_output_lines,
};
use super::output_style::{
    colorize_diagnostic_prefix, colorize_git_commit_line, colorize_git_diff_line,
    colorize_git_log_line, colorize_git_push_line, colorize_git_status_line,
};
use super::syntax::{
    infer_lang_from_bash, infer_lang_from_tool, looks_like_markdown,
    produce_highlighted_block_line_count, produce_highlighted_block_lines,
    produce_highlighted_with_line_numbers_line_count, produce_highlighted_with_line_numbers_lines,
};
use super::tool_height::tool_block_height_with_app;
use super::*;

/// Count the rows produced by `tool_body_lines_themed` without constructing
/// the styled line Vecs used for painting.
#[allow(dead_code)]
pub(super) fn tool_body_line_count(tool: &ToolCall, content_w: usize) -> usize {
    tool_body_line_count_with_app(tool, content_w, None)
}

pub(super) fn tool_body_line_count_with_app(
    tool: &ToolCall,
    content_w: usize,
    app: Option<&App>,
) -> usize {
    let diagnostics = app
        .map(|app| app.engine.diagnostics.as_slice())
        .unwrap_or(&[]);
    tool_body_line_count_with_diagnostics(tool, content_w, diagnostics)
}

pub(super) fn tool_body_line_count_with_diagnostics(
    tool: &ToolCall,
    content_w: usize,
    diagnostics: &[jfc_engine::diagnostics::DiagnosticEntry],
) -> usize {
    let t = crate::theme::Theme::dark();
    let expanded = tool.display.is_expanded();
    // AskModel has a bespoke gutter renderer; count its lines directly so the
    // height index matches what tool_body_lines_themed produces.
    if let ToolInput::AskModel { model, prompt, .. } = &tool.input {
        return ask_model_exchange_lines(model, prompt, &tool.output, content_w, t).len();
    }
    if file_mutation_success_body_is_redundant(tool) {
        return 0;
    }
    match &tool.output {
        ToolOutput::Empty => 0,
        ToolOutput::Text(s) => {
            let s = visible_tool_text_without_guard(s);
            if s.is_empty() {
                return 0;
            }
            if let Some(display) = bash_output_display_text(tool, s) {
                return produce_text_block_line_count(
                    display.as_ref(),
                    content_w,
                    t.text_secondary,
                    expanded,
                );
            }
            // Compact render for background task notifications
            if detect_background_task_notification(s).is_some() {
                return 1;
            }
            if let Some(lang) = infer_lang_from_tool(tool) {
                let diag_lines = if matches!(tool.kind, ToolKind::Read) {
                    diagnostics_for_input(diagnostics, &tool.input)
                } else {
                    std::collections::HashMap::new()
                };
                produce_highlighted_with_line_numbers_line_count(
                    &lang,
                    s,
                    content_w,
                    t,
                    expanded,
                    &diag_lines,
                )
            } else if looks_like_git_diff_output(s) {
                produce_git_diff_output_line_count(s, "", Some(0), content_w, expanded)
            } else if matches!(tool.kind, ToolKind::Task) {
                produce_markdown_block_line_count(s, content_w, t)
            } else {
                produce_text_block_line_count(s, content_w, t.text_secondary, expanded)
            }
        }
        ToolOutput::LargeText(lt) => {
            if matches!(tool.status, ToolStatus::Completed) && is_file_mutation_tool(tool) {
                return 0;
            }
            let huge = lt.line_count > LargeText::COLLAPSE_LINES
                || lt.content.len() > LargeText::COLLAPSE_BYTES;
            if huge && !expanded {
                1
            } else if looks_like_git_diff_output(&lt.content) {
                produce_git_diff_output_line_count(&lt.content, "", Some(0), content_w, expanded)
            } else {
                produce_text_block_line_count(&lt.content, content_w, t.text_secondary, expanded)
            }
        }
        ToolOutput::Command {
            stdout,
            stderr,
            exit_code,
        } => {
            // Compact render for background task notifications
            if detect_background_task_notification(stdout).is_some() {
                return 1;
            }
            let cmd_str = match &tool.input {
                ToolInput::Bash { command, .. } => command.as_str(),
                _ => "",
            };
            let success = !stdout.is_empty() && exit_code.unwrap_or(-1) == 0;
            let cmd_kind = classify_bash_cmd(cmd_str);
            let grep_success = matches!(cmd_kind, BashCmdKind::Grep)
                && !stdout.is_empty()
                && exit_code.unwrap_or(-1) <= 1;
            let gitdiff_success = matches!(cmd_kind, BashCmdKind::GitDiff)
                && !stdout.is_empty()
                && exit_code.unwrap_or(-1) <= 1;
            match cmd_kind {
                BashCmdKind::Grep if grep_success => {
                    produce_grep_output_line_count(stdout, stderr, *exit_code, expanded, cmd_str)
                }
                BashCmdKind::PathList if success => {
                    produce_path_list_output_line_count(stdout, stderr, *exit_code, expanded)
                }
                BashCmdKind::GitDiff if gitdiff_success => produce_git_diff_output_line_count(
                    stdout, stderr, *exit_code, content_w, expanded,
                ),
                BashCmdKind::GitLog if success => {
                    produce_git_log_output_line_count(stdout, stderr, *exit_code, expanded)
                }
                BashCmdKind::HexDump if success => {
                    produce_hex_dump_output_line_count(stdout, stderr, *exit_code, expanded)
                }
                BashCmdKind::TabularList if success => {
                    produce_tabular_list_output_line_count(stdout, stderr, *exit_code, expanded)
                }
                BashCmdKind::CompilerOutput
                    if !stdout.is_empty()
                        && exit_code
                            .map(|c| c == 0 || c == 101 || c == 1)
                            .unwrap_or(true) =>
                {
                    produce_compiler_output_line_count(stdout, stderr, *exit_code, expanded)
                }
                _ => {
                    if looks_like_git_diff_output(stdout) {
                        return produce_git_diff_output_line_count(
                            stdout, stderr, *exit_code, content_w, expanded,
                        );
                    }
                    let lang_hint = infer_lang_from_tool(tool);
                    let lang_lc = lang_hint.as_deref().map(|l| l.to_ascii_lowercase());
                    let is_markdown_lang = lang_lc
                        .as_deref()
                        .map(|l| matches!(l, "md" | "markdown" | "mdx" | "mkd" | "mdown"))
                        .unwrap_or(false);
                    let content_is_md = !is_markdown_lang && looks_like_markdown(stdout);
                    if success && (is_markdown_lang || content_is_md) {
                        produce_cat_markdown_output_line_count(
                            stdout, stderr, *exit_code, content_w, t,
                        )
                    } else if let Some(lang) = lang_hint.as_deref().filter(|_| success) {
                        produce_cat_output_line_count(
                            lang, stdout, stderr, *exit_code, content_w, t, expanded,
                        )
                    } else {
                        produce_command_output_line_count(
                            stdout, stderr, *exit_code, content_w, t, expanded,
                        )
                    }
                }
            }
        }
        ToolOutput::FileContent {
            content, language, ..
        } => {
            if looks_like_git_diff_output(content) {
                produce_git_diff_output_line_count(content, "", Some(0), content_w, expanded)
            } else {
                let hl_lang = if language.is_empty() {
                    "rs"
                } else {
                    language.as_str()
                };
                produce_highlighted_block_line_count(hl_lang, content, content_w, t, expanded)
            }
        }
        ToolOutput::Diff(diff) => diff_view_line_count(diff, expanded, content_w),
        ToolOutput::FileList(files) => produce_file_list_line_count(files),
        ToolOutput::ServerToolResult { tool_kind, content } => {
            let rendered = jfc_core::format_server_tool_result_text_public(tool_kind, content);
            produce_text_block_line_count(&rendered, content_w, t.text_secondary, expanded)
        }
    }
}

/// Theme-aware variant. The renderer calls this with the actual theme so the
/// styled spans match what gets painted. The height path mirrors this dispatch
/// in `tool_body_line_count`; `app` is needed only for the Read-tool
/// diagnostics gutter.
pub(super) fn tool_body_lines_themed(
    tool: &ToolCall,
    content_w: usize,
    t: Theme,
    app: Option<&App>,
) -> Vec<Line<'static>> {
    // AskModel renders as an attributed cross-model exchange: the asked
    // model's reply sits behind a provider-colored side gutter (blue=Claude,
    // green=GPT, …) with a label + glyph header, so the stream reads as a
    // conversation between models rather than a flat tool result.
    if let ToolInput::AskModel { model, prompt, .. } = &tool.input {
        return ask_model_exchange_lines(model, prompt, &tool.output, content_w, t);
    }
    if file_mutation_success_body_is_redundant(tool) {
        return Vec::new();
    }

    // Body content branch by tool.output (mirrors render_tool_content_with_skip).
    match &tool.output {
        ToolOutput::Empty => Vec::new(),
        ToolOutput::Text(s) => {
            let s = visible_tool_text_without_guard(s);
            if s.is_empty() {
                return Vec::new();
            }
            if let Some(display) = bash_output_display_text(tool, s) {
                return produce_text_block_lines(
                    display.as_ref(),
                    content_w,
                    t.text_secondary,
                    t,
                    tool.display.is_expanded(),
                );
            }
            // Compact render for background task notifications — show a
            // single muted line instead of the full infrastructure block.
            if let Some(task_id) = detect_background_task_notification(s) {
                return vec![Line::from(Span::styled(
                    format!("backgrounded -> {task_id}"),
                    Style::default()
                        .fg(t.text_muted)
                        .add_modifier(Modifier::ITALIC),
                ))];
            }
            let lang = infer_lang_from_tool(tool);
            if let Some(lang) = lang.as_deref() {
                let diag_lines = if matches!(tool.kind, ToolKind::Read) {
                    if let Some(app) = app {
                        diagnostics_for_path(app, &tool.input)
                    } else {
                        std::collections::HashMap::new()
                    }
                } else {
                    std::collections::HashMap::new()
                };
                produce_highlighted_with_line_numbers_lines(
                    lang,
                    s,
                    content_w,
                    t,
                    tool.display.is_expanded(),
                    &diag_lines,
                )
            } else if looks_like_git_diff_output(s) {
                produce_git_diff_output_lines(
                    s,
                    "",
                    Some(0),
                    content_w,
                    t,
                    tool.display.is_expanded(),
                )
            } else if matches!(tool.kind, ToolKind::Task) {
                produce_markdown_block_lines(s, content_w, t)
            } else {
                produce_text_block_lines(
                    s,
                    content_w,
                    t.text_secondary,
                    t,
                    tool.display.is_expanded(),
                )
            }
        }
        ToolOutput::LargeText(lt) => {
            if matches!(tool.status, ToolStatus::Completed) && is_file_mutation_tool(tool) {
                return Vec::new();
            }
            let huge = lt.line_count > LargeText::COLLAPSE_LINES
                || lt.content.len() > LargeText::COLLAPSE_BYTES;
            if huge && !tool.display.is_expanded() {
                vec![Line::from(Span::styled(
                    format!("[{} · ctrl+o to expand]", lt.size_label()),
                    Style::default()
                        .fg(t.text_muted)
                        .add_modifier(Modifier::ITALIC),
                ))]
            } else if looks_like_git_diff_output(&lt.content) {
                produce_git_diff_output_lines(
                    &lt.content,
                    "",
                    Some(0),
                    content_w,
                    t,
                    tool.display.is_expanded(),
                )
            } else {
                produce_text_block_lines(
                    &lt.content,
                    content_w,
                    t.text_secondary,
                    t,
                    tool.display.is_expanded(),
                )
            }
        }
        ToolOutput::Command {
            stdout,
            stderr,
            exit_code,
        } => {
            // Compact render for background task notifications
            if let Some(task_id) = detect_background_task_notification(stdout) {
                return vec![Line::from(Span::styled(
                    format!("backgrounded -> {task_id}"),
                    Style::default()
                        .fg(t.text_muted)
                        .add_modifier(Modifier::ITALIC),
                ))];
            }
            let cmd_str = match &tool.input {
                ToolInput::Bash { command, .. } => command.as_str(),
                _ => "",
            };
            let cmd_kind = classify_bash_cmd(cmd_str);
            let success = !stdout.is_empty() && exit_code.unwrap_or(-1) == 0;
            let grep_success = matches!(cmd_kind, BashCmdKind::Grep)
                && !stdout.is_empty()
                && exit_code.unwrap_or(-1) <= 1;
            let gitdiff_success = matches!(cmd_kind, BashCmdKind::GitDiff)
                && !stdout.is_empty()
                && exit_code.unwrap_or(-1) <= 1;
            match cmd_kind {
                BashCmdKind::Grep if grep_success => produce_grep_output_lines(
                    stdout,
                    stderr,
                    *exit_code,
                    t,
                    tool.display.is_expanded(),
                    cmd_str,
                ),
                BashCmdKind::PathList if success => produce_path_list_output_lines(
                    stdout,
                    stderr,
                    *exit_code,
                    t,
                    tool.display.is_expanded(),
                ),
                BashCmdKind::GitDiff if gitdiff_success => produce_git_diff_output_lines(
                    stdout,
                    stderr,
                    *exit_code,
                    content_w,
                    t,
                    tool.display.is_expanded(),
                ),
                BashCmdKind::GitLog if success => produce_git_log_output_lines(
                    stdout,
                    stderr,
                    *exit_code,
                    t,
                    tool.display.is_expanded(),
                ),
                BashCmdKind::HexDump if success => produce_hex_dump_output_lines(
                    stdout,
                    stderr,
                    *exit_code,
                    t,
                    tool.display.is_expanded(),
                ),
                BashCmdKind::TabularList if success => produce_tabular_list_output_lines(
                    stdout,
                    stderr,
                    *exit_code,
                    t,
                    tool.display.is_expanded(),
                ),
                BashCmdKind::CompilerOutput
                    if !stdout.is_empty()
                        && exit_code
                            .map(|c| c == 0 || c == 101 || c == 1)
                            .unwrap_or(true) =>
                {
                    produce_compiler_output_lines(
                        stdout,
                        stderr,
                        *exit_code,
                        t,
                        tool.display.is_expanded(),
                    )
                }
                _ => {
                    if looks_like_git_diff_output(stdout) {
                        return produce_git_diff_output_lines(
                            stdout,
                            stderr,
                            *exit_code,
                            content_w,
                            t,
                            tool.display.is_expanded(),
                        );
                    }
                    let lang_hint = infer_lang_from_tool(tool);
                    let lang_lc = lang_hint.as_deref().map(|l| l.to_ascii_lowercase());
                    let is_markdown_lang = lang_lc
                        .as_deref()
                        .map(|l| matches!(l, "md" | "markdown" | "mdx" | "mkd" | "mdown"))
                        .unwrap_or(false);
                    let content_is_md = !is_markdown_lang && looks_like_markdown(stdout);
                    if success && (is_markdown_lang || content_is_md) {
                        produce_cat_markdown_output_lines(stdout, stderr, *exit_code, content_w, t)
                    } else if let Some(lang) = lang_hint.as_deref().filter(|_| success) {
                        produce_cat_output_lines(
                            lang,
                            stdout,
                            stderr,
                            *exit_code,
                            content_w,
                            t,
                            tool.display.is_expanded(),
                        )
                    } else {
                        produce_command_output_lines(
                            stdout,
                            stderr,
                            *exit_code,
                            content_w,
                            t,
                            tool.display.is_expanded(),
                        )
                    }
                }
            }
        }
        ToolOutput::Diff(_) => {
            // Diff renders directly to buffer with per-row bg tinting that
            // doesn't fit `Paragraph`'s model. Returning empty here keeps the
            // line producer honest: height goes through `tool_body_line_count`,
            // not this Vec path.
            Vec::new()
        }
        ToolOutput::FileContent {
            content, language, ..
        } => {
            if looks_like_git_diff_output(content) {
                return produce_git_diff_output_lines(
                    content,
                    "",
                    Some(0),
                    content_w,
                    t,
                    tool.display.is_expanded(),
                );
            }
            let hl_lang = if language.is_empty() {
                "rs"
            } else {
                language.as_str()
            };
            produce_highlighted_block_lines(
                hl_lang,
                content,
                content_w,
                t,
                tool.display.is_expanded(),
            )
        }
        ToolOutput::FileList(files) => produce_file_list_lines(files, t),
        ToolOutput::ServerToolResult { tool_kind, content } => {
            // Render the parsed JSON via the same text helper used for
            // ToolOutput::Text — the resulting string is already
            // human-readable (titled bulleted list for web_search,
            // pretty-printed JSON for the others).
            let rendered = jfc_core::format_server_tool_result_text_public(tool_kind, content);
            produce_text_block_lines(
                &rendered,
                content_w,
                t.text_secondary,
                t,
                tool.display.is_expanded(),
            )
        }
    }
}

pub(super) fn render_tool_block(
    app: &App,
    tool: &ToolCall,
    area: Rect,
    t: Theme,
    buf: &mut Buffer,
    skip: usize,
) {
    if area.height == 0 {
        return;
    }

    if tool.display.is_collapsed() {
        if skip == 0 {
            // Collapsed-tool header: no gutter glyph (matching the
            // expanded path). The header itself includes the status
            // icon and kind-colored title which carry the same info.
            let header = build_collapsed_header(
                tool,
                &t,
                area.width as usize,
                app.launched_at.elapsed().as_millis(),
            );
            Paragraph::new(header)
                .style(Style::default().bg(t.bg))
                .render(
                    Rect {
                        x: area.x,
                        y: area.y,
                        width: area.width,
                        height: 1,
                    },
                    buf,
                );
        }
        return;
    }

    let frame_idx = app.spinner_frame;
    let (status_icon, status_style) = tool_status_icon_animated(tool, &t, frame_idx);

    let full_h = tool_block_height_with_app(tool, area.width as usize, Some(app)) as u16;
    if skip >= full_h as usize {
        return;
    }

    // No more full-height left gutter bar. The tool's identity is
    // already shown three different ways — title text (`Bash(...)`,
    // `Read(...)`), the status icon (`●`/`○`/`✓`/`✘`), and the
    // kind-colored title — so painting a fourth signal as a column
    // down the left edge was redundant decoration. Same problem
    // the sidebar gutters had. v126's actual tool rendering uses
    // just title-line + indent; mirroring that here.

    // (The 600ms `✦` completion-flash that pulsed here is removed — the
    // tool's status icon already shows completion; a fading sparkle was
    // celebratory decoration, not information.)

    let title_spans = build_title_spans(
        tool,
        &t,
        status_icon,
        status_style,
        area.width.saturating_sub(2) as usize,
    );

    // Title now sits at column 0 (no gutter to dodge). The status
    // icon at the start of `title_spans` is the visual anchor.
    if skip == 0 && area.height > 0 {
        let title_area = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        };
        Paragraph::new(Line::from(title_spans))
            .style(Style::default().bg(t.bg))
            .render(title_area, buf);
        register_bash_command_copy_region(app, tool, title_area);
    }

    let title_consumed: u16 = if skip == 0 { 1 } else { 0 };
    let content_skip = skip.saturating_sub(1);
    let content_y = area.y + title_consumed;
    let content_h = area.height.saturating_sub(title_consumed);
    if content_h == 0 {
        return;
    }
    // Body indents 2 columns from the title's left edge so it
    // visually nests under the tool's status icon. With the gutter
    // gone, the indent is a pure visual cue: title at column 0, body
    // starts at column 2. Mirrors how `gh pr view`, `git log`, and
    // most CLI tools nest output under their headers.
    let content_area = Rect {
        x: area.x + 2,
        y: content_y,
        width: area.width.saturating_sub(2),
        height: content_h,
    };
    if content_area.width > 0 {
        render_tool_content_with_skip(app, tool, content_area, t, buf, content_skip);
    }
}

fn register_bash_command_copy_region(app: &App, tool: &ToolCall, rect: Rect) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    if matches!(&tool.input, ToolInput::Bash { .. }) {
        app.tool_copy_regions
            .borrow_mut()
            .push((tool.id.to_string(), rect));
    }
}

pub(super) fn build_collapsed_header<'a>(
    tool: &'a ToolCall,
    t: &Theme,
    width: usize,
    elapsed_ms: u128,
) -> Line<'a> {
    let frame_idx = (elapsed_ms / crate::app::ANIM_TICK_MS as u128) as usize;
    let (status_icon, status_style) = tool_status_icon_animated(tool, t, frame_idx);
    // Collapsed-tool header: status icon + title. The chevron `▶`
    // that used to mark "expandable" was redundant — a collapsed
    // tool is already visibly missing its body. The status icon is
    // the only visual anchor that carries unique info, so it gets
    // the front spot.
    let mut spans = vec![
        Span::styled(status_icon.to_owned(), status_style),
        Span::raw(" "),
    ];
    spans.extend(build_header_inner_spans(tool, t, width.saturating_sub(4)));
    Line::from(spans)
}

/// Cap the visible tool title at a sensible length even on wide terminals.
/// A 200-column terminal showing a sprawling `bash uname -a && cat … |
/// head -5 && echo --- && lscpu | head -10 && echo --- && free -h`
/// across a full row reads as one giant ribbon of grey instead of as a
/// labeled invocation. v126 keeps tool titles brief; the full command is
/// visible in the expanded body. Tunable via `JFC_TOOL_TITLE_WIDTH` for
/// users who want the full command on a wide screen.
pub(super) fn tool_title_width_cap() -> usize {
    // Default: no artificial cap — use the full terminal width so a long
    // command (the first line of a `git commit -m "…"`, a wide `cargo`
    // invocation) shows as much as physically fits instead of being
    // chopped at 100 cols on a wide terminal, where the rest of line 1
    // would otherwise be invisible (it lives only in the title; the body
    // renders lines 2+). `JFC_TOOL_TITLE_WIDTH` can still pin a cap.
    std::env::var("JFC_TOOL_TITLE_WIDTH")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| *n >= 20)
        .unwrap_or(usize::MAX)
}

pub(super) fn build_title_spans<'a>(
    tool: &'a ToolCall,
    t: &Theme,
    status_icon: &'static str,
    status_style: Style,
    width: usize,
) -> Vec<Span<'a>> {
    // Expanded-tool title: status icon + title. The `▼` chevron that
    // used to mark "expanded" was redundant — the body's presence
    // underneath already shows it's expanded. Cleaner without it.
    let mut spans = vec![
        Span::styled(status_icon.to_owned(), status_style),
        Span::raw(" "),
    ];
    if tool.display.is_pinned() {
        spans.push(Span::styled(
            "◆ ",
            Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
        ));
    }
    // Reserve a few columns at the right for the optional elapsed
    // badge. `format_elapsed_badge` returns `Some("[2.3s]")` only for
    // completed/failed tools that have a measured duration, otherwise
    // None.
    let badge = format_elapsed_badge(tool);
    // Exit-code badge `(N)` for a failed command — codex parity. Lets the
    // user see *why* a bash step failed without expanding the block.
    let exit_badge = format_exit_code_badge(tool);
    let badge_w = badge
        .as_ref()
        .map(|s| crate::render::visual::cell_width(s) + 1)
        .unwrap_or(0)
        + exit_badge
            .as_ref()
            .map(|s| crate::render::visual::cell_width(s) + 1)
            .unwrap_or(0);
    let effective = width
        .min(tool_title_width_cap())
        .saturating_sub(4 + badge_w);
    spans.extend(build_header_inner_spans(tool, t, effective));
    if let Some(code) = exit_badge {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            code,
            Style::default().fg(t.error).add_modifier(Modifier::DIM),
        ));
    }
    if let Some(b) = badge {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            b,
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::DIM),
        ));
    }
    spans
}

fn visible_tool_text_without_guard(raw: &str) -> &str {
    raw.split_once(jfc_engine::tools::SLOP_GUARD_MARKER)
        .map_or(raw, |(visible, _)| visible.trim_end())
}

/// Exit-code badge for a failed command tool, e.g. `(1)`. Returns `None`
/// for non-failed tools, success exit codes, or tools whose output isn't a
/// command (so it never competes with the elapsed badge on a clean run).
pub(super) fn format_exit_code_badge(tool: &ToolCall) -> Option<String> {
    if !matches!(tool.status, ToolStatus::Failed) {
        return None;
    }
    match &tool.output {
        ToolOutput::Command {
            exit_code: Some(code),
            ..
        } if *code != 0 => Some(format!("({code})")),
        _ => None,
    }
}

/// Render the elapsed duration as a compact badge for the title row.
/// Only shown after a tool finishes — pending/running tools show
/// the spinner and don't need a badge yet. Skips sub-100ms results
/// (their badge is too noisy and adds nothing — most reads, glob,
/// memory ops finish in <100ms).
pub(super) fn format_elapsed_badge(tool: &ToolCall) -> Option<String> {
    if !matches!(tool.status, ToolStatus::Completed | ToolStatus::Failed) {
        return None;
    }
    let ms = tool.elapsed_ms?;
    if ms < 100 {
        return None;
    }
    if ms < 10_000 {
        Some(format!("[{:.1}s]", ms as f64 / 1000.0))
    } else if ms < 60_000 {
        Some(format!("[{}s]", ms / 1000))
    } else {
        let mins = ms / 60_000;
        let secs = (ms % 60_000) / 1000;
        Some(format!("[{mins}m {secs}s]"))
    }
}

/// Return the visible file label for a tool title.
///
/// We intentionally do not place OSC 8 hyperlink sequences inside ratatui
/// spans. The terminal backend treats control bytes as zero-width but leaves
/// the printable OSC payload behind in the cell buffer on some paths, which
/// turns a clean `Write(path)` title into raw `]8;;file://...` text. File
/// opening should be implemented through explicit hit regions, not inline
/// terminal controls embedded in the render tree.
fn maybe_osc8_file_link(_path: &str, label: &str) -> String {
    label.to_owned()
}

pub(super) fn tool_progress_verb(kind: &ToolKind) -> &str {
    match kind {
        // Claude 2.1.177 aliases: Write/FileWriteTool -> Writing.
        ToolKind::Write => "Writing",
        // Claude 2.1.177 aliases: Edit/MultiEdit/FileEditTool -> Editing.
        ToolKind::Edit | ToolKind::MultiEdit => "Editing",
        // Claude 2.1.177 alias: NotebookEditTool -> Editing notebook.
        ToolKind::NotebookEdit => "Editing notebook",
        // Claude 2.1.177 aliases: Bash/BashTool -> Running.
        ToolKind::Bash => "Running",
        // Claude 2.1.177 alias: FileReadTool -> Reading.
        ToolKind::Read | ToolKind::NotebookRead => "Reading",
        // Claude 2.1.177 aliases: GlobTool/GrepTool -> Searching.
        ToolKind::Glob | ToolKind::Grep | ToolKind::Search => "Searching",
        ToolKind::BashOutput => "Shell output",
        ToolKind::TaskStop => "TaskStop",
        _ => kind.label(),
    }
}

trait ToolHeaderSummary {
    fn header_summary_tail<'a>(&self, summary: &'a str) -> &'a str;
}

impl ToolHeaderSummary for ToolKind {
    fn header_summary_tail<'a>(&self, summary: &'a str) -> &'a str {
        let summary = summary.trim_start();
        for alias in [self.label(), self.api_name()] {
            if let Some(rest) = strip_redundant_header_colon_prefix(summary, alias) {
                return rest;
            }
        }
        for alias in extra_header_summary_aliases(self) {
            if let Some(rest) = strip_redundant_header_colon_prefix(summary, alias) {
                return rest;
            }
            if let Some(rest) = strip_redundant_header_word_prefix(summary, alias) {
                return rest;
            }
        }
        summary
    }
}

fn extra_header_summary_aliases(kind: &ToolKind) -> &'static [&'static str] {
    match kind {
        ToolKind::AskModel => &["ask", "ask model"],
        ToolKind::AskUserQuestion => &["ask"],
        ToolKind::Council => &["model council"],
        ToolKind::Research => &["deep research"],
        ToolKind::SkillCreate => &["create skill"],
        _ => &[],
    }
}

fn normalized_header_token(s: &str) -> String {
    s.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect()
}

fn strip_redundant_header_colon_prefix<'a>(summary: &'a str, alias: &str) -> Option<&'a str> {
    let (prefix, rest) = summary.split_once(':')?;
    if normalized_header_token(prefix) == normalized_header_token(alias) {
        Some(rest.trim_start())
    } else {
        None
    }
}

fn strip_redundant_header_word_prefix<'a>(summary: &'a str, alias: &str) -> Option<&'a str> {
    let summary = summary.trim_start();
    let prefix = summary.get(..alias.len())?;
    if !prefix.eq_ignore_ascii_case(alias) {
        return None;
    }
    let rest = summary.get(alias.len()..)?;
    let mut chars = rest.chars();
    let first = chars.next()?;
    if first.is_whitespace() {
        Some(chars.as_str().trim_start())
    } else {
        None
    }
}

pub(super) fn build_header_inner_spans<'a>(
    tool: &'a ToolCall,
    t: &Theme,
    max_w: usize,
) -> Vec<Span<'a>> {
    let kind_label = tool.kind.label();
    let summary = tool.input.summary();
    let kind_style = Style::default()
        .fg(tool_kind_color(&tool.kind, t))
        .add_modifier(Modifier::BOLD);

    match &tool.input {
        ToolInput::Bash { command, .. } => {
            // Strip a leading `cd <dir> && ` (or `… ; `) boilerplate prefix
            // — the agent almost always cd's into the repo first, and the
            // long path wastes the whole title width, pushing the real
            // command off the right edge ("…)"). Show what actually ran.
            let first_line = command.lines().next().unwrap_or(command).trim();
            let display = first_line
                .strip_prefix("cd ")
                .and_then(|rest| {
                    rest.split_once(" && ")
                        .or_else(|| rest.split_once("; "))
                        .map(|(_, cmd)| cmd.trim())
                })
                .unwrap_or(first_line);
            // If the command still has more lines (e.g. a multi-line commit
            // message), mark it so the truncation reads as deliberate.
            let multiline = command.lines().nth(1).is_some();
            let cmd = truncate_str(display, max_w.saturating_sub(8));
            let mut spans = vec![
                Span::styled("Bash", kind_style),
                Span::styled("(", Style::default().fg(t.text_muted)),
                Span::styled(cmd, Style::default().fg(t.text_primary)),
            ];
            if multiline {
                spans.push(Span::styled(" …", Style::default().fg(t.text_muted)));
            }
            spans.push(Span::styled(")", Style::default().fg(t.text_muted)));
            spans
        }
        ToolInput::Edit { file_path, .. } => {
            let path = truncate_str(file_path, max_w.saturating_sub(6));
            let linked = maybe_osc8_file_link(file_path, &path);
            vec![
                Span::styled("Edit", kind_style),
                Span::styled("(", Style::default().fg(t.text_muted)),
                Span::styled(linked, Style::default().fg(t.text_primary)),
                Span::styled(")", Style::default().fg(t.text_muted)),
            ]
        }
        ToolInput::MultiEdit { file_path, .. } => {
            let path = truncate_str(file_path, max_w.saturating_sub(6));
            let linked = maybe_osc8_file_link(file_path, &path);
            vec![
                Span::styled("Edit", kind_style),
                Span::styled("(", Style::default().fg(t.text_muted)),
                Span::styled(linked, Style::default().fg(t.text_primary)),
                Span::styled(")", Style::default().fg(t.text_muted)),
            ]
        }
        ToolInput::Write { file_path, .. } => {
            let path = truncate_str(file_path, max_w.saturating_sub(8));
            let linked = maybe_osc8_file_link(file_path, &path);
            vec![
                Span::styled("Write", kind_style),
                Span::styled("(", Style::default().fg(t.text_muted)),
                Span::styled(linked, Style::default().fg(t.text_primary)),
                Span::styled(")", Style::default().fg(t.text_muted)),
            ]
        }
        ToolInput::Read { file_path, .. } => {
            let path = truncate_str(file_path, max_w.saturating_sub(7));
            let linked = maybe_osc8_file_link(file_path, &path);
            vec![
                Span::styled("Read", kind_style),
                Span::styled("(", Style::default().fg(t.text_muted)),
                Span::styled(linked, Style::default().fg(t.text_secondary)),
                Span::styled(")", Style::default().fg(t.text_muted)),
            ]
        }
        ToolInput::NotebookRead { path } => {
            let display_path = truncate_str(path, max_w.saturating_sub(17));
            let linked = maybe_osc8_file_link(path, &display_path);
            vec![
                Span::styled("Read notebook", kind_style),
                Span::styled("(", Style::default().fg(t.text_muted)),
                Span::styled(linked, Style::default().fg(t.text_secondary)),
                Span::styled(")", Style::default().fg(t.text_muted)),
            ]
        }
        ToolInput::NotebookEdit {
            path, edit_mode, ..
        } => {
            let display_path = truncate_str(path, max_w.saturating_sub(22));
            let linked = maybe_osc8_file_link(path, &display_path);
            let verb = tool_progress_verb(&tool.kind);
            let mode = edit_mode.as_deref().unwrap_or("replace");
            vec![
                Span::styled(verb, kind_style),
                Span::styled("(", Style::default().fg(t.text_muted)),
                Span::styled(linked, Style::default().fg(t.text_primary)),
                Span::styled(" · ", Style::default().fg(t.text_muted)),
                Span::styled(mode.to_owned(), Style::default().fg(t.text_secondary)),
                Span::styled(")", Style::default().fg(t.text_muted)),
            ]
        }
        ToolInput::Task(task_input) => {
            let meta = [
                task_input
                    .subagent_type
                    .as_deref()
                    .filter(|s| !s.trim().is_empty()),
                task_input
                    .category
                    .as_deref()
                    .filter(|s| !s.trim().is_empty()),
                task_input.model.as_deref().filter(|s| !s.trim().is_empty()),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
            let meta_w = if meta.is_empty() { 0 } else { meta.len() + 3 };
            let desc = truncate_str(&task_input.description, max_w.saturating_sub(6 + meta_w));
            let label = if task_input.is_teammate_spawn() {
                "Teammate"
            } else {
                "Agent"
            };
            let mut spans = vec![
                Span::styled(label, kind_style),
                Span::styled(" ", Style::default().fg(t.text_muted)),
                Span::styled(desc, Style::default().fg(t.text_primary)),
            ];
            if !meta.is_empty() {
                spans.push(Span::styled(" · ", Style::default().fg(t.text_muted)));
                spans.push(Span::styled(meta, Style::default().fg(t.text_secondary)));
            }
            spans
        }
        ToolInput::BashOutput { task_id, .. } => {
            let id = truncate_str(task_id, max_w.saturating_sub(11));
            vec![
                Span::styled("Shell output", kind_style),
                Span::styled(" ", Style::default().fg(t.text_muted)),
                Span::styled(id, Style::default().fg(t.text_primary)),
            ]
        }
        _ => {
            let display_summary = tool.kind.header_summary_tail(&summary);
            let s = truncate_str(display_summary, max_w.saturating_sub(kind_label.len() + 1));
            vec![
                Span::styled(format!("{kind_label} "), kind_style),
                Span::styled(s, Style::default().fg(t.text_secondary)),
            ]
        }
    }
}

fn bash_output_display_text<'a>(
    tool: &ToolCall,
    raw: &'a str,
) -> Option<std::borrow::Cow<'a, str>> {
    let bash_output_tool = matches!(tool.kind, ToolKind::BashOutput)
        || matches!(tool.input, ToolInput::BashOutput { .. });
    let bash_tool =
        matches!(tool.kind, ToolKind::Bash) || matches!(tool.input, ToolInput::Bash { .. });
    if !bash_output_tool && !bash_tool {
        return None;
    }

    let (metadata, body) = raw.split_once("\n\n").unwrap_or((raw, ""));
    if !looks_like_bash_output_metadata(metadata) {
        return None;
    }

    let body = body.trim_end_matches('\n');
    if !body.trim().is_empty() {
        return Some(std::borrow::Cow::Borrowed(body));
    }

    let retrieval_status = metadata
        .lines()
        .find_map(|line| line.strip_prefix("retrieval_status: "))
        .unwrap_or_default();
    let task_status = metadata
        .lines()
        .find_map(|line| line.strip_prefix("status: "))
        .unwrap_or_default();

    let message =
        if matches!(retrieval_status, "not_ready" | "timeout") || task_status.contains("running") {
            "Task is still running..."
        } else {
            "No task output available"
        };
    Some(std::borrow::Cow::Borrowed(message))
}

fn looks_like_bash_output_metadata(metadata: &str) -> bool {
    let mut has_retrieval = false;
    let mut has_task_id = false;
    let mut has_status = false;
    for line in metadata.lines() {
        if line.starts_with("retrieval_status: ") {
            has_retrieval = true;
        } else if line.starts_with("task_id: ") {
            has_task_id = true;
        } else if line.starts_with("status: ") {
            has_status = true;
        }
    }
    has_retrieval && has_task_id && has_status
}

/// Icon + style for a tool's status. Static for the resolved states
/// (Complete/Failed) and the queued state (Pending). The Running state
/// returns a frame-aware icon so the caller — typically the main
/// renderer with `app.spinner_frame` in hand — can animate it. v126
/// cli.js:323158 pulses tool-use mode at 1Hz via `Math.sin`. We do the
/// equivalent with the same 6-frame spinner cycle the top-of-input
/// spinner uses, so a Running bash tool reads as alive instead of
/// frozen.
fn tool_status_icon(tool: &ToolCall, t: &Theme) -> (&'static str, Style) {
    // ExecutionStatus added two variants (Idle, Cancelled) that tools
    // didn't historically use — Idle is reserved for sub-agent tasks
    // and Cancelled is produced when the user denies a tool. Render
    // both with sensible defaults instead of panicking, so a stray
    // Idle (programmer error) still shows something rather than
    // crashing the renderer.
    match tool.status {
        ToolStatus::Pending => ("○", Style::default().fg(t.text_muted)),
        ToolStatus::Running | ToolStatus::Idle => ("◌", Style::default().fg(t.accent)),
        ToolStatus::Completed => ("●", Style::default().fg(t.success)),
        ToolStatus::Failed => ("✗", Style::default().fg(t.error)),
        ToolStatus::Cancelled => ("⊘", Style::default().fg(t.text_muted)),
    }
}

/// Build the body lines for an `AskModel` cross-model exchange. Renders two
/// attributed sections behind provider-colored side gutters:
///
/// ```text
/// ┃ › You → Claude
/// ┃ why is the sky blue?
/// ║ ▲ GPT
/// ║ Rayleigh scattering — shorter wavelengths scatter more…
/// ```
///
/// The reply gutter uses the *asked* provider's color/glyph/bar (green=GPT,
/// blue=Claude, …); the prompt uses a neutral "you" gutter. Attribution is
/// redundant (color + label + glyph + bar shape) so it survives monochrome
/// terminals and color-vision deficiency.
fn ask_model_exchange_lines(
    model: &str,
    prompt: &str,
    output: &ToolOutput,
    content_w: usize,
    t: Theme,
) -> Vec<Line<'static>> {
    let reply_style = provider_style_for_model(model);
    let mut lines: Vec<Line<'static>> = Vec::new();

    // ── Sent prompt (neutral "you" gutter) ──────────────────────────────
    let sent_bar = "▏";
    let sent_color = t.text_secondary;
    lines.push(Line::from(vec![
        Span::styled(format!("{sent_bar} "), Style::default().fg(sent_color)),
        Span::styled(
            format!("› You → {}", reply_style.label),
            Style::default().fg(sent_color).add_modifier(Modifier::BOLD),
        ),
    ]));
    for chunk in gutter_wrap(prompt, sent_bar, sent_color, content_w, t.text_muted) {
        lines.push(chunk);
    }

    // ── Reply (asked provider's colored gutter) ─────────────────────────
    let reply_text = match output {
        ToolOutput::Text(s) => s.clone(),
        ToolOutput::LargeText(lt) => lt.content.clone(),
        ToolOutput::Empty => String::new(),
        other => format!("{other:?}"),
    };
    lines.push(Line::from(vec![
        Span::styled(
            format!("{} ", reply_style.bar),
            Style::default().fg(reply_style.color),
        ),
        Span::styled(
            format!("{} {}", reply_style.glyph, reply_style.label),
            Style::default()
                .fg(reply_style.color)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    if reply_text.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", reply_style.bar),
                Style::default().fg(reply_style.color),
            ),
            Span::styled(
                "(awaiting reply…)".to_owned(),
                Style::default()
                    .fg(t.text_muted)
                    .add_modifier(Modifier::ITALIC),
            ),
        ]));
    } else {
        for chunk in gutter_wrap(
            &reply_text,
            reply_style.bar,
            reply_style.color,
            content_w,
            t.text_secondary,
        ) {
            lines.push(chunk);
        }
    }

    lines
}

/// Wrap `text` to `content_w`, prefixing every wrapped continuation line with
/// a colored gutter `bar` so the side bar persists down the whole block (a
/// `Block` border can't repeat per logical line inside a `Paragraph`). Caps at
/// 80 lines to match the other body renderers.
fn gutter_wrap(
    text: &str,
    bar: &str,
    bar_color: ratatui::style::Color,
    content_w: usize,
    text_color: ratatui::style::Color,
) -> Vec<Line<'static>> {
    let inner_w = content_w.saturating_sub(2).max(1);
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut count = 0usize;
    'outer: for raw in text.lines() {
        let clean = sanitize_terminal_text(raw);
        for chunk in markdown::hard_wrap_str(&clean, inner_w) {
            if count >= 80 {
                out.push(Line::from(Span::styled(
                    format!(
                        "{bar} {}",
                        terminal_output::expand_hint_text(
                            text.lines().count().saturating_sub(count).max(1),
                            "line",
                        )
                    ),
                    Style::default().fg(bar_color),
                )));
                break 'outer;
            }
            out.push(Line::from(vec![
                Span::styled(format!("{bar} "), Style::default().fg(bar_color)),
                Span::styled(chunk, Style::default().fg(text_color)),
            ]));
            count += 1;
        }
    }
    out
}

/// Tool titles stay intentionally quiet. The status glyph carries state
/// color; the title itself should read like Claude Code's plain
/// `Bash(...)` / `Write(...)` rows instead of a rainbow of tool families.
pub fn tool_kind_color(kind: &ToolKind, t: &Theme) -> ratatui::style::Color {
    match kind {
        ToolKind::Generic(_) => t.text_secondary,
        ToolKind::UnknownTool { .. } => t.text_muted,
        _ => t.text_primary,
    }
}

/// Visual identity for a model provider, used to attribute cross-model
/// exchanges (AskModel results, heterogeneous teammate messages) in the
/// stream. Per the accessibility research, attribution must be redundant —
/// color ALONE fails ~8% of users (color-vision deficiency) — so every
/// provider carries three independent channels: a `color`, a non-color
/// `glyph`, and a distinctly-shaped `bar` character for the side gutter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderStyle {
    /// Human-facing short label, e.g. "Claude", "GPT".
    pub label: &'static str,
    /// Gutter / accent color (ignored under NO_COLOR; glyph+bar+label carry it).
    pub color: ratatui::style::Color,
    /// Non-color sigil shown in the header, distinct per provider family.
    pub glyph: &'static str,
    /// Side-bar character — shape differs per provider so the gutter is
    /// distinguishable in monochrome, not just by hue.
    pub bar: &'static str,
}

/// Provider family a model belongs to. Owns both its classification (from a
/// bare or `provider/`-qualified model id) and its visual identity, so callers
/// never branch on stringly-typed provider names. Enums-over-strings keeps the
/// "which model family is this" decision in one exhaustive place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderFamily {
    Anthropic,
    OpenAI,
    Gemini,
    Other,
}

impl ProviderFamily {
    /// Classify a model id (e.g. "claude-opus-4-7", "openai/gpt-5.5",
    /// "gpt-5.5", "gemini-2.5-pro"). Matches an explicit `provider/` prefix
    /// first, then falls back to model-name heuristics.
    pub fn classify(model: &str) -> Self {
        let lower = model.to_ascii_lowercase();
        let (prefix, bare) = match lower.split_once('/') {
            Some((p, b)) => (p, b),
            None => ("", lower.as_str()),
        };

        if prefix == "anthropic"
            || prefix == "anthropic-oauth"
            || bare.starts_with("claude-")
            || bare.starts_with("fable")
            || bare.starts_with("mythos")
        {
            Self::Anthropic
        } else if prefix == "openai"
            || prefix == "codex"
            || bare.starts_with("gpt-")
            || bare.starts_with("o1")
            || bare.starts_with("o3")
            || bare.starts_with("o4")
            || bare.contains("codex")
        {
            Self::OpenAI
        } else if prefix == "gemini" || prefix == "antigravity" || bare.starts_with("gemini") {
            Self::Gemini
        } else {
            Self::Other
        }
    }

    /// The redundant (color + label + glyph + bar-shape) visual identity for
    /// this family — see [`ProviderStyle`] for why all four channels exist.
    pub fn style(self) -> ProviderStyle {
        use ratatui::style::Color;
        match self {
            Self::Anthropic => ProviderStyle {
                label: "Claude",
                color: Color::Rgb(120, 180, 255), // Anthropic blue gutter
                glyph: "◆",
                bar: "┃",
            },
            Self::OpenAI => ProviderStyle {
                label: "GPT",
                color: Color::Rgb(120, 220, 150), // OpenAI green gutter
                glyph: "▲",
                bar: "║",
            },
            Self::Gemini => ProviderStyle {
                label: "Gemini",
                color: Color::Rgb(190, 160, 250), // violet gutter
                glyph: "●",
                bar: "█",
            },
            Self::Other => ProviderStyle {
                label: "Model",
                color: Color::Rgb(200, 200, 200),
                glyph: "◇",
                bar: "▏",
            },
        }
    }
}

impl From<&str> for ProviderFamily {
    fn from(model: &str) -> Self {
        Self::classify(model)
    }
}

/// Convenience wrapper: classify a model id and return its [`ProviderStyle`].
pub fn provider_style_for_model(model: &str) -> ProviderStyle {
    ProviderFamily::classify(model).style()
}

/// 2-frame pulse for Pending: open ring -> dotted ring. Same column
/// width, just enough motion that "queued behind another tool" reads
/// as queued rather than frozen, but muted so it does not look like an active
/// warning.
const PENDING_FRAMES: &[&str] = &["○", "◌"];
const TASK_RUNNING_FRAMES: &[&str] = &["◌", "◎", "◉", "◎"];

/// Per-frame animated icon. Running tools rotate through the same status-frame
/// spinner as the main turn row, so Bash/Write/Read do not appear to have a
/// separate animation system. Pending tools alternate between `○` and `◌` at a
/// slower cadence so a queued tool reads differently from an idle one.
///
/// Why glyph rotation over color-only blink: terminals with low foreground
/// contrast (light themes, Solarized variants) wash out the bold/muted
/// color toggle to the point of invisibility. A shape change is robust
/// across themes — the user always sees motion. Mirrors v126's tool-use
/// spinner (cli.js:323158) which rotates a glyph on every frame.
pub fn tool_status_icon_animated(
    tool: &ToolCall,
    t: &Theme,
    frame: usize,
) -> (&'static str, Style) {
    match tool.status {
        ToolStatus::Running => {
            if matches!(tool.kind, ToolKind::Task) || matches!(tool.input, ToolInput::Task(_)) {
                let glyph = TASK_RUNNING_FRAMES[(frame / 2) % TASK_RUNNING_FRAMES.len()];
                let bright = (frame / 8).is_multiple_of(2);
                let style = if bright {
                    Style::default()
                        .fg(t.accent)
                        .add_modifier(ratatui::style::Modifier::BOLD)
                } else {
                    Style::default().fg(t.text_muted)
                };
                return (glyph, style);
            }
            // Two-layer animation: the shared glyph rotates, and the color
            // pulse sits on a slower cadence so running tools remain legible
            // in dense transcripts.
            let glyph = crate::spinner::frame_for(frame);
            let bright = (frame / 9).is_multiple_of(2);
            let style = if bright {
                Style::default()
                    .fg(t.accent)
                    .add_modifier(ratatui::style::Modifier::BOLD)
            } else {
                Style::default().fg(t.text_muted)
            };
            (glyph, style)
        }
        ToolStatus::Pending => {
            let glyph = PENDING_FRAMES[(frame / 6) % PENDING_FRAMES.len()];
            (glyph, Style::default().fg(t.text_muted))
        }
        _ => tool_status_icon(tool, t),
    }
}
pub fn border_color_for_status(tool: &ToolCall, t: &Theme) -> Color {
    // Idle is Task-only territory but still valid on the unified
    // ExecutionStatus enum — render with the same accent as Running
    // (the tool, if it ever lands here, is "alive but quiet"). A
    // Cancelled tool drops to muted to match its terminal-but-benign
    // semantics.
    match tool.status {
        ToolStatus::Pending => t.border,
        ToolStatus::Running | ToolStatus::Idle => t.accent,
        ToolStatus::Completed => t.border,
        ToolStatus::Failed => t.error,
        ToolStatus::Cancelled => t.text_muted,
    }
}

/// Body rows that show the *rest* of a Bash command under its title, each
/// rendered with a `┆ ` prefix. The title only has room for a one-line
/// preview, so without this a long invocation reads as `Bash(… RUS…)` with
/// the tail lost. We spill into wrapped continuation rows when the command
/// doesn't fit:
///   * multi-line command → lines 2..N (line 1 is the title), each wrapped;
///   * single long line   → the whole command, wrapped (title shows a preview);
///   * single short line   → nothing (the title already shows it in full).
///
/// `content_w` is the body width (`inner_w - 2`); both the height query and
/// the renderer pass the *same* value so the wrapped row count agrees and the
/// scroll math stays exact. Rows wrap to `content_w - 2` to leave room for the
/// `┆ ` prefix.
pub(super) fn bash_continuation_lines(tool: &ToolCall, content_w: usize) -> Vec<String> {
    let ToolInput::Bash { command, .. } = &tool.input else {
        return Vec::new();
    };
    let src_lines: Vec<&str> = command.lines().collect();
    let spill: Vec<&str> = if src_lines.len() > 1 {
        src_lines[1..].to_vec()
    } else if src_lines.first().map_or(0, |l| l.chars().count()) > content_w {
        src_lines.clone()
    } else {
        return Vec::new();
    };
    let wrap_w = content_w.saturating_sub(2).max(8);
    let mut out = Vec::new();
    for line in spill {
        if line.is_empty() {
            out.push(String::new());
            continue;
        }
        // Hard char-wrap: bash commands are effectively ASCII, so char count
        // tracks display width, and a deterministic chunking keeps the height
        // query and the render in lockstep.
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let end = (i + wrap_w).min(chars.len());
            out.push(chars[i..end].iter().collect());
            i = end;
        }
    }
    out
}

fn render_tool_content_with_skip(
    app: &App,
    tool: &ToolCall,
    area: Rect,
    t: Theme,
    buf: &mut Buffer,
    skip: usize,
) {
    if area.height == 0 {
        return;
    }
    // Show the rest of a Bash command (wrapped) before its output, each
    // continuation row prefixed with `┆ ` in muted color so it nests under
    // the title. Width must match `tool_block_height`'s query (`area.width`
    // here == the height path's `content_w`) so the row count agrees.
    let bash_cont = bash_continuation_lines(tool, area.width as usize);
    let mut local_skip = skip;
    let mut content_y = area.y;
    let mut remaining_h = area.height;
    if !bash_cont.is_empty() {
        for line in &bash_cont {
            if remaining_h == 0 {
                break;
            }
            if local_skip > 0 {
                local_skip -= 1;
                continue;
            }
            let row = Rect {
                x: area.x,
                y: content_y,
                width: area.width,
                height: 1,
            };
            // Truncate to row width so a 200-col heredoc line doesn't
            // spill into the input border below.
            let max_w = (area.width as usize).saturating_sub(2);
            let truncated: String = if line.chars().count() > max_w && max_w > 1 {
                let mut s: String = line.chars().take(max_w.saturating_sub(1)).collect();
                s.push('…');
                s
            } else {
                line.clone()
            };
            Paragraph::new(Line::from(vec![
                Span::styled("┆ ", Style::default().fg(t.text_muted)),
                Span::styled(truncated, Style::default().fg(t.text_secondary)),
            ]))
            .render(row, buf);
            register_bash_command_copy_region(app, tool, row);
            content_y += 1;
            remaining_h -= 1;
        }
    }
    if remaining_h == 0 {
        return;
    }
    let area = Rect {
        x: area.x,
        y: content_y,
        width: area.width,
        height: remaining_h,
    };
    let skip = local_skip;

    // Diff renders directly to buffer because each row needs its own
    // bg-color tint that `Paragraph` can't paint as a per-row band.
    // All other arms route through `tool_body_lines_themed` — the
    // canonical producer the height predictor also walks — so renderer
    // rows == predictor rows by construction.
    if let ToolOutput::Diff(diff) = &tool.output {
        render_diff_skip(diff, area, t, buf, skip, tool.display.is_expanded());
        return;
    }

    let lines = tool_body_lines_themed(tool, area.width as usize, t, Some(app));
    // Two arms historically used `Wrap{trim:false}` so a long inline
    // run (e.g. a long JSON string in a Task result, a wide markdown
    // span from `cat README.md`) word-wraps cleanly instead of
    // clipping at the right edge. `markdown::to_lines` already
    // pre-wraps to width, but the defensive Wrap covers any spans
    // that exceed the area on rendering. Detection mirrors the
    // dispatch in `tool_body_lines_themed` so the predictor and
    // renderer agree on which path was taken.
    let task_md = matches!(
        (&tool.kind, &tool.output),
        (ToolKind::Task, ToolOutput::Text(_))
    ) && infer_lang_from_tool(tool).is_none();
    let cat_md = if let (
        ToolOutput::Command {
            stdout, exit_code, ..
        },
        ToolInput::Bash { command, .. },
    ) = (&tool.output, &tool.input)
    {
        let success = !stdout.is_empty() && exit_code.unwrap_or(-1) == 0;
        let lang_hint = infer_lang_from_bash(command);
        let lang_lc = lang_hint.as_deref().map(|l| l.to_ascii_lowercase());
        let is_markdown_lang = lang_lc
            .as_deref()
            .map(|l| matches!(l, "md" | "markdown" | "mdx" | "mkd" | "mdown"))
            .unwrap_or(false);
        let content_is_md = !is_markdown_lang && looks_like_markdown(stdout);
        let cmd_kind = classify_bash_cmd(command);
        matches!(cmd_kind, BashCmdKind::Other) && success && (is_markdown_lang || content_is_md)
    } else {
        false
    };

    if task_md || cat_md {
        render_lines_with_scroll_wrapped(lines, area, t.bg, skip, buf);
    } else {
        render_lines_with_scroll(lines, area, t.bg, skip, buf);
    }
}

/// The single "lines + scroll → buffer" consumer used by every
/// `render_*_skip` wrapper around a `produce_*_lines` producer.
///
/// Centralising the draw step ensures the height predictor
/// (`tool_body_lines_themed`, which sums the same `produce_*_lines`)
/// and the renderer always agree on row counts: the only difference
/// between them is whether the lines hit the buffer or get measured.
/// Callers that need `.wrap(...)` (markdown blocks where width-aware
/// soft-wrap is desirable) use `render_lines_with_scroll_wrapped`.
fn render_lines_with_scroll(
    lines: Vec<Line<'static>>,
    area: Rect,
    bg: Color,
    skip: usize,
    buf: &mut Buffer,
) {
    Paragraph::new(lines)
        .style(Style::default().bg(bg))
        .scroll((skip as u16, 0))
        .render(area, buf);
}

/// Variant of `render_lines_with_scroll` for markdown-rendered output
/// where `Wrap{trim:false}` matters — it word-wraps long inline runs
/// instead of clipping at the right edge. Without this a Task-tool
/// result whose JSON contains a long string value (e.g. `"message":
/// "Spawned successfully…"`) got cut to `"message": "Spawned su"` with
/// no continuation.
fn render_lines_with_scroll_wrapped(
    lines: Vec<Line<'static>>,
    area: Rect,
    bg: Color,
    skip: usize,
    buf: &mut Buffer,
) {
    Paragraph::new(lines)
        .style(Style::default().bg(bg))
        .wrap(ratatui::widgets::Wrap { trim: false })
        .scroll((skip as u16, 0))
        .render(area, buf);
}

/// Produce `text`-as-markdown lines for use in tool body rendering.
/// Caps at `MAX_LINES` so a runaway agent can't drown the transcript.
fn produce_markdown_block_lines(text: &str, width: usize, t: Theme) -> Vec<Line<'static>> {
    const MAX_LINES: usize = 200;
    let mut lines = markdown::to_lines(text, &t, width.max(1));
    if lines.len() > MAX_LINES {
        let total = lines.len();
        lines.truncate(MAX_LINES);
        lines.push(Line::from(Span::styled(
            format!("… truncated ({total} lines total)"),
            Style::default().fg(t.text_muted),
        )));
    }
    lines
}

fn produce_markdown_block_line_count(text: &str, width: usize, t: Theme) -> usize {
    const MAX_LINES: usize = 200;
    let total = markdown::to_lines(text, &t, width.max(1)).len();
    total.min(MAX_LINES) + usize::from(total > MAX_LINES)
}

pub(super) fn produce_text_block_lines(
    text: &str,
    width: usize,
    text_style: Color,
    t: Theme,
    expanded: bool,
) -> Vec<Line<'static>> {
    // Expanded blocks lift the cap from 80 to 500 so the user can
    // see the full Read/Bash output without leaving the transcript.
    // Click on the tool block (or `o` / Ctrl+O) toggles `expanded`.
    let max_lines = if expanded { 500usize } else { 80usize };
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut count = 0usize;
    let total_rows = soft_wrapped_text_row_count(text, width.max(1));

    'outer: for raw in text.lines() {
        let clean_raw = sanitize_terminal_text(raw);
        let wrapped = soft_wrap_text_str(&clean_raw, width.max(1));
        for chunk in wrapped {
            if count >= max_lines {
                lines.push(terminal_output::expand_hint_line(
                    total_rows.saturating_sub(count),
                    "line",
                    Style::default()
                        .fg(t.text_muted)
                        .add_modifier(Modifier::ITALIC),
                ));
                break 'outer;
            }
            let clean = chunk;
            // Try the git colorizers in order: diffstat first (most
            // specific), then full diff hunks (broader). Falls back
            // to plain styling for any line that doesn't match.
            if let Some(spans) = terminal_output::colorize_diffstat_line(&clean, text_style, t) {
                lines.push(Line::from(spans));
            } else if let Some(spans) = colorize_git_diff_line(&clean, text_style, t) {
                lines.push(Line::from(spans));
            } else if let Some(spans) = colorize_git_status_line(&clean, text_style, t) {
                lines.push(Line::from(spans));
            } else if let Some(spans) = colorize_git_log_line(&clean, text_style, t) {
                lines.push(Line::from(spans));
            } else if let Some(spans) = colorize_git_commit_line(&clean, text_style, t) {
                lines.push(Line::from(spans));
            } else if let Some(spans) = colorize_git_push_line(&clean, text_style, t) {
                lines.push(Line::from(spans));
            } else if let Some(spans) = colorize_diagnostic_prefix(&clean, text_style, t) {
                lines.push(Line::from(spans));
            } else {
                lines.push(Line::from(Span::styled(
                    clean,
                    Style::default().fg(text_style),
                )));
            }
            count += 1;
        }
    }
    lines
}

fn soft_wrap_text_str(s: &str, width: usize) -> Vec<String> {
    use unicode_width::UnicodeWidthChar;

    fn text_width(s: &str) -> usize {
        s.chars()
            .map(|c| UnicodeWidthChar::width(c).unwrap_or(1))
            .sum()
    }

    if width == 0 || text_width(s) <= width {
        return vec![s.to_owned()];
    }

    let mut out = Vec::new();
    let mut buf = String::new();
    let mut col = 0usize;
    let mut last_break: Option<usize> = None;

    for ch in s.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(1);
        if cw > 0 && col + cw > width && !buf.is_empty() {
            if ch.is_whitespace() {
                out.push(buf.trim_end().to_owned());
                buf.clear();
                col = 0;
                last_break = None;
                continue;
            }

            if let Some(idx) = last_break
                && !buf[..idx].trim().is_empty()
            {
                let remainder = buf[idx..].trim_start().to_owned();
                out.push(buf[..idx].trim_end().to_owned());
                buf = remainder;
                col = text_width(&buf);
                last_break = buf
                    .char_indices()
                    .filter_map(|(i, c)| c.is_whitespace().then_some(i))
                    .next_back();
            } else {
                out.push(std::mem::take(&mut buf));
                col = 0;
                last_break = None;
            }
        }

        if ch.is_whitespace() && !buf.trim().is_empty() {
            last_break = Some(buf.len());
        }
        buf.push(ch);
        col += cw;
    }

    if !buf.is_empty() {
        out.push(buf.trim_end().to_owned());
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn soft_wrapped_text_row_count(text: &str, width: usize) -> usize {
    text.lines()
        .map(|raw| soft_wrap_text_str(&sanitize_terminal_text(raw), width.max(1)).len())
        .sum()
}

fn produce_text_block_line_count(
    text: &str,
    width: usize,
    _text_style: Color,
    expanded: bool,
) -> usize {
    let max_lines = if expanded { 500usize } else { 80usize };
    let total_rows = soft_wrapped_text_row_count(text, width.max(1));

    if total_rows > max_lines {
        max_lines + 1
    } else {
        total_rows
    }
}

#[cfg(test)]
mod provider_attribution_tests {
    use super::*;
    use jfc_core::{ChatMessage, Role, ToolCall, ToolInput, ToolKind, ToolOutput};

    // Normal: each known model id maps to its expected provider family.
    #[test]
    fn classify_known_families_normal() {
        assert_eq!(
            ProviderFamily::classify("claude-opus-4-7"),
            ProviderFamily::Anthropic
        );
        assert_eq!(
            ProviderFamily::classify("anthropic/claude-x"),
            ProviderFamily::Anthropic
        );
        assert_eq!(
            ProviderFamily::classify("fable-5"),
            ProviderFamily::Anthropic
        );
        assert_eq!(ProviderFamily::classify("gpt-5.5"), ProviderFamily::OpenAI);
        assert_eq!(
            ProviderFamily::classify("openai/gpt-5.5"),
            ProviderFamily::OpenAI
        );
        assert_eq!(ProviderFamily::classify("o3-mini"), ProviderFamily::OpenAI);
        assert_eq!(
            ProviderFamily::classify("gemini-2.5-pro"),
            ProviderFamily::Gemini
        );
    }

    // Robust: an unknown id falls back to Other.
    #[test]
    fn classify_unknown_is_other_robust() {
        assert_eq!(ProviderFamily::classify("mystery-7"), ProviderFamily::Other);
        assert_eq!(ProviderFamily::classify(""), ProviderFamily::Other);
    }

    // Normal: each family's style channels are distinct (color + glyph + bar),
    // so attribution survives monochrome (glyph/bar differ, not just color).
    #[test]
    fn provider_styles_are_redundantly_distinct_normal() {
        let a = ProviderFamily::Anthropic.style();
        let o = ProviderFamily::OpenAI.style();
        assert_ne!(a.label, o.label);
        assert_ne!(a.glyph, o.glyph);
        assert_ne!(a.bar, o.bar);
        assert_ne!(a.color, o.color);
    }

    fn ask_tool(model: &str, prompt: &str, reply: &str) -> ToolCall {
        ToolCall {
            id: jfc_engine::ids::ToolId::from("t1"),
            kind: ToolKind::AskModel,
            status: jfc_core::ToolStatus::Completed,
            input: ToolInput::AskModel {
                model: model.to_owned(),
                prompt: prompt.to_owned(),
                system: None,
            },
            output: ToolOutput::Text(reply.to_owned()),
            display: jfc_core::ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        }
    }

    // Normal: an AskModel exchange renders the asked provider's gutter bar on
    // every reply line, plus the provider label in the header.
    #[test]
    fn ask_model_exchange_uses_provider_gutter_normal() {
        let t = crate::theme::Theme::dark();
        let tool = ask_tool("gpt-5.5", "why is the sky blue?", "Rayleigh scattering.");
        let lines = tool_body_lines_themed(&tool, 60, t, None);
        let rendered: Vec<String> = lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect();
        let gpt_bar = ProviderFamily::OpenAI.style().bar;
        assert!(
            rendered.iter().any(|l| l.contains("You → GPT")),
            "header missing: {rendered:?}"
        );
        assert!(rendered.iter().any(|l| l.contains("Rayleigh scattering")));
        assert!(
            rendered.iter().any(|l| l.starts_with(gpt_bar)),
            "reply lines must carry the GPT gutter bar: {rendered:?}"
        );
    }

    // Normal: line-count helper matches the actual rendered line count.
    #[test]
    fn ask_model_line_count_matches_render_normal() {
        let t = crate::theme::Theme::dark();
        let tool = ask_tool("claude-opus-4-7", "hi", "hello there");
        let count = tool_body_line_count(&tool, 60);
        let actual = tool_body_lines_themed(&tool, 60, t, None).len();
        assert_eq!(count, actual);
    }

    // Normal: a teammate message gets provider attribution (name + style).
    #[test]
    fn teammate_message_attribution_normal() {
        let mut msg = ChatMessage::user("ignored".to_owned());
        msg.role = Role::Assistant;
        msg.agent_name = Some("gpt-teammate".to_owned());
        msg.model_name = Some("gpt-5.5".to_owned());
        let attr = super::super::core::attribution_for_message_for_test(&msg);
        let (style, who) = attr.expect("teammate should be attributed");
        assert_eq!(who, "gpt-teammate");
        assert_eq!(style.label, ProviderFamily::OpenAI.style().label);
    }

    // Robust: an ordinary assistant message (no agent/model) is not attributed.
    #[test]
    fn plain_assistant_not_attributed_robust() {
        let mut msg = ChatMessage::user("x".to_owned());
        msg.role = Role::Assistant;
        assert!(super::super::core::attribution_for_message_for_test(&msg).is_none());
    }

    // Robust: the stream records the active model on ordinary assistant
    // messages. That must not render a redundant provider header such as
    // `◆ Claude` in normal single-model chat.
    #[test]
    fn plain_assistant_with_model_not_attributed_robust() {
        let mut msg = ChatMessage::user("x".to_owned());
        msg.role = Role::Assistant;
        msg.model_name = Some("claude-opus-4-8".to_owned());
        assert!(super::super::core::attribution_for_message_for_test(&msg).is_none());
    }
}
