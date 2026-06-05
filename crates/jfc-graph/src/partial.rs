//! Partial struct selection — field-level granularity for context windows.
//!
//! When a function only accesses 2 of 8 struct fields, the context should
//! only show those 2 fields. This module provides the analysis primitives.

use crate::graph::CodeGraph;
use crate::nodes::{NodeId, NodeKind};

#[derive(Debug, Clone)]
pub struct FieldInfo {
    pub name: String,
    pub type_str: String,
    pub is_public: bool,
}

#[derive(Debug, Clone)]
pub struct PartialView {
    pub struct_name: String,
    pub struct_id: NodeId,
    pub all_fields: Vec<FieldInfo>,
    pub accessed_fields: Vec<String>,
    pub is_partial: bool,
}

impl PartialView {
    pub fn visible_fields(&self) -> Vec<&FieldInfo> {
        self.all_fields
            .iter()
            .filter(|f| self.accessed_fields.contains(&f.name))
            .collect()
    }

    pub fn all_fields_with_markers(&self) -> Vec<(&FieldInfo, bool)> {
        self.all_fields
            .iter()
            .map(|f| (f, self.accessed_fields.contains(&f.name)))
            .collect()
    }
}

/// Get a partial view of a struct as seen from a specific accessing function.
pub fn get_partial_struct(
    graph: &CodeGraph,
    struct_id: &NodeId,
    accessing_fn: &NodeId,
) -> Option<PartialView> {
    let struct_node = graph.get_node(struct_id)?;
    if struct_node.kind != NodeKind::Struct {
        return None;
    }

    let fields_str = struct_node.metadata.get("fields")?;
    let all_fields = parse_fields_metadata(fields_str);

    let fn_node = graph.get_node(accessing_fn)?;
    let accessed: Vec<String> = fn_node
        .metadata
        .get("accessed_fields")
        .map(|s| s.split(',').map(|f| f.trim().to_string()).collect())
        .unwrap_or_default();

    let is_partial = !accessed.is_empty() && accessed.len() < all_fields.len();

    Some(PartialView {
        struct_name: struct_node.name.clone(),
        struct_id: struct_id.clone(),
        all_fields,
        accessed_fields: accessed,
        is_partial,
    })
}

/// Parse the fields metadata string.
///
/// Format: "name:type:pub;name:type:priv;..."
fn parse_fields_metadata(raw: &str) -> Vec<FieldInfo> {
    raw.split(';')
        .filter(|s| !s.is_empty())
        .filter_map(|entry| {
            let parts: Vec<&str> = entry.split(':').collect();
            if parts.len() >= 2 {
                Some(FieldInfo {
                    name: parts[0].trim().to_string(),
                    type_str: parts[1].trim().to_string(),
                    is_public: parts.get(2).map(|p| *p == "pub").unwrap_or(false),
                })
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::nodes::{NodeData, Span, Visibility};

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

    const BIG_CONFIG_FIELDS: &str = "name:String:pub;port:u16:pub;host:String:pub;debug:bool:pub;\
         max_retries:u32:pub;timeout_ms:u64:pub;log_level:String:pub;workers:usize:pub";

    fn make_struct_node(fields_meta: &str) -> NodeData {
        let id = NodeId::new("src/lib.rs", "crate::BigConfig", NodeKind::Struct);
        NodeData {
            id,
            kind: NodeKind::Struct,
            name: "BigConfig".to_string(),
            qualified_name: "crate::BigConfig".to_string(),
            file_path: PathBuf::from("src/lib.rs"),
            span: sample_span(),
            visibility: Visibility::Public,
            metadata: HashMap::from([("fields".to_string(), fields_meta.to_string())]),
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
        }
    }

    fn make_fn_node(name: &str, accessed: Option<&str>) -> NodeData {
        let id = NodeId::new("src/lib.rs", &format!("crate::{name}"), NodeKind::Function);
        let mut metadata = HashMap::new();
        if let Some(fields) = accessed {
            metadata.insert("accessed_fields".to_string(), fields.to_string());
        }
        NodeData {
            id,
            kind: NodeKind::Function,
            name: name.to_string(),
            qualified_name: format!("crate::{name}"),
            file_path: PathBuf::from("src/lib.rs"),
            span: sample_span(),
            visibility: Visibility::Public,
            metadata,
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
        }
    }

    #[test]
    fn test_partial_struct_two_fields() {
        let mut graph = CodeGraph::new();

        let struct_node = make_struct_node(BIG_CONFIG_FIELDS);
        let struct_id = graph.add_node(struct_node);

        let fn_node = make_fn_node("uses_two_fields", Some("name, port"));
        let fn_id = graph.add_node(fn_node);

        let view = get_partial_struct(&graph, &struct_id, &fn_id).unwrap();

        assert!(view.is_partial);
        assert_eq!(view.struct_name, "BigConfig");
        assert_eq!(view.all_fields.len(), 8);
        assert_eq!(view.accessed_fields.len(), 2);

        let visible = view.visible_fields();
        assert_eq!(visible.len(), 2);
        assert!(visible.iter().any(|f| f.name == "name"));
        assert!(visible.iter().any(|f| f.name == "port"));
    }

    #[test]
    fn test_partial_struct_all_fields() {
        let mut graph = CodeGraph::new();

        let struct_node = make_struct_node(BIG_CONFIG_FIELDS);
        let struct_id = graph.add_node(struct_node);

        let all = "name, port, host, debug, max_retries, timeout_ms, log_level, workers";
        let fn_node = make_fn_node("uses_all_fields", Some(all));
        let fn_id = graph.add_node(fn_node);

        let view = get_partial_struct(&graph, &struct_id, &fn_id).unwrap();

        assert!(!view.is_partial);
        assert_eq!(view.visible_fields().len(), 8);
    }

    #[test]
    fn test_partial_struct_verbose() {
        let mut graph = CodeGraph::new();

        let struct_node = make_struct_node(BIG_CONFIG_FIELDS);
        let struct_id = graph.add_node(struct_node);

        let fn_node = make_fn_node("uses_two_fields", Some("name, port"));
        let fn_id = graph.add_node(fn_node);

        let view = get_partial_struct(&graph, &struct_id, &fn_id).unwrap();
        let markers = view.all_fields_with_markers();

        assert_eq!(markers.len(), 8);

        let accessed_names: Vec<&str> = markers
            .iter()
            .filter(|(_, accessed)| *accessed)
            .map(|(f, _)| f.name.as_str())
            .collect();
        assert_eq!(accessed_names.len(), 2);
        assert!(accessed_names.contains(&"name"));
        assert!(accessed_names.contains(&"port"));

        let not_accessed: Vec<&str> = markers
            .iter()
            .filter(|(_, accessed)| !*accessed)
            .map(|(f, _)| f.name.as_str())
            .collect();
        assert_eq!(not_accessed.len(), 6);
    }

    #[test]
    fn test_partial_struct_not_a_struct() {
        let mut graph = CodeGraph::new();

        let fn_as_struct = make_fn_node("not_a_struct", Some("x, y"));
        let fn_id = graph.add_node(fn_as_struct);

        let accessor = make_fn_node("accessor", Some("x"));
        let accessor_id = graph.add_node(accessor);

        let result = get_partial_struct(&graph, &fn_id, &accessor_id);
        assert!(result.is_none());
    }
}
