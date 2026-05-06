//! Domain-specific language for graph queries.
//!
//! Grammar (exactly 8 operators, pipe-separated):
//! ```text
//! query       := op ( '|' op )*
//! op          := fn_select | type_select | callers | callees | depth | filter | show | taint
//! fn_select   := 'fn' '(' STRING ')'
//! type_select := 'type' '(' STRING ')'
//! callers     := 'callers'
//! callees     := 'callees'
//! depth       := 'depth' NUMBER
//! filter      := 'filter' 'kind' '=' IDENT
//! show        := 'show' PROJECTION
//! taint       := 'taint' STRING
//! ```

use std::collections::{HashSet, VecDeque};

use crate::edges::EdgeKind;
use crate::graph::CodeGraph;
use crate::nodes::{NodeId, NodeKind};
use crate::traversal::{self, TraversalDirection, TraversalConfig};

/// Token types produced by the lexer.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Pipe,
    Fn,
    Type,
    Callers,
    Callees,
    Depth,
    Filter,
    Show,
    Taint,
    /// Backward control-flow analysis: walk *incoming* call edges
    /// to enumerate functions that must have called the target.
    /// The companion `extract_predicates` helper additionally
    /// surfaces the enclosing if/match/while predicate at each
    /// call site as edge metadata. Mirrors Magic's "what must have
    /// been true to reach here?" framing — the dual of `taint`.
    Preconditions,
    Kind,
    Equals,
    String(String),
    Number(usize),
    Ident(String),
}

/// Projection mode for the `show` operator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Projection {
    Fields,
    Signature,
    Body,
}

/// DSL operations — 9 variants.
///
/// The original plan capped at 8; v2 added `Preconditions` as a
/// targeted extension for backward control-flow analysis (the dual
/// of `Taint`). Keep the list tight: every new variant adds prompt
/// description bytes the LLM has to read on every request, so the
/// bar for adding a 10th operator is "no existing operator can do
/// this with reasonable composition".
#[derive(Debug, Clone, PartialEq)]
pub enum DslOp {
    SelectFn(String),
    SelectType(String),
    Callers,
    Callees,
    Depth(usize),
    Filter(NodeKind),
    Show(Projection),
    Taint(String),
    /// "What must have been true to reach this call site?" — walks
    /// incoming Calls edges (callers) iteratively up to the configured
    /// depth, with cycle detection. Like `Callers + Depth` but
    /// semantically distinct: indicates the model wants
    /// preconditions-style reasoning, not just a flat caller list.
    /// The renderer pairs this with `extract_predicates` over each
    /// caller's source span to surface the actual enclosing if/match
    /// expression text.
    Preconditions,
}

/// Parse error with position info.
#[derive(Debug, thiserror::Error)]
#[error("parse error at position {position}: {message}")]
pub struct ParseError {
    pub position: usize,
    pub message: String,
}

impl ParseError {
    fn new(position: usize, message: impl Into<String>) -> Self {
        Self {
            position,
            message: message.into(),
        }
    }
}

pub fn lex(input: &str) -> Result<Vec<Token>, ParseError> {
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut pos = 0;
    let mut tokens = Vec::new();

    while pos < len {
        if bytes[pos].is_ascii_whitespace() {
            pos += 1;
            continue;
        }

        match bytes[pos] {
            b'|' => {
                tokens.push(Token::Pipe);
                pos += 1;
            }
            b'=' => {
                tokens.push(Token::Equals);
                pos += 1;
            }
            b'"' => {
                let start = pos;
                pos += 1;
                let content_start = pos;
                while pos < len && bytes[pos] != b'"' {
                    pos += 1;
                }
                if pos >= len {
                    return Err(ParseError::new(start, "unterminated string literal"));
                }
                let content = &input[content_start..pos];
                tokens.push(Token::String(content.to_string()));
                pos += 1;
            }
            b'0'..=b'9' => {
                let start = pos;
                while pos < len && bytes[pos].is_ascii_digit() {
                    pos += 1;
                }
                let num_str = &input[start..pos];
                let num: usize = num_str.parse().map_err(|_| {
                    ParseError::new(start, format!("invalid number: {num_str}"))
                })?;
                tokens.push(Token::Number(num));
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let start = pos;
                while pos < len && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') {
                    pos += 1;
                }
                let word = &input[start..pos];

                match word {
                    "fn" | "type" => {
                        let kw_token = if word == "fn" { Token::Fn } else { Token::Type };
                        while pos < len && bytes[pos].is_ascii_whitespace() {
                            pos += 1;
                        }
                        if pos >= len || bytes[pos] != b'(' {
                            return Err(ParseError::new(
                                pos,
                                format!("expected '(' after '{word}'"),
                            ));
                        }
                        pos += 1;

                        while pos < len && bytes[pos].is_ascii_whitespace() {
                            pos += 1;
                        }

                        if pos >= len || bytes[pos] != b'"' {
                            return Err(ParseError::new(
                                pos,
                                format!("expected string argument for '{word}'"),
                            ));
                        }
                        pos += 1;
                        let content_start = pos;
                        while pos < len && bytes[pos] != b'"' {
                            pos += 1;
                        }
                        if pos >= len {
                            return Err(ParseError::new(
                                content_start - 1,
                                "unterminated string literal",
                            ));
                        }
                        let content = &input[content_start..pos];
                        pos += 1;

                        while pos < len && bytes[pos].is_ascii_whitespace() {
                            pos += 1;
                        }
                        if pos >= len || bytes[pos] != b')' {
                            return Err(ParseError::new(
                                pos,
                                format!("expected ')' after string in '{word}(...)'"),
                            ));
                        }
                        pos += 1;

                        tokens.push(kw_token);
                        tokens.push(Token::String(content.to_string()));
                    }
                    "callers" => tokens.push(Token::Callers),
                    "callees" => tokens.push(Token::Callees),
                    "depth" => tokens.push(Token::Depth),
                    "filter" => tokens.push(Token::Filter),
                    "show" => tokens.push(Token::Show),
                    "taint" => tokens.push(Token::Taint),
                    "preconditions" => tokens.push(Token::Preconditions),
                    "kind" => tokens.push(Token::Kind),
                    _ => tokens.push(Token::Ident(word.to_string())),
                }
            }
            other => {
                return Err(ParseError::new(
                    pos,
                    format!("unexpected character: '{}'", other as char),
                ));
            }
        }
    }

    Ok(tokens)
}

pub fn parse(tokens: &[Token]) -> Result<Vec<DslOp>, ParseError> {
    if tokens.is_empty() {
        return Err(ParseError::new(0, "empty query"));
    }

    let mut ops = Vec::new();
    let mut pos = 0;

    loop {
        if pos >= tokens.len() {
            break;
        }

        let op = parse_op(tokens, &mut pos)?;
        ops.push(op);

        if pos < tokens.len() {
            if tokens[pos] == Token::Pipe {
                pos += 1;
                if pos >= tokens.len() {
                    return Err(ParseError::new(
                        pos,
                        "expected operation after '|'",
                    ));
                }
            } else {
                return Err(ParseError::new(
                    pos,
                    format!(
                        "expected '|' or end of query, found {:?}",
                        tokens[pos]
                    ),
                ));
            }
        }
    }

    if ops.is_empty() {
        return Err(ParseError::new(0, "empty query"));
    }

    Ok(ops)
}

fn parse_op(tokens: &[Token], pos: &mut usize) -> Result<DslOp, ParseError> {
    let token = &tokens[*pos];
    match token {
        Token::Fn => {
            *pos += 1;
            if *pos >= tokens.len() {
                return Err(ParseError::new(*pos, "expected string after 'fn'"));
            }
            match &tokens[*pos] {
                Token::String(s) => {
                    let name = s.clone();
                    *pos += 1;
                    Ok(DslOp::SelectFn(name))
                }
                _ => Err(ParseError::new(*pos, "expected string after 'fn'")),
            }
        }
        Token::Type => {
            *pos += 1;
            if *pos >= tokens.len() {
                return Err(ParseError::new(*pos, "expected string after 'type'"));
            }
            match &tokens[*pos] {
                Token::String(s) => {
                    let name = s.clone();
                    *pos += 1;
                    Ok(DslOp::SelectType(name))
                }
                _ => Err(ParseError::new(*pos, "expected string after 'type'")),
            }
        }
        Token::Callers => {
            *pos += 1;
            Ok(DslOp::Callers)
        }
        Token::Callees => {
            *pos += 1;
            Ok(DslOp::Callees)
        }
        Token::Depth => {
            *pos += 1;
            if *pos >= tokens.len() {
                return Err(ParseError::new(*pos, "expected number after 'depth'"));
            }
            match &tokens[*pos] {
                Token::Number(n) => {
                    let depth = *n;
                    *pos += 1;
                    Ok(DslOp::Depth(depth))
                }
                _ => Err(ParseError::new(*pos, "expected number after 'depth'")),
            }
        }
        Token::Filter => {
            *pos += 1;
            if *pos >= tokens.len() || tokens[*pos] != Token::Kind {
                return Err(ParseError::new(
                    *pos,
                    "expected 'kind' after 'filter'",
                ));
            }
            *pos += 1;
            if *pos >= tokens.len() || tokens[*pos] != Token::Equals {
                return Err(ParseError::new(
                    *pos,
                    "expected '=' after 'filter kind'",
                ));
            }
            *pos += 1;
            if *pos >= tokens.len() {
                return Err(ParseError::new(
                    *pos,
                    "expected node kind after 'filter kind='",
                ));
            }
            let kind = match &tokens[*pos] {
                Token::Ident(s) => parse_node_kind(s, *pos)?,
                _ => {
                    return Err(ParseError::new(
                        *pos,
                        "expected node kind (Function, Struct, Enum, Module, Trait) after 'filter kind='",
                    ));
                }
            };
            *pos += 1;
            Ok(DslOp::Filter(kind))
        }
        Token::Show => {
            *pos += 1;
            if *pos >= tokens.len() {
                return Err(ParseError::new(
                    *pos,
                    "expected projection (fields, signature, body) after 'show'",
                ));
            }
            let projection = match &tokens[*pos] {
                Token::Ident(s) => parse_projection(s, *pos)?,
                _ => {
                    return Err(ParseError::new(
                        *pos,
                        "expected projection (fields, signature, body) after 'show'",
                    ));
                }
            };
            *pos += 1;
            Ok(DslOp::Show(projection))
        }
        Token::Taint => {
            *pos += 1;
            if *pos >= tokens.len() {
                return Err(ParseError::new(*pos, "expected string after 'taint'"));
            }
            match &tokens[*pos] {
                Token::String(s) => {
                    let name = s.clone();
                    *pos += 1;
                    Ok(DslOp::Taint(name))
                }
                _ => Err(ParseError::new(*pos, "expected string after 'taint'")),
            }
        }
        Token::Preconditions => {
            *pos += 1;
            Ok(DslOp::Preconditions)
        }
        Token::Ident(s) => Err(ParseError::new(
            *pos,
            format!(
                "unknown operation '{s}'. Valid operations: fn, type, callers, callees, depth, filter, show, taint, preconditions"
            ),
        )),
        _ => Err(ParseError::new(
            *pos,
            format!(
                "unexpected token {:?}. Expected an operation (fn, type, callers, callees, depth, filter, show, taint, preconditions)",
                token
            ),
        )),
    }
}

fn parse_node_kind(s: &str, pos: usize) -> Result<NodeKind, ParseError> {
    match s {
        "Function" => Ok(NodeKind::Function),
        "Struct" => Ok(NodeKind::Struct),
        "Enum" => Ok(NodeKind::Enum),
        "Module" => Ok(NodeKind::Module),
        "Trait" => Ok(NodeKind::Trait),
        _ => Err(ParseError::new(
            pos,
            format!(
                "unknown node kind '{s}'. Valid kinds: Function, Struct, Enum, Module, Trait"
            ),
        )),
    }
}

fn parse_projection(s: &str, pos: usize) -> Result<Projection, ParseError> {
    match s {
        "fields" => Ok(Projection::Fields),
        "signature" => Ok(Projection::Signature),
        "body" => Ok(Projection::Body),
        _ => Err(ParseError::new(
            pos,
            format!(
                "unknown projection '{s}'. Valid projections: fields, signature, body"
            ),
        )),
    }
}

pub fn parse_query(input: &str) -> Result<Vec<DslOp>, ParseError> {
    let tokens = lex(input)?;
    parse(&tokens)
}

/// Configuration for query execution.
#[derive(Debug, Clone)]
pub struct QueryConfig {
    pub max_tokens: usize,
    pub max_nodes: usize,
}

impl Default for QueryConfig {
    fn default() -> Self {
        Self {
            max_tokens: 4000,
            max_nodes: 50,
        }
    }
}

/// Result of a query execution.
#[derive(Debug, Clone)]
pub struct QueryResult {
    /// Nodes in the result set.
    pub nodes: Vec<NodeId>,
    /// Edges between result nodes: (from, to, edge_kind_description).
    pub edges: Vec<(NodeId, NodeId, String)>,
    /// Whether result was truncated.
    pub was_truncated: bool,
    /// Total nodes before truncation.
    pub total_before_truncation: usize,
    /// Cycles detected during traversal.
    pub cycles_detected: Vec<NodeId>,
}

/// Errors from query execution.
#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    #[error("parse error: {0}")]
    Parse(#[from] ParseError),
    #[error("execution error: {0}")]
    Execution(String),
}

/// Query engine — executes DSL operations against a CodeGraph.
pub struct QueryEngine<'a> {
    graph: &'a CodeGraph,
}

impl<'a> QueryEngine<'a> {
    pub fn new(graph: &'a CodeGraph) -> Self {
        Self { graph }
    }

    /// Execute a parsed query against the graph.
    pub fn execute(&self, ops: &[DslOp], config: &QueryConfig) -> Result<QueryResult, QueryError> {
        let mut working_set: HashSet<NodeId> = HashSet::new();
        let mut cycles_detected: Vec<NodeId> = Vec::new();

        for op in ops {
            match op {
                DslOp::SelectFn(name) => {
                    working_set = self
                        .graph
                        .find_by_name(name)
                        .into_iter()
                        .filter(|n| n.kind == NodeKind::Function)
                        .map(|n| n.id.clone())
                        .collect();
                }
                DslOp::SelectType(name) => {
                    working_set = self
                        .graph
                        .find_by_name(name)
                        .into_iter()
                        .filter(|n| {
                            matches!(n.kind, NodeKind::Struct | NodeKind::Enum | NodeKind::Trait)
                        })
                        .map(|n| n.id.clone())
                        .collect();
                }
                DslOp::Callers => {
                    let mut new_set = HashSet::new();
                    for node_id in &working_set {
                        for (source_id, edge) in self.graph.get_edges_to(node_id) {
                            if matches!(edge.kind, EdgeKind::Calls | EdgeKind::UnresolvedCall(_)) {
                                new_set.insert(source_id.clone());
                            }
                        }
                    }
                    working_set = new_set;
                }
                DslOp::Callees => {
                    let mut new_set = HashSet::new();
                    for node_id in &working_set {
                        for (target_id, edge) in self.graph.get_edges_from(node_id) {
                            if matches!(edge.kind, EdgeKind::Calls | EdgeKind::UnresolvedCall(_)) {
                                new_set.insert(target_id.clone());
                            }
                        }
                    }
                    working_set = new_set;
                }
                DslOp::Depth(n) => {
                    let mut expanded = HashSet::new();
                    for node_id in &working_set {
                        let result = traversal::traverse(
                            self.graph,
                            node_id,
                            &TraversalConfig {
                                max_depth: *n,
                                max_nodes: config.max_nodes,
                                direction: TraversalDirection::Outgoing,
                            },
                        );
                        for id in result.nodes {
                            expanded.insert(id);
                        }
                        cycles_detected.extend(result.cycles_detected_at);
                    }
                    working_set = expanded;
                }
                DslOp::Filter(kind) => {
                    working_set.retain(|id| {
                        self.graph
                            .get_node(id)
                            .map(|n| n.kind == *kind)
                            .unwrap_or(false)
                    });
                }
                DslOp::Show(_) => {}
                DslOp::Preconditions => {
                    // Backward control-flow: BFS over *incoming* call
                    // edges from the working set with cycle detection.
                    // Symmetric to Taint (which BFS's outgoing) — the
                    // working_set after this op is "every function
                    // that, transitively, must have called the
                    // selection". `extract_predicates` enriches with
                    // enclosing if/match conditions when the renderer
                    // pairs the two.
                    let mut reachers = HashSet::new();
                    let mut visited = HashSet::new();
                    let mut queue: VecDeque<NodeId> =
                        working_set.iter().cloned().collect();

                    while let Some(current) = queue.pop_front() {
                        if visited.contains(&current) {
                            cycles_detected.push(current);
                            continue;
                        }
                        visited.insert(current.clone());
                        reachers.insert(current.clone());

                        for (source_id, edge) in self.graph.get_edges_to(&current) {
                            if matches!(
                                edge.kind,
                                EdgeKind::Calls | EdgeKind::UnresolvedCall(_)
                            ) {
                                if visited.contains(source_id) {
                                    cycles_detected.push(source_id.clone());
                                } else {
                                    queue.push_back(source_id.clone());
                                }
                            }
                        }

                        if reachers.len() >= config.max_nodes {
                            break;
                        }
                    }

                    working_set = reachers;
                }
                DslOp::Taint(_var_name) => {
                    // Taint analysis v1: BFS over outgoing call edges from working set
                    // with cycle detection. The var_name is metadata only — full
                    // inter-procedural tracking is out of scope for v1.
                    let mut tainted = HashSet::new();
                    let mut visited = HashSet::new();
                    let mut queue: VecDeque<NodeId> =
                        working_set.iter().cloned().collect();

                    while let Some(current) = queue.pop_front() {
                        if visited.contains(&current) {
                            cycles_detected.push(current);
                            continue;
                        }
                        visited.insert(current.clone());
                        tainted.insert(current.clone());

                        for (target_id, edge) in self.graph.get_edges_from(&current) {
                            if matches!(
                                edge.kind,
                                EdgeKind::Calls | EdgeKind::UnresolvedCall(_)
                            ) {
                                if visited.contains(target_id) {
                                    cycles_detected.push(target_id.clone());
                                } else {
                                    queue.push_back(target_id.clone());
                                }
                            }
                        }

                        if tainted.len() >= config.max_nodes {
                            break;
                        }
                    }

                    working_set = tainted;
                }
            }
        }

        let node_list: Vec<NodeId> = working_set.into_iter().collect();
        let node_set: HashSet<&NodeId> = node_list.iter().collect();
        let mut edges = Vec::new();
        for node_id in &node_list {
            for (target, edge_data) in self.graph.get_edges_from(node_id) {
                if node_set.contains(target) {
                    edges.push((node_id.clone(), target.clone(), format!("{:?}", edge_data.kind)));
                }
            }
        }

        let total = node_list.len();
        let was_truncated = total > config.max_nodes;
        let nodes = if was_truncated {
            node_list[..config.max_nodes].to_vec()
        } else {
            node_list
        };

        Ok(QueryResult {
            nodes,
            edges,
            was_truncated,
            total_before_truncation: total,
            cycles_detected,
        })
    }
}

/// Convenience function: parse and execute a query string.
pub fn run_query(
    query: &str,
    graph: &CodeGraph,
    config: &QueryConfig,
) -> Result<QueryResult, QueryError> {
    let ops = parse_query(query)?;
    let engine = QueryEngine::new(graph);
    engine.execute(&ops, config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dsl_parse_simple() {
        let ops = parse_query(r#"fn("foo") | callees"#).unwrap();
        assert_eq!(ops, vec![DslOp::SelectFn("foo".into()), DslOp::Callees]);
    }

    #[test]
    fn test_dsl_parse_depth() {
        let ops = parse_query(r#"fn("bar") | callees | depth 3"#).unwrap();
        assert_eq!(
            ops,
            vec![
                DslOp::SelectFn("bar".into()),
                DslOp::Callees,
                DslOp::Depth(3),
            ]
        );
    }

    #[test]
    fn test_dsl_parse_full() {
        let ops =
            parse_query(r#"fn("x") | callers | depth 2 | filter kind=Function | show signature"#)
                .unwrap();
        assert_eq!(
            ops,
            vec![
                DslOp::SelectFn("x".into()),
                DslOp::Callers,
                DslOp::Depth(2),
                DslOp::Filter(NodeKind::Function),
                DslOp::Show(Projection::Signature),
            ]
        );
    }

    #[test]
    fn test_dsl_parse_taint() {
        let ops = parse_query(r#"fn("process") | taint "user_input" | depth 5"#).unwrap();
        assert_eq!(
            ops,
            vec![
                DslOp::SelectFn("process".into()),
                DslOp::Taint("user_input".into()),
                DslOp::Depth(5),
            ]
        );
    }

    // Normal: `preconditions` parses to DslOp::Preconditions and
    // composes with selection / depth like any other operator.
    #[test]
    fn test_dsl_parse_preconditions_normal() {
        let ops = parse_query(r#"fn("danger") | preconditions"#).unwrap();
        assert_eq!(
            ops,
            vec![
                DslOp::SelectFn("danger".into()),
                DslOp::Preconditions,
            ]
        );
    }

    // Robust: preconditions chains with depth and filter without
    // parser ambiguity.
    #[test]
    fn test_dsl_parse_preconditions_chain_robust() {
        let ops = parse_query(
            r#"fn("danger") | preconditions | filter kind=Function | depth 3"#,
        )
        .unwrap();
        assert_eq!(
            ops,
            vec![
                DslOp::SelectFn("danger".into()),
                DslOp::Preconditions,
                DslOp::Filter(NodeKind::Function),
                DslOp::Depth(3),
            ]
        );
    }

    // Normal: executor walks incoming Calls edges (callers
    // direction) — symmetric to Taint's outgoing walk. Build a
    // small graph foo→bar→baz, query `fn("baz") | preconditions`,
    // expect both bar and foo (transitive callers) plus baz itself.
    #[test]
    fn test_query_preconditions_walks_callers_normal() {
        use crate::edges::{EdgeData, EdgeKind};
        use crate::graph::CodeGraph;
        use crate::nodes::{NodeData, NodeId, NodeKind, Span, Visibility};
        use std::collections::HashMap;
        use std::path::PathBuf;

        fn span() -> Span {
            Span {
                file: PathBuf::from("t.rs"),
                start_line: 1,
                start_col: 0,
                end_line: 5,
                end_col: 1,
                byte_range: 0..50,
            }
        }
        fn node(name: &str) -> NodeData {
            NodeData {
                id: NodeId::new("t.rs", &format!("crate::{name}"), NodeKind::Function),
                kind: NodeKind::Function,
                name: name.to_string(),
                qualified_name: format!("crate::{name}"),
                file_path: PathBuf::from("t.rs"),
                span: span(),
                visibility: Visibility::Public,
                metadata: HashMap::new(),
            }
        }

        let mut graph = CodeGraph::new();
        let foo = graph.add_node(node("foo"));
        let bar = graph.add_node(node("bar"));
        let baz = graph.add_node(node("baz"));
        let edge = || EdgeData {
            kind: EdgeKind::Calls,
            source_span: span(),
            weight: 1.0,
        };
        graph.add_edge(&foo, &bar, edge()).unwrap();
        graph.add_edge(&bar, &baz, edge()).unwrap();

        let ops = vec![DslOp::SelectFn("baz".into()), DslOp::Preconditions];
        let res = QueryEngine::new(&graph)
            .execute(&ops, &QueryConfig::default())
            .unwrap();
        let names: std::collections::HashSet<&str> = res
            .nodes
            .iter()
            .filter_map(|id| graph.get_node(id).map(|n| n.name.as_str()))
            .collect();
        // baz itself is in the set (BFS starts from working_set
        // and inserts current). Both transitive callers also.
        assert!(names.contains("baz"));
        assert!(names.contains("bar"));
        assert!(names.contains("foo"));
    }

    // Robust: a mutual-recursion graph (ping↔pong) terminates
    // with cycles_detected populated, doesn't infinite-loop.
    #[test]
    fn test_query_preconditions_terminates_on_cycle_robust() {
        use crate::edges::{EdgeData, EdgeKind};
        use crate::graph::CodeGraph;
        use crate::nodes::{NodeData, NodeId, NodeKind, Span, Visibility};
        use std::collections::HashMap;
        use std::path::PathBuf;

        fn span() -> Span {
            Span {
                file: PathBuf::from("t.rs"),
                start_line: 1,
                start_col: 0,
                end_line: 5,
                end_col: 1,
                byte_range: 0..50,
            }
        }
        fn node(name: &str) -> NodeData {
            NodeData {
                id: NodeId::new("t.rs", &format!("crate::{name}"), NodeKind::Function),
                kind: NodeKind::Function,
                name: name.to_string(),
                qualified_name: format!("crate::{name}"),
                file_path: PathBuf::from("t.rs"),
                span: span(),
                visibility: Visibility::Public,
                metadata: HashMap::new(),
            }
        }
        let mut graph = CodeGraph::new();
        let ping = graph.add_node(node("ping"));
        let pong = graph.add_node(node("pong"));
        let edge = || EdgeData {
            kind: EdgeKind::Calls,
            source_span: span(),
            weight: 1.0,
        };
        graph.add_edge(&ping, &pong, edge()).unwrap();
        graph.add_edge(&pong, &ping, edge()).unwrap();

        let ops = vec![DslOp::SelectFn("ping".into()), DslOp::Preconditions];
        let res = QueryEngine::new(&graph)
            .execute(&ops, &QueryConfig::default())
            .unwrap();
        assert!(res.cycles_detected.iter().any(|id| *id == ping || *id == pong));
        assert!(res.nodes.len() <= 2);
    }

    #[test]
    fn test_dsl_parse_type() {
        let ops = parse_query(r#"type("Config") | callees"#).unwrap();
        assert_eq!(
            ops,
            vec![DslOp::SelectType("Config".into()), DslOp::Callees]
        );
    }

    #[test]
    fn test_dsl_parse_error_empty() {
        let err = parse_query("").unwrap_err();
        assert_eq!(err.position, 0);
        assert!(err.message.contains("empty"));
    }

    #[test]
    fn test_dsl_parse_error_invalid_op() {
        let err = parse_query(r#"fn("x") | invalid_op"#).unwrap_err();
        assert!(err.position > 0);
        assert!(err.message.contains("unknown operation"));
        assert!(err.message.contains("invalid_op"));
    }

    #[test]
    fn test_dsl_parse_error_missing_string() {
        let err = parse_query(r#"fn() | callees"#).unwrap_err();
        assert!(err.position > 0);
        assert!(err.message.contains("string"));
    }

    use std::path::Path;

    use crate::adapter::rust::RustAdapter;
    use crate::builder::GraphBuilder;

    fn build_sample_graph() -> CodeGraph {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let adapter = RustAdapter::new();
        GraphBuilder::build_from_files(&[fixtures.join("sample.rs")], &adapter)
    }

    fn build_mutual_recursion_graph() -> CodeGraph {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let adapter = RustAdapter::new();
        GraphBuilder::build_from_files(&[fixtures.join("mutual_recursion.rs")], &adapter)
    }

    fn node_names(graph: &CodeGraph, ids: &[NodeId]) -> Vec<String> {
        ids.iter()
            .filter_map(|id| graph.get_node(id).map(|n| n.name.clone()))
            .collect()
    }

    #[test]
    fn test_query_fn_callees() {
        let graph = build_sample_graph();
        let config = QueryConfig::default();
        let result = run_query(r#"fn("foo") | callees"#, &graph, &config).unwrap();

        let names = node_names(&graph, &result.nodes);
        assert!(
            names.contains(&"bar".to_string()),
            "expected 'bar' in callees of foo, got: {names:?}"
        );
    }

    #[test]
    fn test_query_fn_callers() {
        let graph = build_sample_graph();
        let config = QueryConfig::default();
        let result = run_query(r#"fn("baz") | callers"#, &graph, &config).unwrap();

        let names = node_names(&graph, &result.nodes);
        assert!(
            names.contains(&"bar".to_string()),
            "expected 'bar' in callers of baz, got: {names:?}"
        );
    }

    #[test]
    fn test_query_depth() {
        let graph = build_sample_graph();
        let config = QueryConfig::default();
        let result = run_query(r#"fn("foo") | callees | depth 2"#, &graph, &config).unwrap();

        let names = node_names(&graph, &result.nodes);
        assert!(
            names.contains(&"bar".to_string()),
            "expected 'bar' in depth-2 from foo's callees, got: {names:?}"
        );
        assert!(
            names.contains(&"baz".to_string()),
            "expected 'baz' in depth-2 from foo's callees, got: {names:?}"
        );
    }

    #[test]
    fn test_query_filter() {
        let graph = build_sample_graph();
        let config = QueryConfig::default();
        let result = run_query(
            r#"fn("foo") | callees | depth 3 | filter kind=Function"#,
            &graph,
            &config,
        )
        .unwrap();

        for id in &result.nodes {
            let node = graph.get_node(id).unwrap();
            assert_eq!(
                node.kind,
                NodeKind::Function,
                "expected only Function nodes after filter, got {:?} for '{}'",
                node.kind,
                node.name
            );
        }
    }

    #[test]
    fn test_query_type_select() {
        let graph = build_sample_graph();
        let config = QueryConfig::default();
        let result = run_query(r#"type("Config")"#, &graph, &config).unwrap();

        let names = node_names(&graph, &result.nodes);
        assert!(
            names.contains(&"Config".to_string()),
            "expected 'Config' in type select result, got: {names:?}"
        );
        assert!(!result.nodes.is_empty());
    }

    #[test]
    fn test_query_cycle_safe() {
        let graph = build_mutual_recursion_graph();
        let config = QueryConfig::default();
        let result = run_query(r#"fn("ping") | callees | depth 10"#, &graph, &config).unwrap();

        let names = node_names(&graph, &result.nodes);
        assert!(
            names.contains(&"pong".to_string()),
            "expected 'pong' reachable from ping, got: {names:?}"
        );
        assert!(
            !result.cycles_detected.is_empty(),
            "expected cycle detection in mutual recursion"
        );
    }

    #[test]
    fn test_query_max_nodes() {
        let graph = build_sample_graph();
        let config = QueryConfig {
            max_tokens: 4000,
            max_nodes: 2,
        };
        let result = run_query(r#"fn("foo") | callees | depth 5"#, &graph, &config).unwrap();

        if result.total_before_truncation > 2 {
            assert!(result.was_truncated);
            assert!(result.nodes.len() <= 2);
        }
    }

    fn build_deep_call_chain_graph() -> CodeGraph {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let adapter = RustAdapter::new();
        GraphBuilder::build_from_files(&[fixtures.join("deep_call_chain.rs")], &adapter)
    }

    #[test]
    fn test_taint_basic() {
        let graph = build_deep_call_chain_graph();
        let config = QueryConfig {
            max_tokens: 4000,
            max_nodes: 50,
        };
        let result = run_query(r#"fn("a") | taint "x""#, &graph, &config).unwrap();

        let names = node_names(&graph, &result.nodes);
        let expected = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];
        for name in &expected {
            assert!(
                names.contains(&name.to_string()),
                "expected '{name}' in taint result, got: {names:?}"
            );
        }
        assert_eq!(result.nodes.len(), 10);
    }

    #[test]
    fn test_taint_cycle_safe() {
        let graph = build_mutual_recursion_graph();
        let config = QueryConfig::default();
        let result = run_query(r#"fn("ping") | taint "n""#, &graph, &config).unwrap();

        let names = node_names(&graph, &result.nodes);
        assert!(
            names.contains(&"ping".to_string()),
            "expected 'ping' in taint result, got: {names:?}"
        );
        assert!(
            names.contains(&"pong".to_string()),
            "expected 'pong' in taint result, got: {names:?}"
        );
        assert!(
            !result.cycles_detected.is_empty(),
            "expected cycle detection in mutual recursion taint"
        );
    }

    #[test]
    fn test_taint_respects_max_nodes() {
        let graph = build_deep_call_chain_graph();
        let config = QueryConfig {
            max_tokens: 4000,
            max_nodes: 3,
        };
        let result = run_query(r#"fn("a") | taint "x""#, &graph, &config).unwrap();

        assert!(
            result.nodes.len() <= 3,
            "expected at most 3 nodes with max_nodes=3, got: {}",
            result.nodes.len()
        );
    }
}
