use super::assistant_parts::{sanitize_terminal_text, truncate_str};
use super::bash::{BashCmdKind, classify_bash_cmd};
use super::core::diagnostics_for_path;
use super::detection::looks_like_git_diff_output;
use super::output_style::{
    colorize_diagnostic_prefix, colorize_git_commit_line, colorize_git_diff_line,
    colorize_git_log_line, colorize_git_push_line, colorize_git_status_line,
};
use super::outputs::{
    produce_cat_markdown_output_lines, produce_cat_output_lines, produce_command_output_lines,
    produce_compiler_output_lines, produce_file_list_lines, produce_git_diff_output_lines,
    produce_git_log_output_lines, produce_grep_output_lines, produce_hex_dump_output_lines,
    produce_path_list_output_lines, produce_tabular_list_output_lines, render_diff_skip,
};
use super::syntax::{
    infer_lang_from_bash, infer_lang_from_tool, looks_like_markdown,
    produce_highlighted_block_lines, produce_highlighted_with_line_numbers_lines,
};
use super::tool_height::tool_block_height;
use super::*;

/// Produce the exact `Vec<Line>` the body of `tool` will render.
/// Called from both `tool_content_height_with_tool` (for row count)
/// and the renderer (delegating to `Paragraph::new(...)`). Width
/// matches what the renderer's body area gets.
///
/// Returns an empty Vec for the `Diff` arm — diff has its own paint
/// path that walks the `DiffView` directly (it needs per-row bg
/// tints). Use `diff_row_count` for height in that arm.
pub(super) fn tool_body_lines(tool: &ToolCall, content_w: usize) -> Vec<Line<'static>> {
    // Theme is only used for span styling, which doesn't affect row
    // count — pass `dark()` as a dummy. The renderer path uses
    // `tool_body_lines_themed` with the real theme.
    tool_body_lines_themed(tool, content_w, crate::theme::Theme::dark(), None)
}

/// Theme-aware variant. The renderer calls this with the actual
/// theme so the styled spans match what gets painted; the height
/// path uses `tool_body_lines` (which passes a default theme) since
/// only the row count matters there. `app` is needed only for the
/// Read-tool diagnostics gutter; height callers can pass `None`.
pub(super) fn tool_body_lines_themed(
    tool: &ToolCall,
    content_w: usize,
    t: Theme,
    app: Option<&App>,
) -> Vec<Line<'static>> {
    // Body content branch by tool.output (mirrors render_tool_content_with_skip).
    match &tool.output {
        ToolOutput::Empty => Vec::new(),
        ToolOutput::Text(s) => {
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
            let huge = lt.line_count > LargeText::COLLAPSE_LINES
                || lt.content.len() > LargeText::COLLAPSE_BYTES;
            if huge && !tool.display.is_expanded() {
                vec![Line::from(Span::styled(
                    format!("[{} · click or press o to expand]", lt.size_label()),
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
            // Diff renders directly to buffer with per-row bg tinting
            // that doesn't fit `Paragraph`'s model — `tool_content_height_with_tool`
            // routes the diff arm through `diff_row_count` instead of
            // counting `tool_body_lines`. Returning empty here means
            // any path that bypasses `tool_content_height_with_tool`
            // and tries to count `tool_body_lines` for a diff fails
            // loudly (zero rows) rather than silently miscounting.
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

    let frame_idx = (app.launched_at.elapsed().as_millis() / 80) as usize;
    let (status_icon, status_style) = tool_status_icon_animated(tool, &t, frame_idx);

    let full_h = tool_block_height(tool, area.width as usize) as u16;
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

    // Sparkle on tool complete: when this tool just finished
    // successfully, flash a `✦` next to the title for 600ms with a
    // fade. Reduced-motion skips it. Now sits at column 0 (where
    // the gutter used to be) since there's no bar to compete with.
    if skip == 0
        && matches!(tool.status, crate::types::ToolStatus::Completed)
        && !crate::spinner::reduced_motion()
    {
        if let Some((id, when)) = &app.recent_tool_completion {
            if id == &tool.id {
                let age = when.elapsed();
                if age < std::time::Duration::from_millis(600) {
                    let intensity = 1.0 - (age.as_millis() as f32 / 600.0);
                    if area.x < buf.area().right() {
                        let cell = &mut buf[(area.x, area.y)];
                        cell.set_symbol("✦");
                        let blended = crate::render::pulse_color_pub(t.bg, t.accent, intensity);
                        cell.set_style(Style::default().fg(blended));
                    }
                }
            }
        }
    }

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

pub(super) fn build_collapsed_header<'a>(
    tool: &'a ToolCall,
    t: &Theme,
    width: usize,
    elapsed_ms: u128,
) -> Line<'a> {
    let frame_idx = (elapsed_ms / 80) as usize;
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
    std::env::var("JFC_TOOL_TITLE_WIDTH")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| *n >= 20)
        .unwrap_or(100)
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
            "📌 ",
            Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
        ));
    }
    // Reserve a few columns at the right for the optional elapsed
    // badge. `format_elapsed_badge` returns `Some("[2.3s]")` only for
    // completed/failed tools that have a measured duration, otherwise
    // None.
    let badge = format_elapsed_badge(tool);
    let badge_w = badge.as_ref().map(|s| s.chars().count() + 1).unwrap_or(0);
    let effective = width
        .min(tool_title_width_cap())
        .saturating_sub(4 + badge_w);
    spans.extend(build_header_inner_spans(tool, t, effective));
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
            let first_line = command.lines().next().unwrap_or(command);
            let cmd = truncate_str(first_line, max_w.saturating_sub(8));
            vec![
                Span::styled("Bash", kind_style),
                Span::styled("(", Style::default().fg(t.text_muted)),
                Span::styled(cmd, Style::default().fg(t.text_primary)),
                Span::styled(")", Style::default().fg(t.text_muted)),
            ]
        }
        ToolInput::Edit { file_path, .. } => {
            let path = truncate_str(file_path, max_w.saturating_sub(8));
            vec![
                Span::styled("Update", kind_style),
                Span::styled("(", Style::default().fg(t.text_muted)),
                Span::styled(path, Style::default().fg(t.text_primary)),
                Span::styled(")", Style::default().fg(t.text_muted)),
            ]
        }
        ToolInput::Write { file_path, .. } => {
            let path = truncate_str(file_path, max_w.saturating_sub(8));
            vec![
                Span::styled("Write", kind_style),
                Span::styled("(", Style::default().fg(t.text_muted)),
                Span::styled(path, Style::default().fg(t.text_primary)),
                Span::styled(")", Style::default().fg(t.text_muted)),
            ]
        }
        ToolInput::Read { file_path, .. } => {
            let path = truncate_str(file_path, max_w.saturating_sub(7));
            vec![
                Span::styled("Read", kind_style),
                Span::styled("(", Style::default().fg(t.text_muted)),
                Span::styled(path, Style::default().fg(t.text_secondary)),
                Span::styled(")", Style::default().fg(t.text_muted)),
            ]
        }
        _ => {
            let s = truncate_str(&summary, max_w.saturating_sub(kind_label.len() + 1));
            vec![
                Span::styled(format!("{kind_label} "), kind_style),
                Span::styled(s, Style::default().fg(t.text_secondary)),
            ]
        }
    }
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
        ToolStatus::Pending => ("○", Style::default().fg(t.warning)),
        ToolStatus::Running | ToolStatus::Idle => ("◌", Style::default().fg(t.accent)),
        ToolStatus::Completed => ("●", Style::default().fg(t.success)),
        ToolStatus::Failed => ("✗", Style::default().fg(t.error)),
        ToolStatus::Cancelled => ("⊘", Style::default().fg(t.text_muted)),
    }
}

/// Distinct accent color per tool kind. The gutter bar and tool name
/// span both pick this color (mixed with status state for Running /
/// Failed) so the user can spot at a glance "this is a Bash" vs
/// "this is a Read" without reading the label. Mirrors Claude Code's
/// per-tool color identity.
///
/// Picks are tuned for the dark theme to stay distinguishable from
/// each other AND from status colors: success (green) and error (red)
/// are reserved for status indicators, so Read/Write/etc. use blues,
/// purples, and ambers that don't collide.
pub fn tool_kind_color(kind: &ToolKind, t: &Theme) -> ratatui::style::Color {
    use ratatui::style::Color;
    match kind {
        ToolKind::Read => Color::Rgb(120, 180, 255), // soft blue
        ToolKind::Write => Color::Rgb(255, 200, 130), // amber
        ToolKind::Edit | ToolKind::ApplyPatch => Color::Rgb(160, 230, 170), // mint
        ToolKind::Bash => Color::Rgb(180, 180, 200), // neutral grey
        ToolKind::Glob | ToolKind::Grep | ToolKind::Search => Color::Rgb(200, 160, 255), // lavender
        ToolKind::Task => Color::Rgb(255, 170, 220), // rose
        ToolKind::TaskCreate
        | ToolKind::TaskUpdate
        | ToolKind::TaskList
        | ToolKind::TaskDone
        | ToolKind::TaskGet
        | ToolKind::TaskValidate => Color::Rgb(140, 220, 220), // teal
        ToolKind::MemoryCreate | ToolKind::MemoryDelete => Color::Rgb(220, 220, 140), // olive
        ToolKind::TeamCreate
        | ToolKind::TeamDelete
        | ToolKind::SendMessage
        | ToolKind::TeamMemberMode => Color::Rgb(255, 150, 130), // coral
        ToolKind::Skill => Color::Rgb(180, 220, 255), // ice
        ToolKind::ToolSearch | ToolKind::ToolSuggest => Color::Rgb(170, 210, 180),
        ToolKind::GraphQuery | ToolKind::SymbolEdit | ToolKind::RunCoverage => {
            Color::Rgb(130, 200, 180)
        } // sage
        ToolKind::PostBounty | ToolKind::RunBounty | ToolKind::MarketStatus => {
            Color::Rgb(255, 215, 100)
        } // gold
        ToolKind::ExitPlanMode => Color::Rgb(170, 200, 255),
        ToolKind::MultiEdit => Color::Rgb(160, 230, 170),
        ToolKind::AskUserQuestion => Color::Rgb(255, 200, 240),
        ToolKind::WebFetch | ToolKind::WebSearch => Color::Rgb(120, 200, 220),
        // Server-side tools: cyan-teal to distinguish them from local WebSearch
        ToolKind::ServerWebSearch => Color::Rgb(80, 210, 200),
        ToolKind::ServerCodeExecution => Color::Rgb(200, 160, 80), // amber-gold
        ToolKind::Mcp(_) => Color::Rgb(190, 170, 240),
        ToolKind::CronCreate
        | ToolKind::CronList
        | ToolKind::CronDelete
        | ToolKind::ScheduleWakeup
        | ToolKind::Monitor => Color::Rgb(180, 200, 255),
        ToolKind::Lsp => Color::Rgb(140, 200, 240),
        ToolKind::PushNotification | ToolKind::RemoteTrigger => Color::Rgb(255, 180, 110),
        ToolKind::EnterPlanMode | ToolKind::EnterWorktree | ToolKind::ExitWorktree => {
            Color::Rgb(180, 220, 180)
        }
        ToolKind::NotebookRead | ToolKind::NotebookEdit => Color::Rgb(255, 170, 100),
        ToolKind::ScratchpadRead | ToolKind::ScratchpadWrite => Color::Rgb(200, 200, 160), // warm grey
        ToolKind::Generic(_) => t.text_secondary,
        // Unknown tools render in a muted style — they're never
        // dispatched (permission layer denies them), so the row is
        // really just a record of "the model asked for this name and
        // we refused." Use text_muted to make it visually distinct
        // from a normal Generic row.
        ToolKind::UnknownTool { .. } => t.text_muted,
    }
}

/// 4-frame star-burst rotation used for Running tools — same shape family
/// as v126's tool-use indicator (Claude Code shows alternating `* ✱ +`
/// glyphs as the bullet). Each frame is one codepoint so column width
/// stays stable regardless of which frame is showing.
const RUNNING_FRAMES: &[&str] = &["✶", "✷", "✸", "✹"];

/// 2-frame pulse for Pending: open ring → dotted ring. Same column
/// width, just enough motion that "queued behind another tool" reads
/// as queued rather than frozen.
const PENDING_FRAMES: &[&str] = &["○", "◌"];

/// Per-frame animated icon. Running tools rotate through the star-burst
/// frames at ~120ms each (one frame per ~1.5 ticks), so the bullet
/// visibly steps through the cycle instead of just two-tone blinking the
/// same shape — that was indistinguishable from a static `●` on most
/// terminal themes. Pending tools alternate between `○` and `◌` at a
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
            // Two-layer animation:
            //  - Glyph rotates slowly (every 4 ticks ≈ 320ms per frame,
            //    full cycle ≈ 1.28s). Rotation tells the eye "this is
            //    moving" without strobing.
            //  - Color pulses at a different cadence (every 9 ticks ≈
            //    720ms BOLD ⇄ DIM) so the two effects don't sync into
            //    a single distracting beat.
            // Picked the prime-ish 4 vs 9 spacing so the two
            // periodicities take ~25 ticks (2s) to align — beyond
            // perceptual gestalt.
            let glyph = RUNNING_FRAMES[(frame / 4) % RUNNING_FRAMES.len()];
            let bright = (frame / 9) % 2 == 0;
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
            (glyph, Style::default().fg(t.warning))
        }
        _ => tool_status_icon(tool, t),
    }
}

pub(super) fn border_color_for_status(tool: &ToolCall, t: &Theme) -> Color {
    // Idle is Task-only territory but still valid on the unified
    // ExecutionStatus enum — render with the same accent as Running
    // (the tool, if it ever lands here, is "alive but quiet"). A
    // Cancelled tool drops to muted to match its terminal-but-benign
    // semantics.
    match tool.status {
        ToolStatus::Pending => t.warning,
        ToolStatus::Running | ToolStatus::Idle => t.accent,
        ToolStatus::Completed => t.border,
        ToolStatus::Failed => t.error,
        ToolStatus::Cancelled => t.text_muted,
    }
}

#[allow(dead_code)]
fn render_tool_content_clipped(app: &App, tool: &ToolCall, area: Rect, t: Theme, buf: &mut Buffer) {
    render_tool_content_with_skip(app, tool, area, t, buf, 0);
}

/// Lines 2+ of a multi-line Bash command (the heredoc body, the `&&`
/// chain wrapped, etc.) — the title only shows line 1 due to the
/// title-width cap. Without rendering the rest, a `cat > file << 'EOF'\n
/// <... source ...>\nEOF` invocation would only ever show the `cat >`
/// line, hiding what was actually written. Mirrors v126's behavior of
/// showing the full command body as part of the tool block.
pub(super) fn bash_continuation_lines(tool: &ToolCall) -> Vec<String> {
    if let ToolInput::Bash { command, .. } = &tool.input {
        let lines: Vec<&str> = command.lines().collect();
        if lines.len() > 1 {
            return lines.iter().skip(1).map(|s| (*s).to_owned()).collect();
        }
    }
    Vec::new()
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
    // For multi-line Bash commands, show the rest of the command body
    // before the output. Each continuation line is prefixed with `┆ ` in
    // muted color so it visually nests under the title and reads as
    // continuation of the same invocation.
    let bash_cont = bash_continuation_lines(tool);
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
            .style(Style::default().bg(t.bg))
            .render(row, buf);
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

    'outer: for raw in text.lines() {
        let clean_raw = sanitize_terminal_text(raw);
        let wrapped = markdown::hard_wrap_str(&clean_raw, width.max(1));
        for chunk in wrapped {
            if count >= max_lines {
                let total = text.lines().count();
                lines.push(Line::from(Span::styled(
                    format!(
                        "… {} more lines · click or press o to expand",
                        total.saturating_sub(count)
                    ),
                    Style::default()
                        .fg(t.text_muted)
                        .add_modifier(Modifier::ITALIC),
                )));
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
