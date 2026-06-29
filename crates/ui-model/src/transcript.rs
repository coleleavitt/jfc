//! Transcript view models.

use crate::trace;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptViewModel {
    pub rows: Vec<TranscriptRow>,
}

impl Default for TranscriptViewModel {
    fn default() -> Self {
        trace::record_count("ui_model.transcript.default", 1);
        Self { rows: Vec::new() }
    }
}

impl TranscriptViewModel {
    pub fn push_row(&mut self, row: TranscriptRow) {
        let before = self.rows.len();
        self.rows.push(row);
        trace::record_collection_change(trace::CollectionChange {
            label: "ui_model.transcript.push_row",
            item_bytes_label: "row_bytes",
            item_bytes: self
                .rows
                .last()
                .map(|row| row.text.len())
                .unwrap_or_default(),
            before,
            after: self.rows.len(),
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptRow {
    pub text: String,
}

impl TranscriptRow {
    pub fn new(text: impl Into<String>) -> Self {
        let _linkscope_row = linkscope::phase("ui_model.transcript.row.new");
        let text = text.into();
        trace::record_text_shape("ui_model.transcript.row.new", "text_bytes", text.len());
        Self { text }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_trace_records_shape_without_text_payload_normal() {
        linkscope::trace_detail_enable();
        let mut transcript = TranscriptViewModel::default();
        transcript.push_row(TranscriptRow::new("private transcript row"));
        assert_eq!(transcript.rows.len(), 1);

        let snapshot = linkscope::snapshot();
        let rendered = format!("{snapshot:?}");
        assert!(rendered.contains("ui_model.transcript.row.new"));
        assert!(rendered.contains("ui_model.transcript.push_row"));
        assert!(rendered.contains("text_bytes"));
        assert!(!rendered.contains("private transcript row"));
    }
}
