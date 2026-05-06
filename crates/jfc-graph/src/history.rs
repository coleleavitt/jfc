//! Graph query history — stores recent queries and results for inspection.

use std::collections::VecDeque;

use crate::dsl::QueryResult;

/// A recorded graph query with its result.
#[derive(Debug, Clone)]
pub struct QueryRecord {
    pub query_text: String,
    pub result_node_count: usize,
    pub was_truncated: bool,
    pub cycles_detected: usize,
    pub timestamp_ms: u64,
}

/// Stores the last N graph queries for inspection and replay.
pub struct GraphHistory {
    records: VecDeque<QueryRecord>,
    max_records: usize,
}

impl GraphHistory {
    pub fn new(max_records: usize) -> Self {
        Self {
            records: VecDeque::new(),
            max_records,
        }
    }

    /// Record a query execution.
    pub fn record(&mut self, query_text: &str, result: &QueryResult) {
        if self.records.len() >= self.max_records {
            self.records.pop_front();
        }
        self.records.push_back(QueryRecord {
            query_text: query_text.to_string(),
            result_node_count: result.nodes.len(),
            was_truncated: result.was_truncated,
            cycles_detected: result.cycles_detected.len(),
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        });
    }

    /// Get all records (most recent last).
    pub fn all(&self) -> &VecDeque<QueryRecord> {
        &self.records
    }

    /// Get the last N records (most recent first).
    pub fn last_n(&self, n: usize) -> Vec<&QueryRecord> {
        self.records.iter().rev().take(n).collect()
    }

    /// Get total query count.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Clear all history.
    pub fn clear(&mut self) {
        self.records.clear();
    }
}

impl Default for GraphHistory {
    fn default() -> Self {
        Self::new(50)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::QueryResult;
    use crate::nodes::NodeId;

    fn mock_result(count: usize) -> QueryResult {
        QueryResult {
            nodes: (0..count)
                .map(|i| NodeId::new("test.rs", &format!("crate::fn_{i}"), crate::nodes::NodeKind::Function))
                .collect(),
            edges: vec![],
            was_truncated: false,
            total_before_truncation: count,
            cycles_detected: vec![],
        }
    }

    #[test]
    fn test_history_record() {
        let mut history = GraphHistory::new(10);
        history.record("fn(\"foo\") | callees", &mock_result(3));
        assert_eq!(history.len(), 1);
        assert_eq!(history.all()[0].query_text, "fn(\"foo\") | callees");
        assert_eq!(history.all()[0].result_node_count, 3);
    }

    #[test]
    fn test_history_max_records() {
        let mut history = GraphHistory::new(3);
        for i in 0..5 {
            history.record(&format!("query_{i}"), &mock_result(i));
        }
        assert_eq!(history.len(), 3);
        assert_eq!(history.all()[0].query_text, "query_2");
    }

    #[test]
    fn test_history_last_n() {
        let mut history = GraphHistory::new(10);
        for i in 0..5 {
            history.record(&format!("q{i}"), &mock_result(i));
        }
        let last_2 = history.last_n(2);
        assert_eq!(last_2.len(), 2);
        assert_eq!(last_2[0].query_text, "q4");
    }

    #[test]
    fn test_history_clear() {
        let mut history = GraphHistory::new(10);
        history.record("q1", &mock_result(1));
        history.record("q2", &mock_result(2));
        assert_eq!(history.len(), 2);
        history.clear();
        assert!(history.is_empty());
    }
}
