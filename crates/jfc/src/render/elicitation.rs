//! MCP elicitation modal — rendered when an MCP server requests user input.
//!
//! Two modes:
//! - **Form**: display a list of fields from the schema with their descriptions,
//!   show current input values, accept key input to fill them.
//! - **URL**: display the URL and a prompt; the user presses Enter to dismiss
//!   (the URL elicitation is completed by the server via a completion notification).
//!
//! The modal is a centered overlay matching the approval modal's style.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::app::App;
use crate::theme::Theme;
use jfc_core::mcp_elicitation::ElicitationKind;

/// Render the elicitation modal. Called from the frame compositor when
/// `app.engine.pending_elicitations` is non-empty.
pub(super) fn elicitation(f: &mut Frame, app: &App) {
    let Some(pending) = app.engine.pending_elicitations.front() else {
        return;
    };
    let t = app.theme;
    let area = f.area();

    let (width, height) = match &pending.kind {
        ElicitationKind::Form { schema, .. } => {
            // Height scales with field count
            let field_count = schema
                .get("properties")
                .and_then(|p| p.as_object())
                .map(|o| o.len())
                .unwrap_or(0);
            let h = (10 + field_count * 3) as u16;
            (
                (area.width * 8 / 10).clamp(60, 100),
                h.clamp(14, 30).min(area.height.saturating_sub(4)),
            )
        }
        ElicitationKind::Url { .. } => (
            (area.width * 7 / 10).clamp(60, 90),
            12u16.min(area.height.saturating_sub(4)),
        ),
    };

    let x = area.width.saturating_sub(width) / 2;
    let y = area.height.saturating_sub(height) / 2;
    let dialog_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, dialog_area);

    let queue_len = app.engine.pending_elicitations.len();
    let title = if queue_len > 1 {
        format!(
            " MCP Input Request · {} ({}  queued) ",
            pending.server_name,
            queue_len - 1
        )
    } else {
        format!(" MCP Input Request · {} ", pending.server_name)
    };

    let border_style = Style::default().fg(t.warning).add_modifier(Modifier::BOLD);
    let title_style = Style::default().fg(t.warning).add_modifier(Modifier::BOLD);

    let outer = Block::default()
        .title(Span::styled(title, title_style))
        .borders(Borders::ALL)
        .border_style(border_style);
    f.render_widget(outer.clone(), dialog_area);

    let inner = outer.inner(dialog_area);

    match &pending.kind {
        ElicitationKind::Form { message, schema } => {
            render_form_elicitation(f, inner, message, schema, &t, &app.elicitation_input);
        }
        ElicitationKind::Url { message, url, .. } => {
            render_url_elicitation(f, inner, message, url, &t);
        }
    }
}

fn render_form_elicitation(
    f: &mut Frame,
    area: Rect,
    message: &str,
    schema: &serde_json::Value,
    t: &Theme,
    current_inputs: &ElicitationInputState,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(area);

    f.render_widget(
        Paragraph::new(message)
            .style(Style::default().fg(t.text_primary))
            .wrap(Wrap { trim: true }),
        chunks[0],
    );

    let lines = build_form_field_lines(schema, current_inputs, t);
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), chunks[1]);
    f.render_widget(Paragraph::new(form_footer_line(t)), chunks[2]);
}

/// Build the field lines for a form elicitation (extracted for line-count).
fn build_form_field_lines<'a>(
    schema: &'a serde_json::Value,
    inputs: &'a ElicitationInputState,
    t: &'a Theme,
) -> Vec<Line<'a>> {
    let Some(props) = schema.get("properties").and_then(|p| p.as_object()) else {
        return vec![Line::from(Span::styled(
            "(no schema provided — press Enter to accept)",
            Style::default().fg(t.text_muted),
        ))];
    };
    let required: Vec<&str> = schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    let mut lines = Vec::new();
    for (name, prop) in props {
        lines.extend(build_field_lines(name, prop, &required, inputs, t));
    }
    lines
}

fn build_field_lines<'a>(
    name: &'a str,
    prop: &'a serde_json::Value,
    required: &[&str],
    inputs: &'a ElicitationInputState,
    t: &'a Theme,
) -> Vec<Line<'a>> {
    let is_required = required.contains(&name);
    let req_mark = if is_required { "*" } else { " " };
    let field_type = prop
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("string");
    let description = prop
        .get("description")
        .and_then(|d| d.as_str())
        .unwrap_or("");

    let label_style = if is_required {
        Style::default().fg(t.warning).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(t.text_primary)
    };

    let mut out = vec![Line::from(vec![
        Span::styled(format!("{req_mark} {name}"), label_style),
        Span::styled(
            format!(" ({field_type})"),
            Style::default().fg(t.text_muted),
        ),
    ])];

    if !description.is_empty() {
        out.push(Line::from(Span::styled(
            format!("  {description}"),
            Style::default().fg(t.text_muted),
        )));
    }

    let current_val = inputs.values.get(name).map(|s| s.as_str()).unwrap_or("");
    let is_active = inputs.active_field.as_deref() == Some(name);
    let val_style = if is_active {
        Style::default()
            .fg(t.accent)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    } else {
        Style::default().fg(t.text_primary)
    };
    let cursor = if is_active { "▏" } else { "" };
    let display = if current_val.is_empty() && !is_active {
        format!("(empty){cursor}")
    } else {
        format!("{current_val}{cursor}")
    };
    out.push(Line::from(vec![
        Span::styled("  > ", Style::default().fg(t.text_muted)),
        Span::styled(display, val_style),
    ]));
    out.push(Line::default());
    out
}

fn form_footer_line(t: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            " Tab ",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled("next  ", Style::default().fg(t.text_muted)),
        Span::styled(
            " Enter ",
            Style::default().fg(t.success).add_modifier(Modifier::BOLD),
        ),
        Span::styled("accept  ", Style::default().fg(t.text_muted)),
        Span::styled(
            " Esc ",
            Style::default().fg(t.error).add_modifier(Modifier::BOLD),
        ),
        Span::styled("decline  ", Style::default().fg(t.text_muted)),
        Span::styled(
            " q ",
            Style::default().fg(t.error).add_modifier(Modifier::BOLD),
        ),
        Span::styled("cancel", Style::default().fg(t.text_muted)),
    ])
}

fn render_url_elicitation(f: &mut Frame, area: Rect, message: &str, url: &str, t: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // message
            Constraint::Length(3), // url
            Constraint::Min(1),    // instruction
            Constraint::Length(2), // footer
        ])
        .split(area);

    let msg_para = Paragraph::new(message)
        .style(Style::default().fg(t.text_primary))
        .wrap(Wrap { trim: true });
    f.render_widget(msg_para, chunks[0]);

    let url_para = Paragraph::new(url)
        .style(
            Style::default()
                .fg(t.accent)
                .add_modifier(Modifier::UNDERLINED),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(url_para, chunks[1]);

    let instruction = Paragraph::new(
        "Visit the URL above to complete this request.\n\
         The dialog will close automatically when the server confirms completion.",
    )
    .style(Style::default().fg(t.text_muted))
    .wrap(Wrap { trim: true });
    f.render_widget(instruction, chunks[2]);

    let footer = Line::from(vec![
        Span::styled(
            " Enter ",
            Style::default().fg(t.success).add_modifier(Modifier::BOLD),
        ),
        Span::styled("dismiss  ", Style::default().fg(t.text_muted)),
        Span::styled(
            " Esc ",
            Style::default().fg(t.error).add_modifier(Modifier::BOLD),
        ),
        Span::styled("cancel", Style::default().fg(t.text_muted)),
    ]);
    f.render_widget(Paragraph::new(footer), chunks[3]);
}

/// A snapshot of user input state for the current elicitation form.
/// Lives on `App` and is reset when a new elicitation arrives.
#[derive(Default, Debug, Clone)]
pub struct ElicitationInputState {
    /// Current text values keyed by field name.
    pub values: std::collections::HashMap<String, String>,
    /// Which field has keyboard focus (Tab cycles through).
    pub active_field: Option<String>,
    /// Ordered list of field names (for Tab cycling).
    pub field_order: Vec<String>,
}

impl ElicitationInputState {
    /// Initialize from a form schema's property names.
    pub fn from_schema(schema: &serde_json::Value) -> Self {
        let field_order: Vec<String> = schema
            .get("properties")
            .and_then(|p| p.as_object())
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default();
        let active_field = field_order.first().cloned();
        Self {
            values: std::collections::HashMap::new(),
            active_field,
            field_order,
        }
    }

    /// Move focus to the next field (Tab).
    pub fn next_field(&mut self) {
        if self.field_order.is_empty() {
            return;
        }
        let current_idx = self
            .active_field
            .as_deref()
            .and_then(|name| self.field_order.iter().position(|f| f == name))
            .unwrap_or(0);
        let next_idx = (current_idx + 1) % self.field_order.len();
        self.active_field = self.field_order.get(next_idx).cloned();
    }

    /// Type a character into the active field.
    pub fn type_char(&mut self, c: char) {
        if let Some(ref name) = self.active_field.clone() {
            self.values.entry(name.clone()).or_default().push(c);
        }
    }

    /// Delete last char in the active field (Backspace).
    pub fn backspace(&mut self) {
        if let Some(ref name) = self.active_field.clone() {
            if let Some(val) = self.values.get_mut(name) {
                val.pop();
            }
        }
    }

    /// Collect current values as a serde_json::Value object.
    pub fn to_json(&self) -> serde_json::Value {
        let map: serde_json::Map<String, serde_json::Value> = self
            .values
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect();
        serde_json::Value::Object(map)
    }
}
