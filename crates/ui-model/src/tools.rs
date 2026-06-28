//! Tool view models.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolViewModel {
    pub title: String,
    pub status: ToolViewStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolViewStatus {
    Pending,
    Running,
    Completed,
    Failed,
}
