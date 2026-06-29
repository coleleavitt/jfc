//! Status-row view models.

use crate::trace;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusTone {
    Muted,
    Alert,
    Error,
    Activity,
    Accent,
    Success,
    ShellActivity,
}

fn tone_label(tone: StatusTone) -> &'static str {
    match tone {
        StatusTone::Muted => "muted",
        StatusTone::Alert => "alert",
        StatusTone::Error => "error",
        StatusTone::Activity => "activity",
        StatusTone::Accent => "accent",
        StatusTone::Success => "success",
        StatusTone::ShellActivity => "shell_activity",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusSegment {
    pub text: String,
    pub tone: StatusTone,
    pub priority: u8,
}

impl StatusSegment {
    pub fn new(text: impl Into<String>, tone: StatusTone, priority: u8) -> Self {
        let _linkscope_segment = linkscope::phase("ui_model.status.segment.new");
        let text = text.into();
        trace::record_status_segment(trace::StatusSegmentTrace {
            label: "ui_model.status.segment.new",
            tone: tone_label(tone),
            priority,
            text_bytes: text.len(),
        });
        Self {
            text,
            tone,
            priority,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusRow {
    pub segments: Vec<StatusSegment>,
}

impl Default for StatusRow {
    fn default() -> Self {
        trace::record_count("ui_model.status.row.default", 1);
        Self {
            segments: Vec::new(),
        }
    }
}

impl StatusRow {
    pub fn push(&mut self, segment: StatusSegment) {
        let _linkscope_push = linkscope::phase("ui_model.status.row.push");
        let rows_before = self.segments.len();
        self.segments.push(segment);
        trace::record_collection_change(trace::CollectionChange {
            label: "ui_model.status.row.push",
            item_bytes_label: "segment_text_bytes",
            item_bytes: self
                .segments
                .last()
                .map(|segment| segment.text.len())
                .unwrap_or_default(),
            before: rows_before,
            after: self.segments.len(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_row_collects_segments_normal() {
        let mut row = StatusRow::default();
        row.push(StatusSegment::new("1 shell", StatusTone::ShellActivity, 87));

        assert_eq!(row.segments.len(), 1);
        assert_eq!(row.segments[0].text, "1 shell");
        assert_eq!(row.segments[0].tone, StatusTone::ShellActivity);
        assert_eq!(row.segments[0].priority, 87);
    }

    #[test]
    fn status_trace_records_shape_without_text_payload_normal() {
        linkscope::trace_detail_enable();
        let mut row = StatusRow::default();
        row.push(StatusSegment::new(
            "private status text",
            StatusTone::Alert,
            99,
        ));

        let snapshot = linkscope::snapshot();
        let rendered = format!("{snapshot:?}");
        assert!(rendered.contains("ui_model.status.segment.new"));
        assert!(rendered.contains("alert"));
        assert!(rendered.contains("text_bytes"));
        assert!(!rendered.contains("private status text"));
    }
}
