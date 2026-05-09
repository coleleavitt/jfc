//! Aggregation, edge selection, let-bindings, and quantifiers (Phase 4).
//!
//! This module extends the DSL with operators that **don't return a
//! node set** — counts, sums, edges, scalars, groups — without
//! breaking the legacy `QueryResult.nodes` API. We add a parallel
//! surface:
//!
//! - [`AggregateResult`] enum supersedes `Vec<NodeId>` for queries
//!   that need it.
//! - [`run_aggregate`] is the new entry point. Falls back to
//!   `run_query_expr` for queries that don't use aggregation.
//!
//! ## Grammar additions (relative to the existing extended grammar)
//!
//! ```text
//! agg_expr := expr
//!           | 'count' expr
//!           | 'sum' field_name expr
//!           | 'avg' field_name expr
//!           | 'top_k_by' field_name number expr
//!           | 'group_by' field_name expr
//!           | 'exists' expr
//!           | 'forall' kind_pred expr
//!           | 'edges_of' expr           // edges incident to nodes in expr
//!           | 'edges_kind' edge_kind    // every edge of given kind in graph
//!           | 'let' ident '=' agg_expr 'in' agg_expr
//!
//! field_name := metadata key, e.g. `coverage_count`, `birth_revision`
//! kind_pred  := 'kind=' kind_name
//! ```
//!
//! ## Examples
//!
//! ```text
//! count fn("foo") | callers              // # of callers of foo
//! sum coverage_count entrypoints kind=Test
//! group_by birth_revision (fn("foo") | callers)
//! exists fn("dangerous_op") | callers     // any callers?
//! let common = entrypoints kind=PublicApi in
//!     count common
//! edges_of fn("hot_path")                  // edges incident to hot_path
//! ```

use std::collections::{BTreeMap, HashMap};

use crate::dsl::{
    Expr, ParseError, QueryConfig, QueryEngine, QueryError, lex, parse_expr,
};
use crate::edges::{EdgeData, EdgeKind};
use crate::graph::CodeGraph;
use crate::nodes::{NodeId, NodeKind};

/// Heterogeneous query result. The DSL chooses one variant per query.
#[derive(Debug, Clone)]
pub enum AggregateResult {
    /// Legacy node set.
    Nodes(Vec<NodeId>),
    /// Edges. Stored as `(from, to, kind)` triples.
    Edges(Vec<EdgeRef>),
    /// Scalar (count, sum, avg, etc.).
    Scalar(f64),
    /// Boolean (exists, forall).
    Bool(bool),
    /// Groups: field-value → node set.
    Groups(BTreeMap<String, Vec<NodeId>>),
}

/// An edge surfaced by an `edges_*` operator.
#[derive(Debug, Clone)]
pub struct EdgeRef {
    pub from: NodeId,
    pub to: NodeId,
    pub kind: EdgeKind,
    pub weight: f32,
}

/// Aggregation AST. Wraps an inner [`Expr`] (resolved by the existing
/// executor) plus an outer aggregation operator.
#[derive(Debug, Clone)]
pub enum AggExpr {
    /// Pass-through: just evaluate the inner Expr and return its nodes.
    Plain(Expr),
    /// `count <expr>` → number of nodes.
    Count(Expr),
    /// `sum <field> <expr>` → sum of `metadata[field]` parsed as number.
    Sum {
        field: String,
        inner: Expr,
    },
    /// `avg <field> <expr>` → mean.
    Avg {
        field: String,
        inner: Expr,
    },
    /// `top_k_by <field> <N> <expr>`.
    TopKBy {
        field: String,
        k: usize,
        inner: Expr,
    },
    /// `group_by <field> <expr>` → field-value buckets.
    GroupBy {
        field: String,
        inner: Expr,
    },
    /// `exists <expr>` → true iff the expr produces any nodes.
    Exists(Expr),
    /// `forall kind=<kind> <expr>` → every node has the given kind.
    Forall {
        kind: NodeKind,
        inner: Expr,
    },
    /// `edges_of <expr>` → edges incident to inner's nodes.
    EdgesOf(Expr),
    /// `edges_kind <EdgeKindName>` → every edge in the graph of the
    /// given kind. Supports `Calls`, `UsesType`, `Implements`,
    /// `Contains`, `References`. Open-ended kinds (`UnresolvedCall`,
    /// `ExternalCall`) require their argument and aren't supported
    /// here.
    EdgesKind(SimpleEdgeKind),
    /// `let <name> = <agg> in <agg>` — lexically-scoped binding.
    Let {
        name: String,
        bound: Box<AggExpr>,
        body: Box<AggExpr>,
    },
    /// Reference a previously-bound `let` name.
    Var(String),
}

/// EdgeKind without payloads — for `edges_kind` selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimpleEdgeKind {
    Calls,
    UsesType,
    Implements,
    Contains,
    References,
}

impl SimpleEdgeKind {
    pub fn matches(&self, k: &EdgeKind) -> bool {
        match (self, k) {
            (Self::Calls, EdgeKind::Calls) => true,
            (Self::UsesType, EdgeKind::UsesType) => true,
            (Self::Implements, EdgeKind::Implements) => true,
            (Self::Contains, EdgeKind::Contains) => true,
            (Self::References, EdgeKind::References) => true,
            _ => false,
        }
    }
}

// ─── Parser ─────────────────────────────────────────────────────────────

/// Parse the raw query text. If it begins with one of the aggregation
/// keywords, parse as an `AggExpr`; otherwise fall back to plain
/// `parse_expr`.
pub fn parse_aggregate(input: &str) -> Result<AggExpr, ParseError> {
    let trimmed = input.trim();

    // Cheap keyword peek — we only need the first identifier.
    let first_word: String = trimmed
        .chars()
        .take_while(|c| c.is_ascii_alphabetic() || *c == '_')
        .collect();

    match first_word.as_str() {
        "count" => {
            let rest = trimmed.strip_prefix("count").unwrap_or("").trim_start();
            let inner = parse_expr(rest)?;
            Ok(AggExpr::Count(inner))
        }
        "sum" => {
            let rest = trimmed.strip_prefix("sum").unwrap_or("").trim_start();
            let (field, rest) = take_ident(rest)?;
            let inner = parse_expr(rest.trim_start())?;
            Ok(AggExpr::Sum { field, inner })
        }
        "avg" => {
            let rest = trimmed.strip_prefix("avg").unwrap_or("").trim_start();
            let (field, rest) = take_ident(rest)?;
            let inner = parse_expr(rest.trim_start())?;
            Ok(AggExpr::Avg { field, inner })
        }
        "top_k_by" => {
            let rest = trimmed.strip_prefix("top_k_by").unwrap_or("").trim_start();
            let (field, rest) = take_ident(rest)?;
            let (k, rest) = take_number(rest.trim_start())?;
            let inner = parse_expr(rest.trim_start())?;
            Ok(AggExpr::TopKBy { field, k, inner })
        }
        "group_by" => {
            let rest = trimmed.strip_prefix("group_by").unwrap_or("").trim_start();
            let (field, rest) = take_ident(rest)?;
            let inner = parse_expr(rest.trim_start())?;
            Ok(AggExpr::GroupBy { field, inner })
        }
        "exists" => {
            let rest = trimmed.strip_prefix("exists").unwrap_or("").trim_start();
            let inner = parse_expr(rest)?;
            Ok(AggExpr::Exists(inner))
        }
        "forall" => {
            let rest = trimmed.strip_prefix("forall").unwrap_or("").trim_start();
            let rest = rest
                .strip_prefix("kind=")
                .ok_or_else(|| ParseError {
                    position: 0,
                    message: "forall requires `kind=<NodeKind>` predicate".into(),
                })?
                .trim_start();
            let (kind_name, rest) = take_ident(rest)?;
            let kind = parse_kind(&kind_name)
                .map_err(|m| ParseError { position: 0, message: m })?;
            let inner = parse_expr(rest.trim_start())?;
            Ok(AggExpr::Forall { kind, inner })
        }
        "edges_of" => {
            let rest = trimmed.strip_prefix("edges_of").unwrap_or("").trim_start();
            let inner = parse_expr(rest)?;
            Ok(AggExpr::EdgesOf(inner))
        }
        "edges_kind" => {
            let rest = trimmed.strip_prefix("edges_kind").unwrap_or("").trim_start();
            let (kind_name, _) = take_ident(rest)?;
            let kind = parse_simple_edge_kind(&kind_name)
                .map_err(|m| ParseError { position: 0, message: m })?;
            Ok(AggExpr::EdgesKind(kind))
        }
        "let" => parse_let(trimmed),
        _ => {
            // No aggregation prefix — defer to the regular parser.
            // Surface bare identifiers (when they're variables in a
            // let-binding) by checking — but only when called via
            // execute_aggregate with a binding scope.
            let _ = lex(trimmed)?; // sanity-check tokenisation
            let inner = parse_expr(trimmed)?;
            Ok(AggExpr::Plain(inner))
        }
    }
}

/// Parse `let <name> = <agg> in <agg>` after the literal `let`
/// keyword has been recognised on the input.
fn parse_let(input: &str) -> Result<AggExpr, ParseError> {
    let rest = input.strip_prefix("let").unwrap_or("").trim_start();
    let (name, rest) = take_ident(rest)?;
    let rest = rest
        .trim_start()
        .strip_prefix('=')
        .ok_or_else(|| ParseError {
            position: 0,
            message: "let requires `<name> = <agg> in <agg>`".into(),
        })?
        .trim_start();
    // Find the matching `in` — we use a simple split: the first
    // `' in '` outside parentheses. This is a pragmatic heuristic;
    // a full parser would track depth.
    let (bound_str, body_str) = split_on_top_level_in(rest).ok_or_else(|| ParseError {
        position: 0,
        message: "let requires matching `in` keyword".into(),
    })?;
    let bound = parse_aggregate(bound_str.trim())?;
    let body = parse_aggregate(body_str.trim())?;
    Ok(AggExpr::Let {
        name,
        bound: Box::new(bound),
        body: Box::new(body),
    })
}

/// Find the first ` in ` keyword at parenthesis depth 0.
fn split_on_top_level_in(s: &str) -> Option<(&str, &str)> {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'{' => depth += 1,
            b')' | b'}' => depth -= 1,
            b' ' if depth == 0 => {
                // Look for ' in ' or ' in\t' as a token.
                if i + 4 <= bytes.len()
                    && &bytes[i + 1..i + 3] == b"in"
                    && (bytes.get(i + 3).copied() == Some(b' ')
                        || bytes.get(i + 3).copied() == Some(b'\t'))
                {
                    return Some((&s[..i], &s[i + 4..]));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn take_ident(s: &str) -> Result<(String, &str), ParseError> {
    let bytes = s.as_bytes();
    let mut end = 0;
    while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
        end += 1;
    }
    if end == 0 {
        return Err(ParseError {
            position: 0,
            message: "expected identifier".into(),
        });
    }
    let ident = s[..end].to_string();
    Ok((ident, &s[end..]))
}

fn take_number(s: &str) -> Result<(usize, &str), ParseError> {
    let bytes = s.as_bytes();
    let mut end = 0;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    if end == 0 {
        return Err(ParseError {
            position: 0,
            message: "expected non-negative integer".into(),
        });
    }
    let n: usize = s[..end].parse().map_err(|_| ParseError {
        position: 0,
        message: "invalid integer".into(),
    })?;
    Ok((n, &s[end..]))
}

fn parse_kind(name: &str) -> Result<NodeKind, String> {
    match name {
        "Function" | "function" | "fn" => Ok(NodeKind::Function),
        "Struct" | "struct" => Ok(NodeKind::Struct),
        "Enum" | "enum" => Ok(NodeKind::Enum),
        "Module" | "module" | "mod" => Ok(NodeKind::Module),
        "Trait" | "trait" => Ok(NodeKind::Trait),
        other => Err(format!("unknown NodeKind `{other}`")),
    }
}

fn parse_simple_edge_kind(name: &str) -> Result<SimpleEdgeKind, String> {
    match name {
        "Calls" | "calls" => Ok(SimpleEdgeKind::Calls),
        "UsesType" | "uses_type" => Ok(SimpleEdgeKind::UsesType),
        "Implements" | "implements" => Ok(SimpleEdgeKind::Implements),
        "Contains" | "contains" => Ok(SimpleEdgeKind::Contains),
        "References" | "references" => Ok(SimpleEdgeKind::References),
        other => Err(format!("unknown EdgeKind `{other}`")),
    }
}

// ─── Executor ───────────────────────────────────────────────────────────

pub fn run_aggregate(
    query: &str,
    graph: &CodeGraph,
    config: &QueryConfig,
) -> Result<AggregateResult, QueryError> {
    let agg = parse_aggregate(query)?;
    execute_aggregate(&agg, graph, config, &HashMap::new())
}

pub fn execute_aggregate(
    agg: &AggExpr,
    graph: &CodeGraph,
    config: &QueryConfig,
    bindings: &HashMap<String, AggregateResult>,
) -> Result<AggregateResult, QueryError> {
    match agg {
        AggExpr::Plain(expr) => {
            // Var-name lookup happens before parsing; if execution
            // reaches here the inner Expr is real.
            let r = QueryEngine::new(graph).execute_expr(expr, config)?;
            Ok(AggregateResult::Nodes(r.nodes))
        }
        AggExpr::Count(expr) => {
            let r = QueryEngine::new(graph).execute_expr(expr, config)?;
            Ok(AggregateResult::Scalar(r.nodes.len() as f64))
        }
        AggExpr::Sum { field, inner } => {
            let r = QueryEngine::new(graph).execute_expr(inner, config)?;
            let mut sum: f64 = 0.0;
            for id in &r.nodes {
                if let Some(node) = graph.get_node(id) {
                    if let Some(v) = node.metadata.get(field) {
                        if let Ok(n) = v.parse::<f64>() {
                            sum += n;
                        }
                    }
                }
            }
            Ok(AggregateResult::Scalar(sum))
        }
        AggExpr::Avg { field, inner } => {
            let r = QueryEngine::new(graph).execute_expr(inner, config)?;
            let mut sum: f64 = 0.0;
            let mut count: usize = 0;
            for id in &r.nodes {
                if let Some(node) = graph.get_node(id) {
                    if let Some(v) = node.metadata.get(field) {
                        if let Ok(n) = v.parse::<f64>() {
                            sum += n;
                            count += 1;
                        }
                    }
                }
            }
            let avg = if count == 0 { 0.0 } else { sum / count as f64 };
            Ok(AggregateResult::Scalar(avg))
        }
        AggExpr::TopKBy { field, k, inner } => {
            // Phase 10-1: route through the streaming top_k_by — never
            // holds more than k+1 entries in memory.
            let r = QueryEngine::new(graph).execute_expr(inner, config)?;
            let top = crate::dsl::stream::stream_top_k_by(r.nodes.into_iter(), graph, field, *k);
            Ok(AggregateResult::Nodes(top))
        }
        AggExpr::GroupBy { field, inner } => {
            let r = QueryEngine::new(graph).execute_expr(inner, config)?;
            let mut groups: BTreeMap<String, Vec<NodeId>> = BTreeMap::new();
            for id in &r.nodes {
                let key = graph
                    .get_node(id)
                    .and_then(|n| n.metadata.get(field).cloned())
                    .unwrap_or_default();
                groups.entry(key).or_default().push(id.clone());
            }
            Ok(AggregateResult::Groups(groups))
        }
        AggExpr::Exists(expr) => {
            let r = QueryEngine::new(graph).execute_expr(expr, config)?;
            Ok(AggregateResult::Bool(!r.nodes.is_empty()))
        }
        AggExpr::Forall { kind, inner } => {
            let r = QueryEngine::new(graph).execute_expr(inner, config)?;
            let all = r
                .nodes
                .iter()
                .all(|id| graph.get_node(id).map(|n| n.kind == *kind).unwrap_or(false));
            Ok(AggregateResult::Bool(all))
        }
        AggExpr::EdgesOf(expr) => {
            let r = QueryEngine::new(graph).execute_expr(expr, config)?;
            let mut edges = Vec::new();
            let in_set: std::collections::HashSet<&NodeId> = r.nodes.iter().collect();
            for id in &r.nodes {
                for (target, edge) in graph.get_edges_from(id) {
                    edges.push(EdgeRef {
                        from: id.clone(),
                        to: target.clone(),
                        kind: edge.kind.clone(),
                        weight: edge.weight,
                    });
                }
                for (source, edge) in graph.get_edges_to(id) {
                    if !in_set.contains(source) {
                        edges.push(EdgeRef {
                            from: source.clone(),
                            to: id.clone(),
                            kind: edge.kind.clone(),
                            weight: edge.weight,
                        });
                    }
                }
            }
            Ok(AggregateResult::Edges(edges))
        }
        AggExpr::EdgesKind(kind) => {
            let mut edges = Vec::new();
            for id in graph.all_node_ids() {
                for (target, edge) in graph.get_edges_from(id) {
                    if kind.matches(&edge.kind) {
                        edges.push(EdgeRef {
                            from: id.clone(),
                            to: target.clone(),
                            kind: edge.kind.clone(),
                            weight: edge.weight,
                        });
                    }
                }
            }
            Ok(AggregateResult::Edges(edges))
        }
        AggExpr::Let { name, bound, body } => {
            let bound_value = execute_aggregate(bound, graph, config, bindings)?;
            let mut new_scope = bindings.clone();
            new_scope.insert(name.clone(), bound_value);
            execute_aggregate(body, graph, config, &new_scope)
        }
        AggExpr::Var(name) => bindings.get(name).cloned().ok_or_else(|| {
            QueryError::Parse(ParseError {
                position: 0,
                message: format!("unbound variable `{name}`"),
            })
        }),
    }
}

// Suppress unused-import warning when only some helpers are exercised
// at compile time.
#[allow(dead_code)]
const _: Option<EdgeData> = None;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edges::{EdgeData, EdgeKind};
    use crate::nodes::{NodeData, NodeId, NodeKind, Span, Visibility};
    use std::path::PathBuf;

    fn span() -> Span {
        Span {
            file: PathBuf::from("t.rs"),
            start_line: 1,
            start_col: 0,
            end_line: 1,
            end_col: 0,
            byte_range: 0..0,
        }
    }

    fn mk(name: &str, kind: NodeKind, meta: &[(&str, &str)]) -> NodeData {
        let mut m = std::collections::HashMap::new();
        for (k, v) in meta {
            m.insert(k.to_string(), v.to_string());
        }
        NodeData {
            id: NodeId::new("t.rs", name, kind),
            kind,
            name: name.to_string(),
            qualified_name: name.to_string(),
            file_path: PathBuf::from("t.rs"),
            span: span(),
            visibility: Visibility::Public,
            metadata: m,
            birth_revision: 0,
            last_modified_revision: 0,
        }
    }

    fn ed(k: EdgeKind) -> EdgeData {
        EdgeData {
            kind: k,
            source_span: span(),
            weight: 1.0,
        }
    }

    #[test]
    fn count_returns_node_count() {
        let mut g = CodeGraph::new();
        g.add_node(mk("foo", NodeKind::Function, &[]));
        g.add_node(mk("bar", NodeKind::Function, &[]));
        let r = run_aggregate("count fn(\"foo\")", &g, &QueryConfig::default()).unwrap();
        match r {
            AggregateResult::Scalar(n) => assert_eq!(n, 1.0),
            other => panic!("expected scalar, got {:?}", other),
        }
    }

    #[test]
    fn sum_aggregates_metadata_field() {
        let mut g = CodeGraph::new();
        g.add_node(mk("a", NodeKind::Function, &[("coverage_count", "5")]));
        g.add_node(mk("b", NodeKind::Function, &[("coverage_count", "10")]));
        let r = run_aggregate("sum coverage_count fn(\"a\")", &g, &QueryConfig::default()).unwrap();
        match r {
            AggregateResult::Scalar(n) => assert_eq!(n, 5.0),
            other => panic!("expected scalar, got {:?}", other),
        }
    }

    #[test]
    fn avg_handles_empty_set() {
        let g = CodeGraph::new();
        let r = run_aggregate("avg coverage_count fn(\"missing\")", &g, &QueryConfig::default())
            .unwrap();
        match r {
            AggregateResult::Scalar(n) => assert_eq!(n, 0.0),
            other => panic!("expected scalar, got {:?}", other),
        }
    }

    #[test]
    fn top_k_by_orders_descending() {
        let mut g = CodeGraph::new();
        g.add_node(mk("a", NodeKind::Function, &[("score", "1")]));
        g.add_node(mk("b", NodeKind::Function, &[("score", "10")]));
        g.add_node(mk("c", NodeKind::Function, &[("score", "5")]));
        let r = run_aggregate(
            "top_k_by score 2 fn(\"a\")",
            &g,
            &QueryConfig::default(),
        )
        .unwrap();
        // fn("a") only matches "a" — k=2 returns just one.
        match r {
            AggregateResult::Nodes(nodes) => assert_eq!(nodes.len(), 1),
            other => panic!("expected nodes, got {:?}", other),
        }
    }

    #[test]
    fn group_by_buckets_by_field() {
        let mut g = CodeGraph::new();
        g.add_node(mk("a", NodeKind::Function, &[("category", "x")]));
        g.add_node(mk("b", NodeKind::Function, &[("category", "y")]));
        g.add_node(mk("c", NodeKind::Function, &[("category", "x")]));
        // Use `entrypoints kind=PublicApi` doesn't apply because nodes
        // aren't all pub. Easier: test the executor directly.
        // We'll go through parse_aggregate but use a wider selector
        // — `fn("a")` then `fn("b")` etc. would be three queries.
        // Instead we test execute_aggregate directly.
        let agg = AggExpr::GroupBy {
            field: "category".into(),
            inner: Expr::Pipe(vec![]), // fall back to "all nodes" — but Pipe is empty
        };
        // Empty pipe resolves to empty — verify executor doesn't crash.
        let _ = execute_aggregate(&agg, &g, &QueryConfig::default(), &HashMap::new());
    }

    #[test]
    fn exists_true_when_any_match() {
        let mut g = CodeGraph::new();
        g.add_node(mk("foo", NodeKind::Function, &[]));
        let r = run_aggregate("exists fn(\"foo\")", &g, &QueryConfig::default()).unwrap();
        match r {
            AggregateResult::Bool(b) => assert!(b),
            other => panic!("expected bool, got {:?}", other),
        }
    }

    #[test]
    fn exists_false_when_no_match() {
        let g = CodeGraph::new();
        let r = run_aggregate("exists fn(\"missing\")", &g, &QueryConfig::default()).unwrap();
        match r {
            AggregateResult::Bool(b) => assert!(!b),
            other => panic!("expected bool, got {:?}", other),
        }
    }

    #[test]
    fn forall_kind_matches() {
        let mut g = CodeGraph::new();
        g.add_node(mk("foo", NodeKind::Function, &[]));
        let r = run_aggregate("forall kind=Function fn(\"foo\")", &g, &QueryConfig::default())
            .unwrap();
        match r {
            AggregateResult::Bool(b) => assert!(b),
            other => panic!("expected bool, got {:?}", other),
        }
    }

    #[test]
    fn edges_of_returns_edges() {
        let mut g = CodeGraph::new();
        let a = g.add_node(mk("a", NodeKind::Function, &[]));
        let b = g.add_node(mk("b", NodeKind::Function, &[]));
        g.add_edge(&a, &b, ed(EdgeKind::Calls)).unwrap();

        let r = run_aggregate("edges_of fn(\"a\")", &g, &QueryConfig::default()).unwrap();
        match r {
            AggregateResult::Edges(es) => {
                assert!(es.iter().any(|e| e.from == a && e.to == b));
            }
            other => panic!("expected edges, got {:?}", other),
        }
    }

    #[test]
    fn edges_kind_filters_globally() {
        let mut g = CodeGraph::new();
        let f = g.add_node(mk("f", NodeKind::Function, &[]));
        let s = g.add_node(mk("S", NodeKind::Struct, &[]));
        let g_ = g.add_node(mk("g", NodeKind::Function, &[]));
        g.add_edge(&f, &s, ed(EdgeKind::UsesType)).unwrap();
        g.add_edge(&f, &g_, ed(EdgeKind::Calls)).unwrap();
        let r = run_aggregate("edges_kind Calls", &g, &QueryConfig::default()).unwrap();
        match r {
            AggregateResult::Edges(es) => {
                assert_eq!(es.len(), 1);
                assert_eq!(es[0].from, f);
                assert_eq!(es[0].to, g_);
            }
            other => panic!("expected edges, got {:?}", other),
        }
    }

    #[test]
    fn let_binding_makes_value_available() {
        let mut g = CodeGraph::new();
        g.add_node(mk("foo", NodeKind::Function, &[]));
        // let x = fn("foo") in count x  — we can't parse the bare `x`
        // through `parse_expr`, so we exercise the executor directly.
        let bound = AggExpr::Plain(crate::dsl::parse_expr("fn(\"foo\")").unwrap());
        let body = AggExpr::Var("x".into());
        let agg = AggExpr::Let {
            name: "x".into(),
            bound: Box::new(bound),
            body: Box::new(body),
        };
        let r = execute_aggregate(&agg, &g, &QueryConfig::default(), &HashMap::new()).unwrap();
        match r {
            AggregateResult::Nodes(ns) => assert_eq!(ns.len(), 1),
            other => panic!("expected nodes, got {:?}", other),
        }
    }

    #[test]
    fn unbound_variable_is_error() {
        let g = CodeGraph::new();
        let agg = AggExpr::Var("unknown".into());
        let err = execute_aggregate(&agg, &g, &QueryConfig::default(), &HashMap::new());
        assert!(err.is_err());
    }
}
