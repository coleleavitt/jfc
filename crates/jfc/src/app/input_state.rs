use ratatui::widgets::TableState;

use crate::query::QueryCache;
use crate::theme::Theme;

#[derive(Debug, Clone, Default)]
pub struct CommandPaletteState {
    pub visible: bool,
    pub input: String,
    pub selected: usize,
}

impl CommandPaletteState {
    pub fn open(&mut self) {
        self.visible = true;
        self.reset_query();
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.reset_query();
    }

    pub fn reset_query(&mut self) {
        self.input.clear();
        self.reset_selection();
    }

    pub fn reset_selection(&mut self) {
        self.selected = 0;
    }
}

#[derive(Default)]
pub struct ThemePickerState {
    pub visible: bool,
    pub input: String,
    pub selected: usize,
    pub preview_original: Option<Theme>,
    pub preview_original_name: Option<String>,
}

impl ThemePickerState {
    pub fn open(&mut self, current_theme: Theme, active_theme_name: &str, selected: usize) {
        self.preview_original = Some(current_theme);
        self.preview_original_name = Some(active_theme_name.to_owned());
        self.input.clear();
        self.selected = selected;
        self.visible = true;
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.input.clear();
        self.selected = 0;
        self.preview_original = None;
        self.preview_original_name = None;
    }

    pub fn reset_selection(&mut self) {
        self.selected = 0;
    }
}

pub struct ModelPickerState {
    pub visible: bool,
    pub filter: String,
    pub selected: usize,
    pub models: Vec<jfc_provider::ModelInfo>,
    pub table: TableState,
    pub query_cache: QueryCache<Vec<jfc_provider::ModelInfo>>,
}

impl Default for ModelPickerState {
    fn default() -> Self {
        Self {
            visible: false,
            filter: String::new(),
            selected: 0,
            models: Vec::new(),
            table: TableState::default().with_selected(Some(0)),
            query_cache: QueryCache::default(),
        }
    }
}

impl ModelPickerState {
    pub fn open(&mut self, models: Vec<jfc_provider::ModelInfo>) {
        self.visible = true;
        self.filter.clear();
        self.models = models;
        self.select(0);
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.filter.clear();
        self.select(0);
    }

    pub fn select(&mut self, selected: usize) {
        self.selected = selected;
        self.table.select(Some(selected));
    }

    pub fn reset_selection(&mut self) {
        self.select(0);
    }
}

pub struct SessionPickerState {
    pub visible: bool,
    pub filter: String,
    pub table: TableState,
}

impl Default for SessionPickerState {
    fn default() -> Self {
        Self {
            visible: false,
            filter: String::new(),
            table: TableState::default().with_selected(Some(0)),
        }
    }
}

impl SessionPickerState {
    pub fn open(&mut self) {
        self.visible = true;
        self.filter.clear();
        self.table.select(Some(0));
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.filter.clear();
        self.table.select(Some(0));
    }
}

pub struct BashPickerState {
    pub visible: bool,
    pub table: TableState,
    pub tasks: Vec<jfc_engine::tools::BashTaskSnapshot>,
}

impl Default for BashPickerState {
    fn default() -> Self {
        Self {
            visible: false,
            table: TableState::default().with_selected(Some(0)),
            tasks: Vec::new(),
        }
    }
}

impl BashPickerState {
    pub fn open_with_tasks(&mut self, tasks: Vec<jfc_engine::tools::BashTaskSnapshot>) {
        let selected = if tasks.is_empty() { None } else { Some(0) };
        self.tasks = tasks;
        self.visible = true;
        self.table.select(selected);
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.table.select(Some(0));
    }
}
