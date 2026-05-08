//! Sub-agent cascade task generation for signature changes.
//!
//! When a function signature changes, all call sites need updating.
//! This module generates structured [`CascadeTask`]s grouped by file,
//! ready for dispatch to sub-agents.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::graph::CodeGraph;
use crate::nodes::{NodeId, Span};
use crate::validation::VirtualValidator;

/// A task to be dispatched to a sub-agent for updating call sites in a single file.
#[derive(Debug, Clone)]
pub struct CascadeTask {
    /// What changed and why.
    pub edit_description: String,
    /// The new function signature.
    pub new_signature: String,
    /// Call sites to update (all in the same file).
    pub call_sites: Vec<CascadeCallSite>,
    /// Instruction for the sub-agent.
    pub instruction: String,
}

/// A single call site that needs updating.
#[derive(Debug, Clone)]
pub struct CascadeCallSite {
    pub caller_id: NodeId,
    pub caller_name: String,
    pub file_path: PathBuf,
    pub call_span: Span,
}

/// Generate cascade tasks for a signature change.
///
/// Groups affected call sites by file — overlapping edits in the same file
/// become a single task to avoid conflicts.
pub fn generate_cascade(
    graph: &CodeGraph,
    target: &NodeId,
    new_signature: &str,
    edit_description: &str,
) -> Vec<CascadeTask> {
    let validator = VirtualValidator::new(graph);
    let affected = validator.preview_affected_call_sites(target);

    if affected.is_empty() {
        return Vec::new();
    }

    // Group by file
    let mut by_file: HashMap<PathBuf, Vec<CascadeCallSite>> = HashMap::new();
    for site in affected {
        let file = site.call_span.file.clone();
        by_file.entry(file).or_default().push(CascadeCallSite {
            caller_id: site.caller_id,
            caller_name: site.caller_name,
            file_path: site.call_span.file.clone(),
            call_span: site.call_span,
        });
    }

    // One CascadeTask per file
    by_file
        .into_iter()
        .map(|(file, sites)| {
            let site_names: Vec<&str> = sites.iter().map(|s| s.caller_name.as_str()).collect();
            let instruction = format!(
                "Update call sites in {} to match new signature: {}. Affected functions: {}",
                file.display(),
                new_signature,
                site_names.join(", ")
            );
            CascadeTask {
                edit_description: edit_description.to_string(),
                new_signature: new_signature.to_string(),
                call_sites: sites,
                instruction,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::edges::{EdgeData, EdgeKind};
    use crate::nodes::{NodeData, NodeKind, Span, Visibility};

    fn make_span(file: &str) -> Span {
        Span {
            file: PathBuf::from(file),
            start_line: 1,
            start_col: 0,
            end_line: 10,
            end_col: 1,
            byte_range: 0..100,
        }
    }

    fn make_node(file: &str, name: &str) -> NodeData {
        let id = NodeId::new(file, &format!("crate::{name}"), NodeKind::Function);
        NodeData {
            id,
            kind: NodeKind::Function,
            name: name.to_string(),
            qualified_name: format!("crate::{name}"),
            file_path: PathBuf::from(file),
            span: make_span(file),
            visibility: Visibility::Public,
            metadata: HashMap::new(),
            birth_revision: 0,
            last_modified_revision: 0,
        }
    }

    fn make_call_edge(file: &str) -> EdgeData {
        EdgeData {
            kind: EdgeKind::Calls,
            source_span: make_span(file),
            weight: 1.0,
        }
    }

    #[test]
    fn test_cascade_generation() {
        let mut graph = CodeGraph::new();

        let bar = make_node("src/lib.rs", "bar");
        let foo = make_node("src/lib.rs", "foo");

        let bar_id = graph.add_node(bar);
        let foo_id = graph.add_node(foo);

        graph
            .add_edge(&foo_id, &bar_id, make_call_edge("src/lib.rs"))
            .unwrap();

        let tasks = generate_cascade(
            &graph,
            &bar_id,
            "fn bar(x: i32, y: i32) -> bool",
            "added parameter y",
        );

        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].call_sites.len(), 1);
        assert_eq!(tasks[0].call_sites[0].caller_name, "foo");
        assert_eq!(tasks[0].new_signature, "fn bar(x: i32, y: i32) -> bool");
        assert!(tasks[0].instruction.contains("foo"));
        assert!(tasks[0].instruction.contains("bar(x: i32, y: i32)"));
    }

    #[test]
    fn test_cascade_groups_by_file() {
        let mut graph = CodeGraph::new();

        let target = make_node("src/target.rs", "target_fn");
        let caller_same_1 = make_node("src/caller.rs", "caller_a");
        let caller_same_2 = make_node("src/caller.rs", "caller_b");
        let caller_other = make_node("src/other.rs", "caller_c");

        let target_id = graph.add_node(target);
        let caller_a_id = graph.add_node(caller_same_1);
        let caller_b_id = graph.add_node(caller_same_2);
        let caller_c_id = graph.add_node(caller_other);

        graph
            .add_edge(&caller_a_id, &target_id, make_call_edge("src/caller.rs"))
            .unwrap();
        graph
            .add_edge(&caller_b_id, &target_id, make_call_edge("src/caller.rs"))
            .unwrap();
        graph
            .add_edge(&caller_c_id, &target_id, make_call_edge("src/other.rs"))
            .unwrap();

        let tasks = generate_cascade(&graph, &target_id, "fn target_fn(x: u8)", "added param");

        // 2 callers in src/caller.rs → 1 task, 1 caller in src/other.rs → 1 task
        assert_eq!(tasks.len(), 2);

        let caller_rs_task = tasks
            .iter()
            .find(|t| t.call_sites[0].file_path == PathBuf::from("src/caller.rs"))
            .expect("should have task for src/caller.rs");
        assert_eq!(caller_rs_task.call_sites.len(), 2);

        let other_rs_task = tasks
            .iter()
            .find(|t| t.call_sites[0].file_path == PathBuf::from("src/other.rs"))
            .expect("should have task for src/other.rs");
        assert_eq!(other_rs_task.call_sites.len(), 1);
    }

    #[test]
    fn test_cascade_no_callers() {
        let mut graph = CodeGraph::new();

        let lonely = make_node("src/lib.rs", "lonely_fn");
        let lonely_id = graph.add_node(lonely);

        let tasks = generate_cascade(&graph, &lonely_id, "fn lonely_fn()", "no change");
        assert!(tasks.is_empty());
    }
}
