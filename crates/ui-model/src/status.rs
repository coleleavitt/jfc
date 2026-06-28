//! Status-row view models.

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusSegment {
    pub text: String,
    pub tone: StatusTone,
    pub priority: u8,
}

impl StatusSegment {
    pub fn new(text: impl Into<String>, tone: StatusTone, priority: u8) -> Self {
        Self {
            text: text.into(),
            tone,
            priority,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StatusRow {
    pub segments: Vec<StatusSegment>,
}

impl StatusRow {
    pub fn push(&mut self, segment: StatusSegment) {
        self.segments.push(segment);
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
}
