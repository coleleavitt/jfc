use std::collections::{HashMap, HashSet};

use ratatui::widgets::{ListState, TableState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExpandedView {
    #[default]
    None,
    Tasks,
    Teammates,
}

#[derive(Debug, Default)]
pub struct SessionSidebarState {
    pub visible: bool,
    pub meta: Vec<jfc_session::SessionMetadata>,
    pub selected: usize,
    pub list: ListState,
}

impl SessionSidebarState {
    pub fn select(&mut self, selected: usize) {
        self.selected = selected;
        self.list.select(Some(selected));
    }

    pub fn reset_selection(&mut self) {
        self.select(0);
    }
}

#[derive(Debug)]
pub struct InfoSidebarState {
    pub visible: bool,
    pub scroll: u16,
    pub focused_widget: Option<FocusedUiWidget>,
    pub focused_panel: Option<FocusedUiPanel>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusedUiWidget {
    pub plugin_id: String,
    pub widget_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusedUiPanel {
    pub plugin_id: String,
    pub panel_id: String,
}

impl Default for InfoSidebarState {
    fn default() -> Self {
        Self {
            visible: true,
            scroll: 0,
            focused_widget: None,
            focused_panel: None,
        }
    }
}

impl InfoSidebarState {
    pub fn focus_widget(&mut self, plugin_id: impl Into<String>, widget_id: impl Into<String>) {
        self.focused_widget = Some(FocusedUiWidget {
            plugin_id: plugin_id.into(),
            widget_id: widget_id.into(),
        });
        self.focused_panel = None;
    }

    pub fn clear_widget_focus(&mut self) {
        self.focused_widget = None;
    }

    pub fn focus_panel(&mut self, plugin_id: impl Into<String>, panel_id: impl Into<String>) {
        self.focused_panel = Some(FocusedUiPanel {
            plugin_id: plugin_id.into(),
            panel_id: panel_id.into(),
        });
        self.focused_widget = None;
    }

    pub fn clear_panel_focus(&mut self) {
        self.focused_panel = None;
    }
}

#[derive(Debug)]
pub struct TaskPanelUiState {
    pub visible: bool,
    pub expanded_view: ExpandedView,
    pub selected: usize,
    pub table: TableState,
    pub detail: bool,
    pub viewing_task_id: Option<String>,
    pub viewing_expanded: HashMap<String, HashSet<usize>>,
}

impl Default for TaskPanelUiState {
    fn default() -> Self {
        Self {
            visible: false,
            expanded_view: ExpandedView::None,
            selected: 0,
            table: TableState::default().with_selected(Some(0)),
            detail: false,
            viewing_task_id: None,
            viewing_expanded: HashMap::new(),
        }
    }
}

impl TaskPanelUiState {
    pub fn select(&mut self, selected: usize) {
        self.selected = selected;
        self.table.select(Some(selected));
    }

    pub fn reset_selection(&mut self) {
        self.select(0);
        self.detail = false;
    }

    pub fn reset_drilldown(&mut self) {
        self.viewing_task_id = None;
        self.viewing_expanded.clear();
    }
}
