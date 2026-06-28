//! Panel view models.

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PanelViewModel {
    pub title: String,
    pub rows: Vec<String>,
}
