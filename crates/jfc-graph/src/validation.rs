//! Virtual edit validation — pre-commit simulation for signature changes.

use crate::edges::EdgeKind;
use crate::graph::CodeGraph;
use crate::nodes::{NodeId, Span};

/// Result of validating a signature change.
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// Call sites that are compatible with the new signature.
    pub compatible: Vec<NodeId>,
    /// Call sites that are incompatible: (node_id, reason).
    pub incompatible: Vec<(NodeId, String)>,
    /// Whether the edit is safe (no incompatible sites).
    pub is_safe: bool,
}

/// Information about a call site that would be affected by an edit.
#[derive(Debug, Clone)]
pub struct AffectedCallSite {
    pub caller_id: NodeId,
    pub caller_name: String,
    pub call_span: Span,
}

/// Validates edits before they're applied.
pub struct VirtualValidator<'a> {
    graph: &'a CodeGraph,
}

impl<'a> VirtualValidator<'a> {
    pub fn new(graph: &'a CodeGraph) -> Self {
        Self { graph }
    }

    /// Validate a function signature change.
    /// Checks all callers of `target` to see if they pass the right number of args.
    pub fn validate_signature_change(
        &self,
        target: &NodeId,
        old_param_count: usize,
        new_param_count: usize,
    ) -> ValidationResult {
        let mut compatible = Vec::new();
        let mut incompatible = Vec::new();

        for (caller_id, edge) in self.graph.get_edges_to(target) {
            if !matches!(edge.kind, EdgeKind::Calls) {
                continue;
            }

            if old_param_count != new_param_count {
                let reason = format!(
                    "passes {} args, but new signature expects {}",
                    old_param_count, new_param_count
                );
                incompatible.push((caller_id.clone(), reason));
            } else {
                compatible.push(caller_id.clone());
            }
        }

        let is_safe = incompatible.is_empty();
        ValidationResult {
            compatible,
            incompatible,
            is_safe,
        }
    }

    /// Preview which call sites would be affected by editing a function.
    pub fn preview_affected_call_sites(&self, target: &NodeId) -> Vec<AffectedCallSite> {
        let mut sites = Vec::new();

        for (caller_id, edge) in self.graph.get_edges_to(target) {
            if matches!(edge.kind, EdgeKind::Calls | EdgeKind::UnresolvedCall(_)) {
                if let Some(caller_node) = self.graph.get_node(caller_id) {
                    sites.push(AffectedCallSite {
                        caller_id: caller_id.clone(),
                        caller_name: caller_node.name.clone(),
                        call_span: edge.source_span.clone(),
                    });
                }
            }
        }

        sites
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::edges::EdgeData;
    use crate::graph::CodeGraph;
    use crate::nodes::{NodeData, NodeKind, Span, Visibility};

    fn sample_span() -> Span {
        Span {
            file: PathBuf::from("src/lib.rs"),
            start_line: 1,
            start_col: 0,
            end_line: 10,
            end_col: 1,
            byte_range: 0..100,
        }
    }

    fn make_node(name: &str) -> NodeData {
        let id = NodeId::new("src/lib.rs", &format!("crate::{name}"), NodeKind::Function);
        NodeData {
            id,
            kind: NodeKind::Function,
            name: name.to_string(),
            qualified_name: format!("crate::{name}"),
            file_path: PathBuf::from("src/lib.rs"),
            span: sample_span(),
            visibility: Visibility::Public,
            metadata: HashMap::new(),
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
        }
    }

    fn make_call_edge() -> EdgeData {
        EdgeData {
            kind: EdgeKind::Calls,
            source_span: sample_span(),
            weight: 1.0,
        }
    }

    fn make_unresolved_edge(name: &str) -> EdgeData {
        EdgeData {
            kind: EdgeKind::UnresolvedCall(name.to_string()),
            source_span: sample_span(),
            weight: 0.5,
        }
    }

    #[test]
    fn test_validate_added_param() {
        let mut graph = CodeGraph::new();

        let target = make_node("target_fn");
        let caller_a = make_node("caller_a");
        let caller_b = make_node("caller_b");

        let target_id = graph.add_node(target);
        let caller_a_id = graph.add_node(caller_a);
        let caller_b_id = graph.add_node(caller_b);

        graph
            .add_edge(&caller_a_id, &target_id, make_call_edge())
            .unwrap();
        graph
            .add_edge(&caller_b_id, &target_id, make_call_edge())
            .unwrap();

        let validator = VirtualValidator::new(&graph);
        let result = validator.validate_signature_change(&target_id, 0, 1);

        assert!(!result.is_safe);
        assert!(result.compatible.is_empty());
        assert_eq!(result.incompatible.len(), 2);

        // Both callers should be flagged
        let ids: Vec<&NodeId> = result.incompatible.iter().map(|(id, _)| id).collect();
        assert!(ids.contains(&&caller_a_id));
        assert!(ids.contains(&&caller_b_id));

        // Check reason message
        for (_, reason) in &result.incompatible {
            assert!(reason.contains("passes 0 args"));
            assert!(reason.contains("expects 1"));
        }
    }

    #[test]
    fn test_validate_no_change() {
        let mut graph = CodeGraph::new();

        let target = make_node("target_fn");
        let caller = make_node("caller_fn");

        let target_id = graph.add_node(target);
        let caller_id = graph.add_node(caller);

        graph
            .add_edge(&caller_id, &target_id, make_call_edge())
            .unwrap();

        let validator = VirtualValidator::new(&graph);
        let result = validator.validate_signature_change(&target_id, 2, 2);

        assert!(result.is_safe);
        assert_eq!(result.compatible.len(), 1);
        assert_eq!(result.compatible[0], caller_id);
        assert!(result.incompatible.is_empty());
    }

    #[test]
    fn test_validate_no_callers() {
        let mut graph = CodeGraph::new();

        let target = make_node("lonely_fn");
        let target_id = graph.add_node(target);

        let validator = VirtualValidator::new(&graph);
        let result = validator.validate_signature_change(&target_id, 1, 3);

        assert!(result.is_safe);
        assert!(result.compatible.is_empty());
        assert!(result.incompatible.is_empty());
    }

    #[test]
    fn test_preview_affected() {
        let mut graph = CodeGraph::new();

        let target = make_node("target_fn");
        let caller_a = make_node("caller_a");
        let caller_b = make_node("caller_b");

        let target_id = graph.add_node(target);
        let caller_a_id = graph.add_node(caller_a);
        let caller_b_id = graph.add_node(caller_b);

        // One resolved call, one unresolved call
        graph
            .add_edge(&caller_a_id, &target_id, make_call_edge())
            .unwrap();
        graph
            .add_edge(&caller_b_id, &target_id, make_unresolved_edge("target_fn"))
            .unwrap();

        let validator = VirtualValidator::new(&graph);
        let sites = validator.preview_affected_call_sites(&target_id);

        assert_eq!(sites.len(), 2);

        let names: Vec<&str> = sites.iter().map(|s| s.caller_name.as_str()).collect();
        assert!(names.contains(&"caller_a"));
        assert!(names.contains(&"caller_b"));

        // All spans should match our sample
        for site in &sites {
            assert_eq!(site.call_span, sample_span());
        }
    }
}
