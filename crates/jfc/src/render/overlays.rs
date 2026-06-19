use super::*;
use crate::input::slash_commands_table;

pub(super) fn toast_overlay(f: &mut Frame, app: &App) {
    use jfc_engine::toast::ToastKind;
    let t = app.theme;
    let visible_toasts = app
        .engine
        .toasts
        .iter()
        .filter(|toast| matches!(toast.kind, ToastKind::Error))
        .collect::<Vec<_>>();
    if visible_toasts.is_empty() {
        *app.toasts_rect.borrow_mut() = None;
        return;
    }
    let frame_area = f.area();
    if frame_area.width < 30 || frame_area.height < 4 {
        return;
    }
    const MAX_W: u16 = 60;
    // Reserve room for a 1-cell border on each side so the strip
    // reads as a contained unit rather than text bleeding into the
    // transcript below it.
    let w = MAX_W.min(frame_area.width.saturating_sub(2));
    let count = visible_toasts.len() as u16;
    let body_h = count.min(5); // MAX_TOASTS, but bound to layout
    if body_h == 0 {
        *app.toasts_rect.borrow_mut() = None;
        return;
    }
    let h = body_h + 2; // borders top/bottom
    // Slide-in: the strip enters from off-screen-right and eases in
    // over the freshest toast's first 200ms. Ease-out cubic so it
    // settles softly rather than overshooting. Reduced-motion skips
    // the slide and the strip pops in at its final position.
    let slide_offset: u16 = if crate::spinner::reduced_motion() {
        0
    } else {
        let freshest_age = visible_toasts
            .iter()
            .map(|tt| tt.created_at.elapsed())
            .min()
            .unwrap_or_default();
        let progress = (freshest_age.as_millis() as f32 / 200.0).min(1.0);
        // Ease-out cubic: 1 - (1 - t)^3
        let eased = 1.0 - (1.0 - progress).powi(3);
        // Off-screen distance is the strip width — at progress=0 the
        // strip is fully off the right edge; at progress=1 it sits
        // flush with its target.
        ((1.0 - eased) * (w as f32 + 4.0)).round() as u16
    };
    let target_x = frame_area.x + frame_area.width.saturating_sub(w + 1);
    let frame_right = frame_area.x + frame_area.width;
    // Resting x of the strip + the slide offset. Capped to the
    // frame's right edge so we never go past the buffer.
    let actual_x = target_x
        .saturating_add(slide_offset)
        .min(frame_area.x + frame_area.width.saturating_sub(1));
    // Width *must* be derived from `actual_x` so `actual_x + width`
    // never exceeds `frame_right`. Earlier this was computed
    // independently (`w.saturating_sub(slide_offset)`), which clamped
    // the x within bounds but left a width that walked off the right
    // edge — a 60-cell-wide strip starting at column 207 of a
    // 208-cell-wide buffer panicked the ratatui Clear widget at
    // `index_of((208, 1))`. The bug surfaced on slide-in's first
    // frame (offset=full strip width).
    let actual_w = w.min(frame_right.saturating_sub(actual_x));
    let area = Rect {
        x: actual_x,
        y: frame_area.y + 1,
        width: actual_w,
        height: h.min(frame_area.height.saturating_sub(2)),
    };
    if area.width == 0 || area.height == 0 {
        return;
    }
    *app.toasts_rect.borrow_mut() = Some(area);
    f.render_widget(Clear, area);
    let border_color = t.error;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let mut lines: Vec<Line> = Vec::new();
    for toast in visible_toasts.into_iter().rev().take(inner.height as usize) {
        let (icon, color) = ("✘", t.error);
        let max_text = (inner.width as usize).saturating_sub(4);
        let text: String = if toast.text.chars().count() > max_text {
            let mut out: String = toast
                .text
                .chars()
                .take(max_text.saturating_sub(1))
                .collect();
            out.push('…');
            out
        } else {
            toast.text.clone()
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {icon} "), Style::default().fg(color)),
            Span::styled(text, Style::default().fg(t.text_primary)),
        ]));
    }
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(t.surface)),
        inner,
    );
}

/// One-line diagnostic summary row. v126 cli.js:338035-338038 renders this
/// as `Found <bold>N</bold> new diagnostic <issue/issues> in M <file/files>
/// (ctrl+o to expand)` in dim color. Shown above the spinner row when
/// `app.engine.diagnostics` has any entries; the formatter and dedup-by-file
/// logic live in `diagnostics.rs`.
pub(super) fn diagnostic_row(f: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 {
        return;
    }
    let t = app.theme;
    // Count only the *new* diagnostics — entries the user has already
    // acknowledged via Ctrl+O don't show up in the row count. v126
    // cli.js:231036 surfaces the same delta-only count: `Found N new
    // diagnostic issue(s)` — the word "new" is load-bearing.
    let new_entries: Vec<&jfc_engine::diagnostics::DiagnosticEntry> =
        jfc_engine::diagnostics::unacknowledged(
            &app.engine.diagnostics,
            &app.delivered_diagnostics,
        );
    let issues = new_entries.len();
    let files = {
        let mut s: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for e in &new_entries {
            s.insert(e.file.as_str());
        }
        s.len()
    };
    let Some(text) = jfc_engine::diagnostics::format_summary(issues, files) else {
        return;
    };
    let has_errors = new_entries
        .iter()
        .any(|e| matches!(e.severity, jfc_engine::diagnostics::Severity::Error));
    let icon_color = if has_errors { t.error } else { t.warning };
    let line = Line::from(vec![
        Span::styled("● ", Style::default().fg(icon_color)),
        Span::styled(text, Style::default().fg(t.text_muted)),
    ]);
    f.render_widget(Paragraph::new(line).style(Style::default().bg(t.bg)), area);
}

// Modal diagnostic-expansion panel (`Ctrl+O` from the summary row,
// `Esc` to close). Mirrors v126 cli.js:338043-338053:
//
// ```text
//   <relative path bold>  (file://)
//     ✘ [Line 12:5] unresolved import [E0432] (cargo)
//     ⚠ [Line 1:1]  unused variable
//   ...
// ```
//
// Diagnostics are grouped by file (first occurrence preserves cargo's
// emission order) and listed underneath. We don't render the URI scheme
// suffix v126 does (`(file://)`) — paths are already cwd-relative so
// it's noise.
//
// The autocomplete popup's command list is NOT defined here — it is the
// macro-generated `SLASH_COMMANDS` table from the `slash_commands!` registry
// (the single source of truth for dispatch + metadata), imported at the top
// of this module so the popup and `/help` can never drift from the handlers.

/// Returns the `/<prefix>` the user is currently typing, when the
/// input bar contains a single line that starts with `/`. The popup
/// renders only when this returns Some so multi-line drafts and
/// non-slash input don't trigger.
pub(crate) fn current_slash_prefix(app: &App) -> Option<String> {
    let lines = app.textarea.lines();
    if lines.len() != 1 {
        return None;
    }
    let line = &lines[0];
    if !line.starts_with('/') {
        return None;
    }
    // Single-token: drop everything after the first space so the
    // popup goes away once the user has committed to a verb and
    // is typing arguments. v126's slash UI does the same.
    let token = line.split_whitespace().next().unwrap_or(line);
    Some(token.to_string())
}

pub(crate) fn slash_matches(prefix: &str) -> Vec<&'static (&'static str, &'static str)> {
    slash_commands_table()
        .iter()
        .filter(|(cmd, _)| cmd.starts_with(prefix))
        .collect()
}

pub(super) fn slash_popup(f: &mut Frame, app: &App, prefix: &str) {
    let matches = slash_matches(prefix);
    if matches.is_empty() {
        return;
    }
    let t = app.theme;
    let area = f.area();
    let h = (matches.len() as u16).min(8) + 2;
    let w: u16 = 60u16.min(area.width.saturating_sub(2));
    if area.height < h + 4 {
        return;
    }
    // Anchor above the input bar (which sits at the bottom of the
    // frame). Reserve 2 rows for the input border and 1 for the
    // status bar so the popup doesn't overlap them.
    let popup_y = area.y + area.height.saturating_sub(h + 4);
    let popup = Rect {
        x: area.x + 2,
        y: popup_y,
        width: w,
        height: h,
    };
    f.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(t.style_accent)
        .title(Span::styled(" slash commands ", t.style_accent_bold))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let selected = app.slash_popup_selected.unwrap_or(0).min(matches.len() - 1);
    let lines: Vec<Line> = matches
        .iter()
        .enumerate()
        .take(inner.height as usize)
        .map(|(i, (cmd, desc))| {
            let active = i == selected;
            let row_style = if active {
                Style::default()
                    .fg(t.bg)
                    .bg(t.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                t.style_text_primary
            };
            let desc_style = if active {
                Style::default().fg(t.bg).bg(t.accent)
            } else {
                t.style_text_muted
            };
            Line::from(vec![
                Span::styled(format!(" {:<18}", cmd), row_style),
                Span::styled(format!(" {desc}"), desc_style),
            ])
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(t.surface)),
        inner,
    );
}

/// Bottom-anchored search bar shown while `app.transcript_search`
/// is `Some`. Mirrors Vim's `/` prompt: query on the left, match
/// counter on the right ("3 of 12"). The currently-focused match
/// is already scrolled into view by the input handler — this is
/// purely the input-bar UI for typing the query and stepping
/// through matches.
pub(super) fn search_bar(f: &mut Frame, app: &App) {
    let Some(s) = &app.transcript_search else {
        return;
    };
    let t = app.theme;
    let area = f.area();
    if area.width < 20 || area.height < 3 {
        return;
    }
    let h: u16 = 2;
    let bar = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(h + 2),
        width: area.width,
        height: h,
    };
    f.render_widget(Clear, bar);

    let count_label = if s.query.is_empty() {
        " (type to search) ".to_string()
    } else if s.matches.is_empty() {
        " (no matches) ".to_string()
    } else {
        format!(" {} of {} ", s.cursor + 1, s.matches.len())
    };
    let prompt = Line::from(vec![
        Span::styled(
            "  /",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            s.query.clone(),
            Style::default()
                .fg(t.text_primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("▏", Style::default().fg(t.accent)),
    ]);
    let hint = Line::from(vec![
        Span::styled(
            count_label,
            Style::default()
                .fg(if s.matches.is_empty() && !s.query.is_empty() {
                    t.warning
                } else {
                    t.text_secondary
                })
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " ↑/↓ navigate · Enter jump · Esc close",
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        ),
    ]);
    f.render_widget(
        Paragraph::new(vec![prompt, hint]).style(Style::default().bg(t.surface)),
        bar,
    );
}

/// Ctrl+R reverse-history search overlay: a query header plus a windowed
/// list of matching past prompts, the selected one ribboned in accent.
pub(super) fn prompt_search_overlay(f: &mut Frame, app: &App) {
    let Some(s) = &app.prompt_search else {
        return;
    };
    let t = app.theme;
    let area = f.area();
    if area.width < 24 || area.height < 6 {
        return;
    }
    const MAX_ROWS: usize = 6;
    let shown = s.results.len().min(MAX_ROWS);
    let h = shown as u16 + 1; // +1 header row
    let overlay = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(h + 3),
        width: area.width,
        height: h,
    };
    f.render_widget(Clear, overlay);

    let header = if s.results.is_empty() {
        format!("  ⌕ history  {}▏  (no matches · Esc)", s.query)
    } else {
        format!(
            "  ⌕ history  {}▏  ({}/{} · ↑↓ · Enter loads · Esc)",
            s.query,
            s.selected + 1,
            s.results.len()
        )
    };
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        header,
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
    ))];

    // Window the results so the selected row stays visible.
    let start = if s.selected < MAX_ROWS {
        0
    } else {
        s.selected - (MAX_ROWS - 1)
    };
    let body_w = (overlay.width as usize).saturating_sub(4);
    for (ri, &idx) in s.results.iter().enumerate().skip(start).take(shown) {
        let is_sel = ri == s.selected;
        let text = s.all.get(idx).map(String::as_str).unwrap_or("");
        // Flatten newlines so a multi-line prompt reads on one row.
        let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let trimmed = super::truncate_str(&flat, body_w);
        let (prefix, style) = if is_sel {
            (
                "▌ ",
                Style::default()
                    .fg(t.text_primary)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            ("  ", Style::default().fg(t.text_muted))
        };
        lines.push(Line::from(vec![
            Span::styled(prefix, Style::default().fg(t.accent)),
            Span::styled(trimmed, style),
        ]));
    }
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(t.surface)),
        overlay,
    );
}

/// Centered keybinding overlay toggled by `?`. Groups bindings by
/// context — Input bar, Transcript, Task view, Picker/Palette, Leader
/// chord, ESC behavior — so the user can find the chord they need
/// without grepping the source.
pub(super) fn help_overlay(f: &mut Frame, app: &App) {
    let t = app.theme;
    let area = f.area();
    let width = (area.width * 7 / 10).clamp(60, 96);
    let height = (area.height * 8 / 10).clamp(20, 32);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    let popup = Rect::new(x, y, width, height);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(t.style_accent)
        .title(Span::styled(
            " keybindings · press ? or Esc to close ",
            t.style_accent_bold,
        ))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    // Each entry: (key, description). Sections separated by None.
    type Section = (&'static str, &'static [(&'static str, &'static str)]);
    const SECTIONS: &[Section] = &[
        (
            "Input bar",
            &[
                ("Enter", "send message"),
                ("Shift+Enter", "newline"),
                ("Up", "recall queued prompt (if input empty)"),
                ("Ctrl+V", "paste (text or image from clipboard)"),
                ("Ctrl+Z", "undo last edit"),
                ("Ctrl+Shift+Z", "redo"),
                ("Ctrl+F", "search inside the textarea"),
                ("Ctrl+R", "retry last prompt"),
                ("Ctrl+E", "edit + resubmit previous user message"),
                ("Alt+Y", "yank current input to clipboard"),
                ("Ctrl+L", "yank file:line ref to clipboard"),
                ("Alt+./Alt+,", "raise / lower reasoning effort"),
                ("/export", "save transcript as markdown"),
                ("/theme", "switch theme"),
                ("/dump-context", "show what the model sees"),
            ],
        ),
        (
            "Transcript",
            &[
                ("Ctrl+P", "command palette"),
                ("Ctrl+M", "switch model"),
                ("Alt+./Alt+,", "raise / lower reasoning effort"),
                ("Ctrl+B", "toggle sessions sidebar"),
                ("Ctrl+S/I", "toggle info sidebar"),
                ("Ctrl+T", "show task panel"),
                ("Ctrl+O", "expand diagnostic row / large tool block"),
                ("o", "toggle expand on most recent collapsible block"),
                ("Ctrl+Y", "yank last assistant response"),
                ("Ctrl+L", "yank file:line ref from recent output"),
                ("j / k", "vim scroll down / up (empty input)"),
                ("g / G", "vim jump to top / bottom (empty input)"),
                ("Shift+Tab", "cycle permission modes"),
                ("PgUp/PgDn", "scroll a page"),
                ("Ctrl+Home/End", "jump to top/bottom"),
            ],
        ),
        (
            "Task view",
            &[
                ("Ctrl+X then ↓", "enter task view"),
                ("←/→", "previous / next running task"),
                ("↓", "jump to most recent task"),
                ("↑ or Esc", "exit task view"),
                ("o", "expand the most recent collapsible message"),
            ],
        ),
        (
            "Picker / Palette",
            &[
                ("↑/↓ or k/j", "navigate"),
                ("Home/End", "first / last"),
                ("PgUp/PgDn", "page"),
                ("Enter", "select"),
                ("Esc", "cancel"),
                ("type", "filter inline"),
            ],
        ),
        (
            "Interrupt & approvals",
            &[
                ("Esc Esc", "interrupt streaming / agentic loop"),
                ("y / n / a", "approve / deny / always (in approval modal)"),
                ("/swarm-approve <id>", "approve teammate permission request"),
                (
                    "/swarm-deny <id> [reason]",
                    "deny teammate permission request",
                ),
            ],
        ),
    ];

    let mut lines: Vec<Line<'static>> = Vec::new();
    for (section_name, entries) in SECTIONS.iter() {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(
            (*section_name).to_string(),
            t.style_accent
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )));
        for (key, desc) in entries.iter() {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("{key:<24}"),
                    Style::default()
                        .fg(t.text_primary)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled((*desc).to_string(), Style::default().fg(t.text_secondary)),
            ]));
        }
    }

    // ── Custom keybindings from ~/.config/jfc/keybindings.toml ───────────
    let custom = crate::keybindings::all_bindings();
    if !custom.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Custom bindings".to_string(),
            t.style_accent
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )));
        for (key_str, desc) in &custom {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("{key_str:<24}"),
                    Style::default()
                        .fg(t.text_primary)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(desc.clone(), Style::default().fg(t.text_secondary)),
            ]));
        }
    } else {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Custom bindings".to_string(),
            t.style_accent
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "none — create ~/.config/jfc/keybindings.toml to add your own",
                Style::default().fg(t.text_muted),
            ),
        ]));
    }

    let para = Paragraph::new(lines)
        .style(Style::default().bg(t.surface))
        .scroll((0, 0));
    f.render_widget(para, inner);
}

pub(super) fn diagnostic_panel(f: &mut Frame, app: &App) {
    let t = app.theme;
    let area = f.area();
    let w = area.width.saturating_mul(3) / 4;
    let h = area.height.saturating_mul(3) / 4;
    let x = (area.width.saturating_sub(w)) / 2;
    let y = (area.height.saturating_sub(h)) / 2;
    let rect = Rect {
        x: area.x + x,
        y: area.y + y,
        width: w,
        height: h,
    };
    f.render_widget(Clear, rect);
    let issues = app.engine.diagnostics.len();
    let files = jfc_engine::diagnostics::count_files(&app.engine.diagnostics);

    // Group entries by file in first-seen order. Avoid HashMap iteration
    // for ordering stability — use a Vec of (file, Vec<&entry>).
    let mut groups: Vec<(String, Vec<&jfc_engine::diagnostics::DiagnosticEntry>)> = Vec::new();
    for entry in &app.engine.diagnostics {
        if let Some(g) = groups.iter_mut().find(|(f, _)| f == &entry.file) {
            g.1.push(entry);
        } else {
            groups.push((entry.file.clone(), vec![entry]));
        }
    }

    let mut lines: Vec<Line> = Vec::new();
    for (file, items) in &groups {
        lines.push(Line::from(Span::styled(
            file.clone(),
            t.style_text_primary.add_modifier(Modifier::BOLD),
        )));
        for entry in items {
            let body = jfc_engine::diagnostics::format_entry(entry);
            // Two-cell extra indent so file headers visually anchor.
            let color = match entry.severity {
                jfc_engine::diagnostics::Severity::Error => t.error,
                jfc_engine::diagnostics::Severity::Warning => t.warning,
                jfc_engine::diagnostics::Severity::Info => t.text_secondary,
                jfc_engine::diagnostics::Severity::Hint => t.text_muted,
            };
            lines.push(Line::from(Span::styled(body, Style::default().fg(color))));
        }
        lines.push(Line::from(""));
    }

    // Title now embeds a scroll position when the body overflows the
    // panel — the user can see at a glance how much more there is to
    // scroll through, and the key hints are visible on the title bar
    // instead of being hidden in the help overlay.
    let total_lines = lines.len();
    let inner_h = rect.height.saturating_sub(2) as usize; // borders
    let scroll = app
        .diagnostic_panel_scroll
        .min(total_lines.saturating_sub(inner_h.max(1)));
    let scroll_pos = if total_lines > inner_h && inner_h > 0 {
        format!(
            " · {}/{}",
            scroll + 1,
            total_lines.saturating_sub(inner_h.max(1)) + 1
        )
    } else {
        String::new()
    };
    let title = format!(
        " Diagnostics — {issues} {} in {files} {}{scroll_pos} · ↑↓/PgUp/PgDn scroll · Esc close ",
        if issues == 1 { "issue" } else { "issues" },
        if files == 1 { "file" } else { "files" },
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(t.style_error)
        .title(Span::styled(
            title,
            t.style_error.add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    f.render_widget(
        Paragraph::new(lines)
            .style(Style::default().bg(t.surface))
            .wrap(ratatui::widgets::Wrap { trim: false })
            .scroll((scroll as u16, 0)),
        inner,
    );
}

/// Floating completion list anchored just above the input bar.
/// Renders up to 8 candidates from `app.mention.candidates`, with the
/// current `selected` index highlighted. Mirrors v126 cli.js:161602
/// (`autocomplete:accept` / `autocomplete:dismiss`) — non-modal,
/// non-blocking, dismissed by Esc or by typing past the `@token`.
pub(super) fn mention_popup(f: &mut Frame, app: &App, input_area: Rect) {
    let t = app.theme;
    let frame_area = f.area();
    let candidates = &app.mention.candidates;
    if candidates.is_empty() || frame_area.height < 6 {
        return;
    }
    const MAX_ROWS: u16 = 8;
    let visible: u16 = candidates.len().min(MAX_ROWS as usize) as u16;
    let h = visible + 2; // borders
    let w = 60u16.min(frame_area.width.saturating_sub(2));
    // Prefer placing the popup directly above the input. Fall back to
    // below when there isn't enough room above (small terminals).
    let above_top = input_area.y.saturating_sub(h);
    let area = if above_top >= frame_area.y && input_area.y >= h {
        Rect {
            x: input_area.x.min(frame_area.width.saturating_sub(w)),
            y: above_top,
            width: w,
            height: h,
        }
    } else {
        Rect {
            x: input_area.x.min(frame_area.width.saturating_sub(w)),
            y: input_area.y + input_area.height,
            width: w,
            height: h.min(
                frame_area
                    .height
                    .saturating_sub(input_area.y + input_area.height),
            ),
        }
    };
    f.render_widget(Clear, area);
    let title = format!(
        " @ {} ({} match{}) ",
        if app.mention.query.is_empty() {
            "<type to filter>".into()
        } else {
            app.mention.query.clone()
        },
        candidates.len(),
        if candidates.len() == 1 { "" } else { "es" }
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(t.style_accent)
        .title(Span::styled(title, t.style_accent))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let items: Vec<ListItem> = candidates
        .iter()
        .take(MAX_ROWS as usize)
        .enumerate()
        .map(|(i, path)| {
            let is_sel = i == app.mention.selected;
            let style = if is_sel {
                t.style_text_primary.bg(t.accent)
            } else {
                t.style_text_secondary
            };
            let prefix = if is_sel { "▸ " } else { "  " };
            let max_w = inner.width.saturating_sub(prefix.len() as u16) as usize;
            let truncated: String = if path.chars().count() > max_w && max_w > 1 {
                let mut s: String = path.chars().take(max_w.saturating_sub(1)).collect();
                s.push('…');
                s
            } else {
                path.clone()
            };
            ListItem::new(Line::from(vec![
                Span::styled(prefix.to_string(), style),
                Span::styled(truncated, style),
            ]))
        })
        .collect();
    f.render_widget(List::new(items), inner);
}
