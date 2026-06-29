//! Widget view models.

use crate::trace;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WidgetViewModel {
    pub label: String,
    pub rows: Vec<String>,
}

impl Default for WidgetViewModel {
    fn default() -> Self {
        trace::record_count("ui_model.widget.default", 1);
        Self {
            label: String::new(),
            rows: Vec::new(),
        }
    }
}

impl WidgetViewModel {
    pub fn new(label: impl Into<String>) -> Self {
        let _linkscope_widget = linkscope::phase("ui_model.widget.new");
        let label = label.into();
        trace::record_text_shape("ui_model.widget.new", "label_bytes", label.len());
        Self {
            label,
            rows: Vec::new(),
        }
    }

    pub fn push_row(&mut self, row: impl Into<String>) {
        let _linkscope_row = linkscope::phase("ui_model.widget.push_row");
        let row = row.into();
        let rows_before = self.rows.len();
        trace::record_collection_change(trace::CollectionChange {
            label: "ui_model.widget.push_row",
            item_bytes_label: "row_bytes",
            item_bytes: row.len(),
            before: rows_before,
            after: rows_before.saturating_add(1),
        });
        self.rows.push(row);
        trace::record_count("ui_model.widget.rows", self.rows.len());
    }

    pub fn row_count(&self) -> usize {
        trace::record_count("ui_model.widget.row_count", self.rows.len());
        self.rows.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widget_trace_records_shape_without_text_payload_normal() {
        linkscope::trace_detail_enable();
        let mut widget = WidgetViewModel::new("private widget label");
        widget.push_row("private widget row");
        assert_eq!(widget.row_count(), 1);

        let snapshot = linkscope::snapshot();
        let rendered = format!("{snapshot:?}");
        assert!(rendered.contains("ui_model.widget.new"));
        assert!(rendered.contains("label_bytes"));
        assert!(rendered.contains("row_bytes"));
        assert!(!rendered.contains("private widget label"));
        assert!(!rendered.contains("private widget row"));
    }
}
