//! Fleet view — terminal dashboard showing all agents' statuses.
//!
//! Renders a ratatui-based grid of all active/idle/completed agents
//! with their turn status, tools used, and action needed.
//!
//! Used by: `jfc daemon status --live` or `/fleet` slash command.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Frame,
};

use crate::swarm::turn_classifier::{TurnStatus, TurnSummary};

/// One row in the fleet dashboard.
#[derive(Debug, Clone)]
pub struct FleetAgent {
    pub name: String,
    pub session_id: String,
    pub status: TurnStatus,
    pub detail: String,
    pub tools_this_turn: Vec<String>,
    pub total_tokens: usize,
    pub total_turns: usize,
    pub needs_action: Option<String>,
    pub model: Option<String>,
}

/// State for the fleet view (cursor, scroll, etc).
pub struct FleetViewState {
    pub agents: Vec<FleetAgent>,
    pub selected: usize,
    pub scroll_offset: usize,
}

impl FleetViewState {
    pub fn new() -> Self {
        Self {
            agents: Vec::new(),
            selected: 0,
            scroll_offset: 0,
        }
    }

    /// Update the agent list from daemon state.
    pub fn update_agents(&mut self, agents: Vec<FleetAgent>) {
        self.agents = agents;
        if self.selected >= self.agents.len() && !self.agents.is_empty() {
            self.selected = self.agents.len() - 1;
        }
    }

    /// Move selection up.
    pub fn select_prev(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    /// Move selection down.
    pub fn select_next(&mut self) {
        if self.selected + 1 < self.agents.len() {
            self.selected += 1;
        }
    }

    /// Get the currently selected agent.
    pub fn selected_agent(&self) -> Option<&FleetAgent> {
        self.agents.get(self.selected)
    }
}

impl Default for FleetViewState {
    fn default() -> Self {
        Self::new()
    }
}

/// Render the fleet dashboard into a ratatui frame.
pub fn render_fleet_view(f: &mut Frame, area: Rect, state: &FleetViewState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Header
            Constraint::Min(5),    // Agent table
            Constraint::Length(5), // Detail panel
        ])
        .split(area);

    // Header
    render_header(f, chunks[0], state);

    // Agent table
    render_agent_table(f, chunks[1], state);

    // Detail panel (selected agent info)
    render_detail_panel(f, chunks[2], state);
}

fn render_header(f: &mut Frame, area: Rect, state: &FleetViewState) {
    let running = state.agents.iter().filter(|a| a.status == TurnStatus::Running).count();
    let blocked = state.agents.iter().filter(|a| a.status == TurnStatus::Blocked).count();
    let idle = state.agents.iter().filter(|a| a.status == TurnStatus::Idle).count();
    let completed = state.agents.iter().filter(|a| a.status == TurnStatus::Completed).count();

    let header = Paragraph::new(Line::from(vec![
        Span::styled(" ⚡ Fleet ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw("│ "),
        Span::styled(format!("🔄 {running} running"), Style::default().fg(Color::Green)),
        Span::raw("  "),
        Span::styled(format!("🚫 {blocked} blocked"), Style::default().fg(Color::Red)),
        Span::raw("  "),
        Span::styled(format!("💤 {idle} idle"), Style::default().fg(Color::Yellow)),
        Span::raw("  "),
        Span::styled(format!("✅ {completed} done"), Style::default().fg(Color::DarkGray)),
        Span::raw(format!("  │ {} total", state.agents.len())),
    ]))
    .block(Block::default().borders(Borders::ALL).title(" jfc fleet "));

    f.render_widget(header, area);
}

fn render_agent_table(f: &mut Frame, area: Rect, state: &FleetViewState) {
    let header_cells = ["", "Agent", "Status", "Detail", "Tokens", "Turns", "Model"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)));
    let header = Row::new(header_cells).height(1);

    let rows: Vec<Row> = state
        .agents
        .iter()
        .enumerate()
        .map(|(i, agent)| {
            let style = if i == state.selected {
                Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let status_color = match agent.status {
                TurnStatus::Running => Color::Green,
                TurnStatus::Blocked => Color::Red,
                TurnStatus::Idle => Color::Yellow,
                TurnStatus::Completed => Color::DarkGray,
                TurnStatus::ReviewReady => Color::Magenta,
                TurnStatus::Error => Color::LightRed,
            };

            let cells = vec![
                Cell::from(agent.status.emoji()),
                Cell::from(agent.name.as_str()),
                Cell::from(agent.status.label()).style(Style::default().fg(status_color)),
                Cell::from(truncate_str(&agent.detail, 30)),
                Cell::from(format_tokens(agent.total_tokens)),
                Cell::from(agent.total_turns.to_string()),
                Cell::from(agent.model.as_deref().unwrap_or("-")),
            ];

            Row::new(cells).style(style)
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Length(15),
        Constraint::Length(12),
        Constraint::Min(20),
        Constraint::Length(8),
        Constraint::Length(6),
        Constraint::Length(20),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(" Agents "))
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    f.render_widget(table, area);
}

fn render_detail_panel(f: &mut Frame, area: Rect, state: &FleetViewState) {
    let content = if let Some(agent) = state.selected_agent() {
        let tools = if agent.tools_this_turn.is_empty() {
            "none".to_string()
        } else {
            agent.tools_this_turn.join(", ")
        };
        let action = agent.needs_action.clone().unwrap_or_else(|| "none".to_string());
        let session_line = format!("Session: {}  Tools: {}", agent.session_id, tools);
        let action_line = format!("Action needed: {}", action);
        vec![
            Line::from(Span::raw(session_line)),
            Line::from(Span::styled(action_line, Style::default().fg(Color::Yellow))),
            Line::from(Span::styled(
                " [Enter] attach  [s] stop  [m] mirror  [r] restart  [q] quit ",
                Style::default().fg(Color::DarkGray),
            )),
        ]
    } else {
        vec![Line::from(Span::styled(
            "No agents active. Use `jfc daemon run <task>` to start one.",
            Style::default().fg(Color::DarkGray),
        ))]
    };

    let panel = Paragraph::new(content)
        .block(Block::default().borders(Borders::ALL).title(" Details "));
    f.render_widget(panel, area);
}

// ─── Helpers ────────────────────────────────────────────────────────────────

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max - 1])
    }
}

fn format_tokens(tokens: usize) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}K", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fleet_view_state_navigation() {
        let mut state = FleetViewState::new();
        state.update_agents(vec![
            FleetAgent {
                name: "agent-1".to_string(),
                session_id: "s1".to_string(),
                status: TurnStatus::Running,
                detail: "Working on task".to_string(),
                tools_this_turn: vec!["bash".to_string()],
                total_tokens: 5000,
                total_turns: 3,
                needs_action: None,
                model: Some("claude-3-5-sonnet".to_string()),
            },
            FleetAgent {
                name: "agent-2".to_string(),
                session_id: "s2".to_string(),
                status: TurnStatus::Blocked,
                detail: "Waiting for permission".to_string(),
                tools_this_turn: vec!["edit".to_string()],
                total_tokens: 12000,
                total_turns: 7,
                needs_action: Some("Approve edit".to_string()),
                model: Some("claude-3-5-sonnet".to_string()),
            },
        ]);

        assert_eq!(state.selected, 0);
        state.select_next();
        assert_eq!(state.selected, 1);
        state.select_next();
        assert_eq!(state.selected, 1); // Can't go past end
        state.select_prev();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn format_tokens_display() {
        assert_eq!(format_tokens(500), "500");
        assert_eq!(format_tokens(5000), "5.0K");
        assert_eq!(format_tokens(1_500_000), "1.5M");
    }

    #[test]
    fn truncate_string() {
        assert_eq!(truncate_str("short", 10), "short");
        assert_eq!(truncate_str("this is a longer string", 10), "this is a…");
    }

    #[test]
    fn selected_agent() {
        let mut state = FleetViewState::new();
        assert!(state.selected_agent().is_none());
        state.update_agents(vec![FleetAgent {
            name: "test".to_string(),
            session_id: "s1".to_string(),
            status: TurnStatus::Idle,
            detail: "waiting".to_string(),
            tools_this_turn: vec![],
            total_tokens: 0,
            total_turns: 0,
            needs_action: None,
            model: None,
        }]);
        assert_eq!(state.selected_agent().unwrap().name, "test");
    }
}
