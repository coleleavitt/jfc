use super::*;

pub(super) fn push_metrics_section<'a>(
    lines: &mut Vec<Line<'a>>,
    app: &App,
    width: u16,
    theme: Theme,
) {
    let mut metric_rows = super::status_plugins::cache_metric_rows(
        &app.plugins.metric_descriptors,
        &app.engine.usage_by_model,
        app.plugins.reload_report.as_ref(),
    );
    metric_rows.extend(super::status_plugins::rsi_metric_rows(
        &app.plugins.metric_descriptors,
        app.engine.current_stream_request.as_ref(),
    ));
    let panel_rows =
        super::status_plugins::metric_panel_descriptor_rows(&app.plugins.metric_descriptors);
    if !panel_rows.is_empty() {
        metric_rows.push("panel metrics".to_owned());
        metric_rows.extend(panel_rows.into_iter().map(|row| format!("panel {row}")));
    }
    if metric_rows.is_empty() {
        return;
    }
    lines.push(section("Metrics", theme));
    push_muted_rows(lines, metric_rows, width, theme);
    lines.push(Line::from(""));
}

pub(super) fn push_plugin_widgets_section<'a>(
    lines: &mut Vec<Line<'a>>,
    app: &App,
    width: u16,
    theme: Theme,
) {
    let widget_rows = super::status_widgets::ui_widget_panel_rows(
        &app.plugins.ui_widget_descriptors,
        &app.plugins.ui_widget_snapshots,
        &app.plugins.ui_widget_refresh_status,
        app.info_sidebar.focused_widget.as_ref(),
    );
    if widget_rows.is_empty() {
        return;
    }
    lines.push(section("Plugin widgets", theme));
    push_muted_rows(lines, widget_rows, width, theme);
    lines.push(Line::from(""));
}

pub(super) fn push_plugin_panels_section<'a>(
    lines: &mut Vec<Line<'a>>,
    app: &App,
    width: u16,
    theme: Theme,
) {
    let panel_sections = super::status_panels::info_sidebar_panel_sections(
        &app.plugins.ui_panel_descriptors,
        &app.plugins.ui_panel_snapshots,
        &app.plugins.ui_panel_refresh_status,
        app.info_sidebar.focused_panel.as_ref(),
    );
    if panel_sections.is_empty() {
        return;
    }
    lines.push(section("Plugin panels", theme));
    for panel in panel_sections {
        lines.push(Line::from(vec![Span::styled(
            format!(
                " {}",
                truncate_str(&panel.title, width.saturating_sub(1) as usize)
            ),
            Style::default().fg(theme.accent),
        )]));
        for row in panel.rows {
            for wrapped in wrap_text_to_width(&row, width.saturating_sub(2) as usize) {
                lines.push(Line::from(vec![Span::styled(
                    format!("  {wrapped}"),
                    Style::default().fg(theme.text_muted),
                )]));
            }
        }
    }
    lines.push(Line::from(""));
}

pub(super) fn push_usage_by_model_section<'a>(
    lines: &mut Vec<Line<'a>>,
    app: &App,
    width: u16,
    theme: Theme,
) {
    if app.engine.usage_by_model.is_empty() {
        return;
    }
    lines.push(section("Usage by model", theme));
    let mut model_entries = app.engine.usage_by_model.iter().collect::<Vec<_>>();
    model_entries.sort_by_key(|(k, _)| k.as_str());

    for (model_name, usage) in &model_entries {
        lines.push(Line::from(vec![Span::styled(
            format!(
                " {}:",
                truncate_str(model_name, width.saturating_sub(2) as usize)
            ),
            Style::default().fg(theme.accent),
        )]));
        lines.push(Line::from(vec![Span::styled(
            format!(
                "  {} in, {} out",
                fmt_number(usage.input_tokens),
                fmt_number(usage.output_tokens),
            ),
            Style::default().fg(theme.text_muted),
        )]));
        if usage.cache_read_tokens > 0 || usage.cache_write_tokens > 0 {
            lines.push(Line::from(vec![Span::styled(
                format!(
                    "  {} cache read, {} write",
                    fmt_number(usage.cache_read_tokens),
                    fmt_number(usage.cache_write_tokens),
                ),
                Style::default().fg(theme.text_muted),
            )]));
            let hit_pct = usage.cache_hit_pct();
            if hit_pct > 0.0 {
                lines.push(Line::from(vec![
                    Span::styled("  cache hit: ", Style::default().fg(theme.text_muted)),
                    Span::styled(format!("{hit_pct:.0}%"), Style::default().fg(theme.success)),
                ]));
            }
        }
        if let Some(cost) = usage.cost_usd {
            lines.push(Line::from(vec![Span::styled(
                format!("  ${cost:.2} spent"),
                Style::default().fg(theme.text_secondary),
            )]));
        }
    }

    let total = jfc_engine::cost::total_cost(&app.engine.usage_by_model);
    if total > 0.001 {
        lines.push(Line::from(vec![Span::styled(
            format!("Total cost: {}", jfc_engine::cost::fmt_cost(total)),
            Style::default().fg(theme.text_muted),
        )]));
    }
    lines.push(Line::from(""));
}

pub(super) fn push_lsp_section<'a>(lines: &mut Vec<Line<'a>>, app: &App, width: u16, theme: Theme) {
    lines.push(section("LSP", theme));
    if app.engine.lsp_servers.is_empty() {
        for row in wrap_text_to_width("LSPs will activate as files are read", width as usize) {
            lines.push(Line::from(vec![Span::styled(
                row,
                Style::default().fg(theme.text_muted),
            )]));
        }
    } else {
        for srv in &app.engine.lsp_servers {
            let (dot_color, label) = match srv.status {
                jfc_core::LspStatus::Active => (theme.success, "Active"),
                jfc_core::LspStatus::Inactive => (theme.text_muted, "Inactive"),
            };
            lines.push(Line::from(vec![
                Span::styled("• ", Style::default().fg(dot_color)),
                Span::styled(
                    truncate_str(&srv.name, width.saturating_sub(12) as usize),
                    Style::default().fg(theme.accent),
                ),
                Span::raw(" "),
                Span::styled(label, Style::default().fg(dot_color)),
            ]));
        }
    }
    lines.push(Line::from(""));
}

pub(super) fn push_mcp_section<'a>(lines: &mut Vec<Line<'a>>, app: &App, width: u16, theme: Theme) {
    lines.push(section("MCP", theme));
    if app.engine.mcp_servers.is_empty() {
        for row in wrap_text_to_width("No MCP servers configured", width as usize) {
            lines.push(Line::from(vec![Span::styled(
                row,
                Style::default().fg(theme.text_muted),
            )]));
        }
    } else {
        for srv in &app.engine.mcp_servers {
            lines.push(Line::from(vec![
                Span::styled(
                    "● ",
                    Style::default().fg(mcp_status_color(srv.status, theme)),
                ),
                Span::styled(
                    truncate_str(&srv.name, width.saturating_sub(2) as usize),
                    Style::default().fg(theme.text_secondary),
                ),
            ]));
        }
    }
    lines.push(Line::from(""));
}

fn section(label: &'static str, theme: Theme) -> Line<'static> {
    Line::from(vec![Span::styled(
        label,
        Style::default()
            .fg(theme.text_primary)
            .add_modifier(Modifier::BOLD),
    )])
}

fn push_muted_rows<'a, I>(lines: &mut Vec<Line<'a>>, rows: I, width: u16, theme: Theme)
where
    I: IntoIterator<Item = String>,
{
    for row in rows {
        lines.push(Line::from(vec![Span::styled(
            format!("  {}", truncate_str(&row, width.saturating_sub(2) as usize)),
            Style::default().fg(theme.text_muted),
        )]));
    }
}
