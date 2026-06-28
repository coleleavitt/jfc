use super::*;
pub(super) fn info_sidebar(f: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme;

    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(t.style_border)
        .padding(Padding::new(1, 0, 1, 0))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    // Helper: section header. Bold + text_primary (white). Earlier
    // I had this as bold + accent (cyan), but every section header
    // pulling the same accent color flattened the hierarchy — your
    // eye couldn't tell a structural section title from a value
    // that *also* used accent (e.g. the model-name sub-header). The
    // three-tier rule wins: primary white bold for sections, accent
    // for sub-headings/values, muted for body. Mirrors how Claude
    // Code's actual sidebar treats its section labels (cli.js'
    // panel renderers use `text` color + bold for the header line,
    // reserving accent for interactive or live elements).
    let section = |label: &'static str| -> Line<'static> {
        Line::from(vec![Span::styled(
            label,
            Style::default()
                .fg(t.text_primary)
                .add_modifier(Modifier::BOLD),
        )])
    };

    lines.push(section("Session"));

    // Show the user-readable title first (custom `/rename` → first prompt →
    // formatted-id-timestamp fallback). Stash the raw session id in a
    // muted second row so the user can still see / copy it.
    let (title, id_str) = match app.engine.current_session_id.as_ref() {
        Some(id) => {
            let id_str = id.as_str().to_owned();
            let title = app
                .session_sidebar
                .meta
                .iter()
                .find(|m| m.id == *id)
                .map(|m| m.display_title())
                .unwrap_or_else(|| id_str.clone());
            (title, id_str)
        }
        None => ("untitled".to_owned(), String::new()),
    };
    lines.push(Line::from(vec![Span::styled(
        truncate_str(&title, inner.width as usize),
        Style::default()
            .fg(t.text_primary)
            .add_modifier(Modifier::BOLD),
    )]));
    if !id_str.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            format!(
                "  {}",
                truncate_str(&id_str, inner.width.saturating_sub(2) as usize)
            ),
            Style::default().fg(t.text_muted),
        )]));
    }

    lines.push(Line::from(""));

    let total_tokens = app.engine.tool_ctx.approx_tokens as u64;
    let ctx_max = app.engine.selected_context_window_tokens().max(1) as u64;
    let pct = (total_tokens as f64 / ctx_max as f64 * 100.0).min(100.0);
    let context_rows = context_breakdown_rows(app, t);
    let context_row_total = context_rows
        .iter()
        .map(|row| row.tokens)
        .sum::<u64>()
        .max(total_tokens)
        .max(1);

    lines.push(Line::from(vec![
        Span::styled(
            " ▼ Context ",
            Style::default()
                .fg(t.bg)
                .bg(t.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {:>4.0}%", pct),
            Style::default().fg(gauge_color(pct, t)),
        ),
    ]));
    lines.push(Line::from(vec![Span::styled(
        format!("{} / {}", fmt_number(total_tokens), fmt_number(ctx_max)),
        Style::default().fg(t.text_secondary),
    )]));

    let bar_width = inner.width.saturating_sub(1) as usize;
    if bar_width > 4 {
        lines.push(context_breakdown_bar(
            &context_rows,
            context_row_total,
            bar_width,
            t,
        ));
    }

    for row in &context_rows {
        let percent = row.tokens.saturating_mul(100) / context_row_total;
        let inner_width = inner.width as usize;
        let percent_width = 7.min(inner_width);
        let token_width = 9.min(inner_width.saturating_sub(percent_width.saturating_add(1)));
        let spacer_width = usize::from(token_width > 0 && percent_width > 0);
        let value_width = token_width
            .saturating_add(spacer_width)
            .saturating_add(percent_width);
        let label_width = inner_width.saturating_sub(value_width);
        lines.push(Line::from(vec![
            Span::styled(
                align_left(&truncate_str(row.label, label_width), label_width),
                Style::default().fg(row.color),
            ),
            Span::styled(
                align_right(&fmt_number(row.tokens), token_width),
                Style::default().fg(t.text_secondary),
            ),
            Span::styled(" ".repeat(spacer_width), Style::default().fg(t.text_muted)),
            Span::styled(
                align_right(&format!("({percent}%)"), percent_width),
                Style::default().fg(t.text_secondary),
            ),
        ]));
    }

    if app.engine.token_history.len() >= 2 {
        const BARS: &[char] = &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        let max_val = app
            .engine
            .token_history
            .iter()
            .copied()
            .max()
            .unwrap_or(1)
            .max(1);
        let bar_width = (inner.width as usize).min(app.engine.token_history.len());
        // Take the most recent N values so a long history doesn't
        // squish the recent samples into single-cell averages.
        let start = app.engine.token_history.len().saturating_sub(bar_width);
        let bars: String = app
            .engine
            .token_history
            .iter()
            .skip(start)
            .map(|v| {
                let idx =
                    (((*v as f64) / max_val as f64) * (BARS.len() - 1) as f64).round() as usize;
                BARS[idx.min(BARS.len() - 1)]
            })
            .collect();
        lines.push(Line::from(vec![
            Span::styled("tok/turn ", Style::default().fg(t.text_muted)),
            Span::styled(bars, Style::default().fg(t.accent)),
        ]));
    }

    lines.push(Line::from(""));

    super::sidebar_panels::push_metrics_section(&mut lines, app, inner.width, t);
    super::sidebar_panels::push_plugin_panels_section(&mut lines, app, inner.width, t);
    super::sidebar_panels::push_plugin_widgets_section(&mut lines, app, inner.width, t);
    super::sidebar_panels::push_usage_by_model_section(&mut lines, app, inner.width, t);
    super::sidebar_panels::push_lsp_section(&mut lines, app, inner.width, t);
    super::sidebar_panels::push_mcp_section(&mut lines, app, inner.width, t);

    let plugin_rows = super::status_plugins::plugin_health_detail_render_rows(
        &app.plugins.health,
        app.plugins.reload_report.as_ref(),
        &app.plugins.runtime_action_descriptors,
    );
    if !plugin_rows.is_empty() {
        lines.push(section("Plugins"));
        let plugin_health = super::status_plugins::plugin_detail_health(
            &app.plugins.health,
            app.plugins.reload_report.as_ref(),
        );
        let plugin_color = if super::status_plugins::plugin_health_is_alert(plugin_health) {
            t.error
        } else if super::status_plugins::plugin_health_is_warning(plugin_health) {
            t.warning
        } else {
            t.success
        };
        for row in &plugin_rows {
            let color = match row.tone {
                super::status_plugins::PluginDetailRowTone::Health => plugin_color,
                super::status_plugins::PluginDetailRowTone::Muted => t.text_muted,
                super::status_plugins::PluginDetailRowTone::Error => t.error,
                super::status_plugins::PluginDetailRowTone::Warning => t.warning,
            };
            lines.push(Line::from(vec![Span::styled(
                format!(
                    "  {}",
                    truncate_str(&row.text, inner.width.saturating_sub(2) as usize)
                ),
                Style::default().fg(color),
            )]));
        }
        lines.push(Line::from(""));
    }

    // Team section - show active teammates. Single-blank separator
    // is enough; the section() helper's gutter glyph already gives
    // the eye an anchor, no need for a double-row break.
    if app.engine.team_context.is_active() {
        lines.push(section("Team"));

        if let Some(ref team_name) = app.engine.team_context.team_name {
            lines.push(Line::from(vec![Span::styled(
                format!("  {team_name}"),
                Style::default().fg(t.text_secondary),
            )]));
        }

        // Surface each teammate as one row. Color the active-marker
        // dot with the teammate's assigned palette color (mirrors the
        // teammate-tree below) so the team panel and the spinner-row
        // tree read the same way.
        for info in app.engine.team_context.teammates.values() {
            if info.name == jfc_engine::swarm::TEAM_LEAD_NAME {
                continue;
            }
            let dot_color = crate::theme::teammate_color(info.color.as_deref());
            lines.push(Line::from(vec![
                Span::styled("  ● ", Style::default().fg(dot_color)),
                Span::styled(&info.name, Style::default().fg(t.text_secondary)),
            ]));
        }

        if app.engine.team_context.teammates.len() <= 1 {
            lines.push(Line::from(vec![Span::styled(
                "  (no teammates)",
                Style::default().fg(t.text_secondary),
            )]));
        }
    }

    // Tasks moved out of this sidebar: they now render as a pinned row
    // directly above the input box (`tasks_pinned_row` below), Claude-Code
    // style. Keeps todo state visible while you type a follow-up without
    // having to glance to the far right column.

    // Diffs section - count files with edit/write tool outputs
    let diff_stats = collect_diff_stats(app);
    if diff_stats.total_files > 0 {
        lines.push(Line::from(vec![Span::styled(
            "Changes",
            Style::default()
                .fg(t.text_primary)
                .add_modifier(Modifier::BOLD),
        )]));

        lines.push(Line::from(vec![
            Span::styled(
                format!("{} file(s)", diff_stats.total_files),
                Style::default().fg(t.text_secondary),
            ),
            Span::raw(" "),
            Span::styled(
                format!("+{}", diff_stats.additions),
                Style::default().fg(t.success),
            ),
            Span::styled(" ", Style::default()),
            Span::styled(
                format!("-{}", diff_stats.deletions),
                Style::default().fg(t.error),
            ),
        ]));

        // Show up to 3 most recently modified files
        for file in diff_stats.files.iter().take(3) {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    truncate_str(file, inner.width.saturating_sub(4) as usize),
                    Style::default().fg(t.accent),
                ),
            ]));
        }
        if diff_stats.files.len() > 3 {
            lines.push(Line::from(vec![Span::styled(
                format!("  … +{} more", diff_stats.files.len() - 3),
                Style::default().fg(t.text_muted),
            )]));
        }

        lines.push(Line::from(""));
    }

    // The cwd + provider + effort + fast + claude-status footer that used
    // to live here was redundant with the bottom status bar — both showed
    // `~/path · model · effort · ⚡ · branch · cost · msgs`. Cleaned up so
    // the sidebar focuses on stuff the bottom bar *doesn't* surface
    // (Context gauge, Usage-by-model breakdown, Changes/diff stats,
    // MCP/LSP rosters, recent sessions). The body now uses the full
    // sidebar height.
    let body_area = inner;
    // Body scrolls — long content used to overflow the panel silently.
    // Clamp scroll so at least one row stays visible.
    let total_body_rows = lines.len() as u16;
    let max_scroll = total_body_rows.saturating_sub(body_area.height.max(1));
    if app.info_sidebar.scroll > max_scroll {
        app.info_sidebar.scroll = max_scroll;
    }
    let scroll_y = app.info_sidebar.scroll;
    f.render_widget(
        Paragraph::new(lines)
            .scroll((scroll_y, 0))
            .style(Style::default().bg(t.bg)),
        body_area,
    );
}

struct ContextBreakdownRow {
    label: &'static str,
    tokens: u64,
    color: ratatui::style::Color,
}

struct TranscriptBreakdown {
    conversation_tokens: u64,
    tool_call_tokens: u64,
    compartment_tokens: u64,
}

fn context_breakdown_rows(app: &App, theme: Theme) -> Vec<ContextBreakdownRow> {
    let transcript = transcript_breakdown(&app.engine.messages);
    let budget = app
        .engine
        .current_stream_request
        .as_ref()
        .and_then(|metadata| metadata.context_budget)
        .or(app.engine.last_context_budget);
    let transcript_total = transcript
        .conversation_tokens
        .saturating_add(transcript.tool_call_tokens)
        .saturating_add(transcript.compartment_tokens);
    let conversation_tokens = budget
        .map(|budget| budget.user_message_tokens.saturating_sub(transcript_total))
        .unwrap_or(0)
        .saturating_add(transcript.conversation_tokens);

    let system_tokens = budget
        .map(|budget| budget.system_prompt_tokens)
        .unwrap_or_else(|| app.engine.last_system_prompt_len.unwrap_or(0) as u64);
    let docs_tokens = budget
        .map(|budget| budget.project_instructions_tokens)
        .unwrap_or(0);
    let memory_tokens = budget.map(|budget| budget.memory_tokens).unwrap_or(0);
    let tool_def_tokens = budget
        .map(|budget| budget.tool_definition_tokens)
        .unwrap_or(0);

    vec![
        ContextBreakdownRow {
            label: "System",
            tokens: system_tokens,
            color: theme.accent,
        },
        ContextBreakdownRow {
            label: "Docs",
            tokens: docs_tokens,
            color: theme.accent_secondary,
        },
        ContextBreakdownRow {
            label: "Compartments",
            tokens: transcript.compartment_tokens,
            color: theme.text_secondary,
        },
        ContextBreakdownRow {
            label: "Memories",
            tokens: memory_tokens,
            color: theme.success,
        },
        ContextBreakdownRow {
            label: "Conversation",
            tokens: conversation_tokens,
            color: theme.error,
        },
        ContextBreakdownRow {
            label: "Tool Calls",
            tokens: transcript.tool_call_tokens,
            color: theme.warning,
        },
        ContextBreakdownRow {
            label: "Tool Defs",
            tokens: tool_def_tokens,
            color: theme.reasoning_fg,
        },
    ]
}

fn transcript_breakdown(messages: &[jfc_core::ChatMessage]) -> TranscriptBreakdown {
    let mut out = TranscriptBreakdown {
        conversation_tokens: 0,
        tool_call_tokens: 0,
        compartment_tokens: 0,
    };
    for message in messages {
        let compartment_message = message
            .parts
            .iter()
            .any(|part| matches!(part, jfc_core::MessagePart::CompactBoundary { .. }));
        for part in &message.parts {
            match part {
                jfc_core::MessagePart::Tool(tool) => {
                    out.tool_call_tokens = out
                        .tool_call_tokens
                        .saturating_add(tokens_from_chars(tool.input.summary().len()))
                        .saturating_add(tokens_from_chars(tool.output.approx_text_len()));
                }
                jfc_core::MessagePart::CompactBoundary { .. }
                | jfc_core::MessagePart::ReasoningSignature(_) => {}
                _ if compartment_message => {
                    out.compartment_tokens = out
                        .compartment_tokens
                        .saturating_add(tokens_from_chars(part.approx_text_len()));
                }
                _ => {
                    out.conversation_tokens = out
                        .conversation_tokens
                        .saturating_add(tokens_from_chars(part.approx_text_len()));
                }
            }
        }
    }
    out
}

fn context_breakdown_bar<'a>(
    rows: &[ContextBreakdownRow],
    total: u64,
    width: usize,
    theme: Theme,
) -> Line<'a> {
    let active = rows.iter().filter(|row| row.tokens > 0).collect::<Vec<_>>();
    if active.is_empty() {
        return Line::from(Span::styled(
            "░".repeat(width),
            Style::default().fg(theme.border),
        ));
    }

    let mut used = 0usize;
    let mut spans = Vec::new();
    for (idx, row) in active.iter().enumerate() {
        let last = idx + 1 == active.len();
        let cells = if last {
            width.saturating_sub(used)
        } else {
            ((row.tokens as f64 / total as f64) * width as f64)
                .round()
                .max(1.0) as usize
        }
        .min(width.saturating_sub(used));
        if cells > 0 {
            spans.push(Span::styled(
                "█".repeat(cells),
                Style::default().fg(row.color),
            ));
            used = used.saturating_add(cells);
        }
    }
    if used < width {
        spans.push(Span::styled(
            "░".repeat(width - used),
            Style::default().fg(theme.border),
        ));
    }
    Line::from(spans)
}

fn tokens_from_chars(chars: usize) -> u64 {
    u64::try_from(chars.saturating_add(3) / 4).unwrap_or(u64::MAX)
}

fn align_right(value: &str, width: usize) -> String {
    let value_width = unicode_width::UnicodeWidthStr::width(value);
    if value_width >= width {
        value.to_owned()
    } else {
        format!("{}{}", " ".repeat(width - value_width), value)
    }
}

fn align_left(value: &str, width: usize) -> String {
    let value_width = unicode_width::UnicodeWidthStr::width(value);
    if value_width >= width {
        value.to_owned()
    } else {
        format!("{}{}", value, " ".repeat(width - value_width))
    }
}
