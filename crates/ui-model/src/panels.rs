//! Panel view models.

use crate::trace;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelViewModel {
    pub title: String,
    pub rows: Vec<String>,
}

impl Default for PanelViewModel {
    fn default() -> Self {
        trace::record_count("ui_model.panel.default", 1);
        Self {
            title: String::new(),
            rows: Vec::new(),
        }
    }
}

impl PanelViewModel {
    pub fn new(title: impl Into<String>) -> Self {
        let _linkscope_panel = linkscope::phase("ui_model.panel.new");
        let title = title.into();
        trace::record_text_shape("ui_model.panel.new", "title_bytes", title.len());
        Self {
            title,
            rows: Vec::new(),
        }
    }

    pub fn push_row(&mut self, row: impl Into<String>) {
        let _linkscope_row = linkscope::phase("ui_model.panel.push_row");
        let row = row.into();
        let rows_before = self.rows.len();
        trace::record_collection_change(trace::CollectionChange {
            label: "ui_model.panel.push_row",
            item_bytes_label: "row_bytes",
            item_bytes: row.len(),
            before: rows_before,
            after: rows_before.saturating_add(1),
        });
        self.rows.push(row);
        trace::record_count("ui_model.panel.rows", self.rows.len());
    }

    pub fn row_count(&self) -> usize {
        trace::record_count("ui_model.panel.row_count", self.rows.len());
        self.rows.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_trace_records_shape_without_text_payload_normal() {
        linkscope::trace_detail_enable();
        let mut panel = PanelViewModel::new("private panel title");
        panel.push_row("private panel row");
        assert_eq!(panel.row_count(), 1);

        let snapshot = linkscope::snapshot();
        let rendered = format!("{snapshot:?}");
        assert!(rendered.contains("ui_model.panel.new"));
        assert!(rendered.contains("title_bytes"));
        assert!(rendered.contains("row_bytes"));
        assert!(!rendered.contains("private panel title"));
        assert!(!rendered.contains("private panel row"));
    }
}
