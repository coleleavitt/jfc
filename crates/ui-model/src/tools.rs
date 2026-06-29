//! Tool view models.

use crate::trace;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolViewModel {
    pub title: String,
    pub status: ToolViewStatus,
}

fn status_label(status: ToolViewStatus) -> &'static str {
    match status {
        ToolViewStatus::Pending => "pending",
        ToolViewStatus::Running => "running",
        ToolViewStatus::Completed => "completed",
        ToolViewStatus::Failed => "failed",
    }
}

impl ToolViewModel {
    pub fn new(title: impl Into<String>, status: ToolViewStatus) -> Self {
        let _linkscope_tool = linkscope::phase("ui_model.tool.new");
        let title = title.into();
        trace::record_named_shape(trace::NamedShape {
            label: "ui_model.tool.new",
            kind_label: "status",
            kind: status_label(status),
            text_label: "title_bytes",
            text_bytes: title.len(),
        });
        Self { title, status }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolViewStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_trace_records_shape_without_title_payload_normal() {
        linkscope::trace_detail_enable();
        let tool = ToolViewModel::new("private tool title", ToolViewStatus::Running);
        assert_eq!(tool.status, ToolViewStatus::Running);

        let snapshot = linkscope::snapshot();
        let rendered = format!("{snapshot:?}");
        assert!(rendered.contains("ui_model.tool.new"));
        assert!(rendered.contains("running"));
        assert!(rendered.contains("title_bytes"));
        assert!(!rendered.contains("private tool title"));
    }
}
