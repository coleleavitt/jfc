//! Build an interprocedural IR map from a live [`CodeGraph`].
//!
//! The slicing ([`crate::slicing`]) and taint ([`crate::taint_v2`]) analyses
//! run over a [`crate::slicing::DataflowOracle`]; the real oracle
//! ([`crate::slicing::PointsToOracle`]) is built from
//! `HashMap<NodeId, IrFunction>` via [`crate::points_to::analyze_interprocedural`].
//! Until now that map was only ever constructed in unit-test fixtures — there
//! was no production driver that turns the indexed graph into IR, so the
//! analyses were unreachable from the agent-facing tool surface.
//!
//! This module is that driver. For every `Function` node in the graph it:
//! 1. reads the function's source file,
//! 2. parses it with the tree-sitter grammar for the file's language,
//! 3. locates the function node covering the node's recorded byte span, and
//! 4. lowers it to an [`IrFunction`] via [`crate::ir::lower_for_language`].
//!
//! IR lowering currently supports rust / python / typescript-javascript / go
//! (see `lower_for_language`); functions in other indexed languages are skipped
//! and simply get no IR entry — the downstream analyses degrade gracefully
//! (those nodes contribute no dataflow edges) rather than erroring.
//!
//! Files are read and parsed once each and cached across the functions that
//! live in them, so the cost is O(files parsed) not O(functions).

use std::collections::HashMap;
use std::path::Path;

use tree_sitter::{Node as TsNode, Parser, Tree};

use crate::graph::CodeGraph;
use crate::ir::{IrFunction, lower_for_language};
use crate::nodes::{NodeId, NodeKind};

/// Map a file extension to the IR-lowering language id, if IR is supported for
/// it. Mirrors the routing in [`crate::ir::lower_for_language`].
fn lang_id_for_ext(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "rs" => "rust",
        "py" | "pyi" => "python",
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => "typescript",
        "go" => "go",
        _ => return None,
    })
}

/// Construct a tree-sitter parser for a supported language id.
fn parser_for(lang_id: &str) -> Option<Parser> {
    let mut parser = Parser::new();
    let lang = match lang_id {
        "rust" => tree_sitter_rust::LANGUAGE.into(),
        "python" => tree_sitter_python::LANGUAGE.into(),
        "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        _ => return None,
    };
    parser.set_language(&lang).ok()?;
    Some(parser)
}

/// Find the smallest named node whose byte range covers `[start, end)` and
/// whose kind names a function-like declaration. Walks down the tree from the
/// root, descending into the child that contains the span.
fn find_function_node<'t>(root: TsNode<'t>, start: usize, end: usize) -> Option<TsNode<'t>> {
    // Descend to the deepest node that still contains the whole span.
    let mut node = root;
    loop {
        let mut cursor = node.walk();
        let child = node
            .named_children(&mut cursor)
            .find(|c| c.start_byte() <= start && c.end_byte() >= end);
        match child {
            Some(c) => node = c,
            None => break,
        }
    }
    // From that node walk back up to the nearest function-like ancestor.
    let mut cur = Some(node);
    while let Some(n) = cur {
        if is_function_kind(n.kind()) {
            return Some(n);
        }
        cur = n.parent();
    }
    None
}

/// tree-sitter node kinds that denote a function/method across the supported
/// grammars.
fn is_function_kind(kind: &str) -> bool {
    matches!(
        kind,
        "function_item"            // rust
            | "function_definition"  // python, c-like
            | "function_declaration" // go, js/ts
            | "method_declaration"   // go, ts
            | "method_definition"    // js/ts
            | "arrow_function"       // js/ts
    )
}

/// A parsed file: its tree + source, cached so multiple functions in one file
/// reuse a single parse.
struct ParsedSource {
    tree: Tree,
    source: String,
    lang_id: &'static str,
}

/// Build the IR map for every supported-language `Function` node in `graph`.
///
/// Returns a map from each function's [`NodeId`] to its lowered [`IrFunction`].
/// Functions whose file can't be read, whose language has no IR lowering, or
/// whose body can't be located/lowered are simply absent from the map.
pub fn build_ir_map(graph: &CodeGraph) -> HashMap<NodeId, IrFunction> {
    let mut out: HashMap<NodeId, IrFunction> = HashMap::new();
    // Cache one parse per file path.
    let mut file_cache: HashMap<String, Option<ParsedSource>> = HashMap::new();

    for node in graph.nodes_by_kind(NodeKind::Function) {
        let path = &node.file_path;
        let key = path.display().to_string();

        let parsed = file_cache
            .entry(key)
            .or_insert_with(|| parse_source_file(path));
        let Some(parsed) = parsed.as_ref() else {
            continue;
        };

        let start = node.span.byte_range.start;
        let end = node.span.byte_range.end;
        let Some(fn_node) = find_function_node(parsed.tree.root_node(), start, end) else {
            continue;
        };
        if let Some(ir) = lower_for_language(parsed.lang_id, fn_node, &parsed.source) {
            out.insert(node.id.clone(), ir);
        }
    }
    out
}

/// Read + parse one file, returning `None` if unsupported or unreadable.
fn parse_source_file(path: &Path) -> Option<ParsedSource> {
    let ext = path.extension().and_then(|e| e.to_str())?;
    let lang_id = lang_id_for_ext(ext)?;
    let source = std::fs::read_to_string(path).ok()?;
    let mut parser = parser_for(lang_id)?;
    let tree = parser.parse(&source, None)?;
    Some(ParsedSource { tree, source, lang_id })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edges::{EdgeData, EdgeKind};
    use crate::nodes::{NodeData, Span, Visibility};
    use std::collections::HashMap as Map;
    use std::io::Write;

    fn write_temp(name: &str, content: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("jfc_irmap_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    fn fn_node(path: &Path, name: &str, source: &str, fn_decl: &str) -> NodeData {
        // byte range of the function declaration within source.
        let start = source.find(fn_decl).expect("fn present");
        let end = start + fn_decl.len();
        NodeData {
            id: NodeId::new(&path.display().to_string(), name, NodeKind::Function),
            kind: NodeKind::Function,
            name: name.into(),
            qualified_name: name.into(),
            file_path: path.to_path_buf(),
            span: Span {
                file: path.to_path_buf(),
                start_line: 1,
                start_col: 0,
                end_line: 1,
                end_col: 0,
                byte_range: start..end,
            },
            visibility: Visibility::Public,
            metadata: Map::new(),
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
        }
    }

    // Normal: a rust function in a real file lowers to IR keyed by its NodeId.
    #[test]
    fn builds_ir_for_rust_function_normal() {
        let src = "pub fn add_one(x: i32) -> i32 {\n    let y = x + 1;\n    y\n}\n";
        let path = write_temp("add.rs", src);
        let mut g = CodeGraph::new();
        let id = g.add_node(fn_node(&path, "add_one", src, "fn add_one"));

        let ir_map = build_ir_map(&g);
        assert!(ir_map.contains_key(&id), "expected IR for add_one");
        assert_eq!(ir_map[&id].name, "add_one");
        let _ = std::fs::remove_file(&path);
    }

    // Robust: a function in an unsupported language is skipped, not an error.
    #[test]
    fn unsupported_language_is_skipped_robust() {
        let src = "fn whatever() {}";
        let path = write_temp("thing.zzz", src);
        let mut g = CodeGraph::new();
        let id = g.add_node(fn_node(&path, "whatever", src, "fn whatever"));

        let ir_map = build_ir_map(&g);
        assert!(!ir_map.contains_key(&id));
        let _ = std::fs::remove_file(&path);
    }

    // Robust: a missing file is skipped rather than panicking.
    #[test]
    fn missing_file_is_skipped_robust() {
        let mut g = CodeGraph::new();
        let phantom = Path::new("/nonexistent/jfc_irmap/ghost.rs");
        let id = g.add_node(fn_node(phantom, "ghost", "fn ghost() {}", "fn ghost"));
        let ir_map = build_ir_map(&g);
        assert!(!ir_map.contains_key(&id));
    }

    // Robust: a non-function node never appears in the IR map.
    #[test]
    fn non_function_nodes_absent_robust() {
        let src = "pub fn real() {}\n";
        let path = write_temp("real.rs", src);
        let mut g = CodeGraph::new();
        let f = g.add_node(fn_node(&path, "real", src, "fn real"));
        // A struct node sharing the file — must not get IR.
        let s = NodeData {
            id: NodeId::new(&path.display().to_string(), "S", NodeKind::Struct),
            kind: NodeKind::Struct,
            name: "S".into(),
            qualified_name: "S".into(),
            file_path: path.clone(),
            span: Span {
                file: path.clone(),
                start_line: 1,
                start_col: 0,
                end_line: 1,
                end_col: 0,
                byte_range: 0..1,
            },
            visibility: Visibility::Public,
            metadata: Map::new(),
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
        };
        let s_id = g.add_node(s);
        g.add_edge(&f, &s_id, EdgeData { kind: EdgeKind::UsesType, source_span: g.get_node(&f).unwrap().span.clone(), weight: 1.0 }).ok();

        let ir_map = build_ir_map(&g);
        assert!(ir_map.contains_key(&f));
        assert!(!ir_map.contains_key(&s_id));
        let _ = std::fs::remove_file(&path);
    }
}
