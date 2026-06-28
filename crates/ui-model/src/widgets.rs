//! Widget view models.

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WidgetViewModel {
    pub label: String,
    pub rows: Vec<String>,
}
