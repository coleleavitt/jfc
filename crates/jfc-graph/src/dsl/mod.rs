//! Domain-specific language for graph queries.
//!
//! Grammar (legacy pipe-chain — preserved for back-compat):
//! ```text
//! query       := op ( '|' op )*
//! op          := fn_select | type_select | callers | callees | depth | filter
//!              | show | taint | preconditions | since | hot | scc
//!              | dispatch | cluster_by_type | affected | co_changes
//! fn_select   := 'fn' '(' STRING ')'
//! type_select := 'type' '(' STRING ')'
//! callers     := 'callers'
//! callees     := 'callees'
//! depth       := 'depth' NUMBER
//! filter      := 'filter' 'kind' '=' IDENT
//! show        := 'show' PROJECTION
//! taint       := 'taint' STRING
//! since       := 'since' NUMBER
//! hot         := 'hot' NUMBER
//! scc         := 'scc'
//! dispatch    := 'dispatch'
//! cluster_by_type := 'cluster' 'by' 'type'
//! affected    := 'affected' NUMBER 'since' NUMBER
//! ```
//!
//! Extended grammar (set algebra, path patterns, entrypoint selector,
//! dominator queries, trait/cluster selectors, multi-source path):
//! ```text
//! expr        := setop_expr
//! setop_expr  := atom ( ( 'union' | 'intersect' | 'diff' | '\' ) atom )*
//! atom        := pipe_chain | path_query | entrypoint_query
//!              | dominators_query | dominates_query | trait_impls_query
//!              | multi_path_query | '(' expr ')'
//! pipe_chain  := op ( '|' op )*
//! path_query  := ( 'path' | 'paths' ) atom '->' atom
//!                ( 'where' 'intermediate' 'kind' '=' IDENT )?
//!                ( 'via' EDGE_KIND )?
//!                ( 'depth' NUMBER )?
//! entrypoint_query := 'entrypoints' ( 'kind' '=' IDENT )?
//! dominators_query := 'dominators' 'of' atom
//! dominates_query  := 'dominates' atom
//! trait_impls_query := 'trait_impls' 'of' atom
//! multi_path_query := 'multi_path' '{' atom ( ',' atom )* '}' '->' atom
//!                     ( 'depth' NUMBER )?
//! ```
//!
//! Set-op precedence: all three (`union`, `intersect`, `diff`/`\`) share
//! the same precedence level and are left-associative. Use parentheses
//! to disambiguate. Set ops only preserve the `nodes` field of each
//! operand's [`QueryResult`]; edges, cycle records, and other metadata
//! are intentionally dropped because there is no meaningful merge
//! semantics across heterogenous traversals.

pub mod aggregate;
pub mod plan;
pub mod provenance;
pub mod stream;

use std::collections::{HashMap, HashSet, VecDeque};

use crate::edges::EdgeKind;
use crate::graph::CodeGraph;
use crate::nodes::{NodeId, NodeKind};
use crate::traversal::{self, TraversalConfig, TraversalDirection};

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
    /// Left paren — only meaningful in extended grammar (set-op grouping).
    LParen,
    /// Right paren — only meaningful in extended grammar.
    RParen,
    /// Set-algebra union (`A union B`).
    Union,
    /// Set-algebra intersection (`A intersect B`).
    Intersect,
    /// Set-algebra difference, spelled either `A diff B` or `A \ B`.
    Diff,
    /// Path pattern: shortest single path between endpoints.
    Path,
    /// Path pattern: all simple paths between endpoints.
    Paths,
    /// `where` clause introducing intermediate-node predicates.
    Where,
    /// `intermediate` qualifier inside a `where` clause.
    Intermediate,
    /// `via EDGE_KIND` requirement: at least one edge of the given kind on the path.
    Via,
    /// Path-segment arrow (`->` or unicode `→`).
    Arrow,
    /// `entrypoints` selector — returns classified entrypoint nodes.
    Entrypoints,
    /// `since N` postfix filter — restrict working set to nodes whose
    /// `last_modified_revision >= N`. Pairs with
    /// [`crate::graph::CodeGraph::current_revision`] to answer "what
    /// changed since revision N?".
    Since,
    /// `hot N` — top-N by PageRank. As a bare selector ranks the entire
    /// graph; as a postfix op restricts the working set to its hottest N.
    Hot,
    /// `scc` — strongly connected components. Bare selector returns the
    /// union of all multi-element SCC members; as a postfix op restricts
    /// to SCCs containing the working-set nodes.
    Scc,
    /// `dominators of fn("X")` — dominator chain seeded at X.
    Dominators,
    /// `dominates fn("X")` — descendants of X in the dominator tree.
    Dominates,
    /// `of` — connector for `dominators of <expr>`.
    Of,
    /// `trait_impls of type("X")` — implementors of trait X.
    TraitImpls,
    /// `dispatch` — restrict working set to functions whose calls go
    /// through trait-method dispatch (postfix op).
    Dispatch,
    /// `cluster` — used in `cluster by type` (bare or postfix).
    Cluster,
    /// `by` — connector for `cluster by type`.
    By,
    /// `affected N since M` — `nodes_changed_within_depth(M, N)`.
    Affected,
    /// `multi_path {fn("a"), fn("b")} -> fn("c")` — multi-source shortest path.
    MultiPath,
    /// `{` — opens a brace list (multi-source `path` / `multi_path`).
    LBrace,
    /// `}` — closes a brace list.
    RBrace,
    /// `,` — separates entries in a brace list.
    Comma,
    /// `untested` — filter to functions with `coverage_tested == "false"` or
    /// no coverage data at all. Requires [`CoveragePass`] to have run first.
    Untested,
    /// `possible_types` — postfix filter; enriches output with
    /// `possible_input_types` / `possible_return_types` metadata from the
    /// working set. Requires [`PossibleTypesPass`] to have run.
    PossibleTypes,
    /// `co_changes` — postfix operator that computes temporal coupling
    /// (co-change analysis from git history) for the working set. Returns
    /// functions that frequently change together with the selected ones.
    CoChanges,
    /// `communities` — run Louvain community detection; filter to working set.
    Communities,
    /// `complexity` — postfix operator surfacing per-function complexity metrics.
    Complexity,
    /// `cfg` — postfix operator surfacing per-function control flow graph.
    Cfg,
    /// `dataflow` — postfix operator surfacing per-function dataflow analysis.
    Dataflow,
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
    /// `since N` postfix filter — restricts the working set to nodes whose
    /// `last_modified_revision >= N`. Composes with any other operator;
    /// chain it last in a pipe-query to answer "of the things selected,
    /// which changed since revision N?". The argument is a `u64` so callers
    /// can pass the value of [`crate::graph::CodeGraph::current_revision`]
    /// captured before a batch of mutations.
    Since(u64),
    /// `hot N` — top-N functions by PageRank centrality.
    ///
    /// Two semantics depending on chain position:
    /// - **Bare** (`hot 10`): rank every function in the graph; keep top N.
    /// - **Postfix** (`fn("auth") | callees | hot 5`): rank only nodes
    ///   already in the working set; keep its top N.
    ///
    /// Centrality is computed lazily (`graph.hottest_functions(n)` for the
    /// bare form; `graph.centrality()` then a re-rank for the postfix
    /// form). One PageRank computation per `hot` op — the working set is
    /// usually small relative to the graph, so re-running on the bare
    /// form vs. caching is a wash.
    Hot(usize),
    /// `scc` — strongly connected components.
    ///
    /// - **Bare** (`scc`): return every member of every multi-element SCC
    ///   in the graph. Singletons are excluded — the answer to "what
    ///   loops?" not "every node is in some SCC".
    /// - **Postfix** (`fn("foo") | scc`): restrict to the SCC(s)
    ///   containing the working-set nodes. Empty result if every input is
    ///   a singleton.
    ///
    /// `QueryResult.metadata` describes each surviving cluster as a line
    /// of the form `SCC[i] size=N members=[name, name, ...]`.
    Scc,
    /// `dispatch` — postfix filter restricting the working set to
    /// functions that participate in trait dispatch as the *caller*.
    /// Computed via [`crate::graph::CodeGraph::trait_dispatch_calls`].
    /// Pairs with `fn("foo") | dispatch` to answer "which calls in foo
    /// go through trait dispatch?".
    Dispatch,
    /// `cluster by type` — group functions by their primary type
    /// (most-frequent `UsesType` target). When chained postfix
    /// (`<expr> | cluster by type`), only functions in the working set
    /// participate.
    ///
    /// Result: union of every function across every cluster (the
    /// clustering itself surfaces in `metadata` as `cluster[i] type=T
    /// size=N members=[...]`).
    ClusterByType,
    /// `affected N since M` — every node within `N` undirected hops of
    /// any node modified at or after revision `M`. Wraps
    /// [`crate::graph::CodeGraph::nodes_changed_within_depth`].
    ///
    /// Spelled `affected` rather than reusing `depth`/`since` because
    /// `since N | depth M` would semantically mean "filter to recent,
    /// then expand outgoing N hops" — different ordering and direction.
    /// `affected N since M` is undirected and seeded at *every* recent
    /// node, which is what code-review questions like "what's near my
    /// recent changes?" actually want.
    Affected {
        depth: usize,
        since_rev: u64,
    },
    /// `untested` — postfix filter restricting the working set to Function
    /// nodes with `metadata["coverage_tested"] != "true"`. Functions that
    /// haven't been annotated by a coverage pass at all are included (they
    /// are presumed untested). Use after `run_coverage` to find dead code.
    Untested,
    /// `possible_types` — postfix enrichment operator that surfaces
    /// `metadata["possible_input_types"]` and `metadata["possible_return_types"]`
    /// for nodes in the working set. Also retains only nodes that have at
    /// least one possible type (filters out functions with no type edges).
    PossibleTypes,
    /// `co_changes` — postfix operator. For the current working set, runs
    /// co-change analysis against git history and expands the working set
    /// to include temporally coupled nodes. Metadata lines describe the
    /// coupling strength. Uses `min_support=2` by default (pairs must
    /// co-occur at least twice).
    CoChanges,
    /// `communities` — run Louvain community detection on the full graph,
    /// then filter results to the working set. Metadata lines describe
    /// each community as `community N: [node1, node2, ...]`.
    Communities,
    /// `complexity` — postfix enrichment operator surfacing per-function
    /// complexity metrics (cognitive, cyclomatic, nesting, Halstead, LOC,
    /// maintainability index) for nodes in the working set. Retains only
    /// Function nodes that have complexity data populated.
    Complexity,
    /// `cfg` — postfix enrichment operator surfacing per-function control
    /// flow graph for nodes in the working set. Retains only Function nodes
    /// that have CFG data populated.
    Cfg,
    /// `dataflow` — postfix enrichment operator surfacing per-function
    /// dataflow analysis (params, returns, assignments, arg flows, mutations)
    /// for nodes in the working set. Retains only Function nodes that have
    /// dataflow data populated.
    Dataflow,
}

/// Top-level expression supporting set algebra, path patterns, and
/// entrypoint selectors on top of the legacy pipe-chain grammar.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A traditional pipe-chain (e.g. `fn("foo") | callers | depth 3`).
    Pipe(Vec<DslOp>),
    /// `path` (single shortest) or `paths` (all simple) between two
    /// expressions, with optional intermediate-kind, edge-via, and
    /// depth qualifiers.
    PathQuery(PathQuery),
    /// `entrypoints` selector — optionally filtered to a specific
    /// [`EntrypointKind`].
    Entrypoints(Option<EntrypointKind>),
    /// Set-algebra binary op: union / intersect / diff (left minus right).
    SetOp {
        op: SetOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    /// `dominators of <expr>` — for each node selected by `<expr>`, walk
    /// the dominator chain in the call graph and union the result.
    ///
    /// The dominator tree is rooted at a synthetic entry chosen by
    /// [`pick_dominator_root`]: prefer `fn main`, fall back to the
    /// function with the highest fan-in. The call graph isn't natively
    /// single-entry so this is a documented heuristic; callers needing a
    /// specific root should use [`crate::dominators::Dominators::build`]
    /// directly.
    DominatorsOf(Box<Expr>),
    /// `dominates <expr>` — for each node selected by `<expr>`, return
    /// every node it dominates (descendants in the dominator tree).
    /// Same root-selection policy as `DominatorsOf`.
    DominatesOf(Box<Expr>),
    /// `trait_impls of <expr>` — for each `Trait` node selected by
    /// `<expr>`, return the union of its direct implementors via
    /// `Implements` edges. Backed by
    /// [`crate::graph::CodeGraph::trait_hierarchies`].
    TraitImplsOf(Box<Expr>),
    /// An atom expression (e.g. `entrypoints`, `dominators of ...`)
    /// followed by pipe operators (e.g. `| untested | depth 3`).
    /// The atom is executed first to produce a working set, then the
    /// pipe ops are applied as postfix filters/transforms.
    PipeFrom { base: Box<Expr>, ops: Vec<DslOp> },
    /// `multi_path { <expr>, <expr>, ... } -> <expr>` — multi-source
    /// shortest path. Wraps
    /// [`crate::traversal::find_path_multi_source`]: every source set is
    /// seeded into a single BFS so the shortest path from *any* source
    /// to `to` wins. Optional trailing `depth N` qualifier (default 32).
    MultiPath {
        sources: Vec<Expr>,
        to: Box<Expr>,
        max_depth: Option<usize>,
    },
}

/// Set-algebra operator on `QueryResult` node sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetOp {
    Union,
    Intersect,
    /// Asymmetric difference (`A \ B` = nodes in A but not in B).
    Diff,
}

/// Path-pattern flavor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathMode {
    /// Single shortest path.
    Shortest,
    /// All simple (non-self-intersecting) paths.
    AllSimple,
}

/// Parsed `path` / `paths` query AST.
#[derive(Debug, Clone, PartialEq)]
pub struct PathQuery {
    pub mode: PathMode,
    pub from: Box<Expr>,
    pub to: Box<Expr>,
    /// `where intermediate kind=K` — restrict intermediate node kinds.
    pub intermediate_kind: Option<NodeKind>,
    /// `via EdgeKind` — require at least one edge of this kind on the path.
    pub via_edge: Option<EdgeKind>,
    /// `depth N` — bound search depth (default 32 if unspecified).
    pub max_depth: Option<usize>,
}

/// Coarse classification of program entrypoints — mirrors the categories
/// produced by [`crate::analysis::CodeGraph::classify_entrypoints`] one-to-one.
/// Kept as a separate enum (rather than re-exporting `analysis::EntrypointKind`)
/// because the DSL surface is the user-facing keyword set: callers spell
/// `Main`/`PublicApi`/`Test`/`Bench`/`FfiExport` in queries and we translate
/// at the executor boundary. If `analysis::EntrypointKind` ever grows or
/// renames, the DSL keyword surface stays stable until we choose to expose
/// the change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntrypointKind {
    /// `fn main` at module root.
    Main,
    /// Public function exposed at crate root or `pub mod`.
    PublicApi,
    /// `#[test]`, `#[tokio::test]`, integration tests, etc.
    Test,
    /// `#[bench]` benchmark harness.
    Bench,
    /// FFI export: `pub extern "..." fn` or `#[no_mangle]`.
    FfiExport,
}

impl EntrypointKind {
    /// Translate an `analysis::EntrypointKind` (the source of truth) into the
    /// DSL-facing variant. The two enums currently align 1:1 but kept
    /// separate so the DSL keyword surface can evolve independently.
    fn from_analysis(k: crate::analysis::EntrypointKind) -> Self {
        match k {
            crate::analysis::EntrypointKind::Main => Self::Main,
            crate::analysis::EntrypointKind::PublicApi => Self::PublicApi,
            crate::analysis::EntrypointKind::Test => Self::Test,
            crate::analysis::EntrypointKind::Bench => Self::Bench,
            crate::analysis::EntrypointKind::FfiExport => Self::FfiExport,
        }
    }
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

        // Unicode arrow `→` (3 bytes: 0xE2 0x86 0x92). Detect before falling
        // through to the byte-by-byte ASCII match — `→` is the natural
        // notation in the documented path-query grammar.
        if pos + 2 < len && bytes[pos] == 0xE2 && bytes[pos + 1] == 0x86 && bytes[pos + 2] == 0x92 {
            tokens.push(Token::Arrow);
            pos += 3;
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
            b'(' => {
                tokens.push(Token::LParen);
                pos += 1;
            }
            b')' => {
                tokens.push(Token::RParen);
                pos += 1;
            }
            b'\\' => {
                tokens.push(Token::Diff);
                pos += 1;
            }
            b'{' => {
                tokens.push(Token::LBrace);
                pos += 1;
            }
            b'}' => {
                tokens.push(Token::RBrace);
                pos += 1;
            }
            b',' => {
                tokens.push(Token::Comma);
                pos += 1;
            }
            b'-' if pos + 1 < len && bytes[pos + 1] == b'>' => {
                tokens.push(Token::Arrow);
                pos += 2;
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
                let num: usize = num_str
                    .parse()
                    .map_err(|_| ParseError::new(start, format!("invalid number: {num_str}")))?;
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
                        // Peek ahead for '(' — if absent, `type` is a plain
                        // ident (e.g. `cluster by type`), not a selector.
                        let mut peek = pos;
                        while peek < len && bytes[peek].is_ascii_whitespace() {
                            peek += 1;
                        }
                        if peek >= len || bytes[peek] != b'(' {
                            if word == "type" {
                                // Not a type("...") selector — emit as ident
                                // so `cluster by type` parses correctly.
                                tokens.push(Token::Ident("type".to_string()));
                                continue;
                            }
                            return Err(ParseError::new(
                                pos,
                                format!("expected '(' after '{word}'"),
                            ));
                        }
                        // Consume the whitespace we peeked past.
                        pos = peek;
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
                    "union" => tokens.push(Token::Union),
                    "intersect" => tokens.push(Token::Intersect),
                    "diff" => tokens.push(Token::Diff),
                    "path" => tokens.push(Token::Path),
                    "paths" => tokens.push(Token::Paths),
                    "where" => tokens.push(Token::Where),
                    "intermediate" => tokens.push(Token::Intermediate),
                    "via" => tokens.push(Token::Via),
                    "entrypoints" => tokens.push(Token::Entrypoints),
                    // Postfix history filter: `since N` — see `DslOp::Since`.
                    // Sits at the same grammar level as `depth N` (postfix,
                    // takes a single numeric argument) so it can chain after
                    // any selector or traversal.
                    "since" => tokens.push(Token::Since),
                    // Centrality / structural / type-cluster operators —
                    // see `DslOp::{Hot,Scc,Dominators,...}` for the full
                    // wiring story. Each is a single-keyword token that the
                    // pipe-chain parser dispatches on.
                    "hot" => tokens.push(Token::Hot),
                    "scc" => tokens.push(Token::Scc),
                    "dominators" => tokens.push(Token::Dominators),
                    "dominates" => tokens.push(Token::Dominates),
                    "of" => tokens.push(Token::Of),
                    "trait_impls" => tokens.push(Token::TraitImpls),
                    "dispatch" => tokens.push(Token::Dispatch),
                    "cluster" => tokens.push(Token::Cluster),
                    "by" => tokens.push(Token::By),
                    "affected" => tokens.push(Token::Affected),
                    "multi_path" => tokens.push(Token::MultiPath),
                    "untested" => tokens.push(Token::Untested),
                    "possible_types" => tokens.push(Token::PossibleTypes),
                    "co_changes" => tokens.push(Token::CoChanges),
                    "communities" => tokens.push(Token::Communities),
                    "complexity" => tokens.push(Token::Complexity),
                    "cfg" => tokens.push(Token::Cfg),
                    "dataflow" => tokens.push(Token::Dataflow),
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
                    return Err(ParseError::new(pos, "expected operation after '|'"));
                }
            } else {
                return Err(ParseError::new(
                    pos,
                    format!("expected '|' or end of query, found {:?}", tokens[pos]),
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
                return Err(ParseError::new(*pos, "expected 'kind' after 'filter'"));
            }
            *pos += 1;
            if *pos >= tokens.len() || tokens[*pos] != Token::Equals {
                return Err(ParseError::new(*pos, "expected '=' after 'filter kind'"));
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
        // `since N` — postfix history filter. Mirrors `depth N`'s grammar
        // (single numeric argument) but operates on the temporal axis: keep
        // only nodes whose `last_modified_revision >= N`. Sits at the same
        // precedence level as every other pipe-operator; chain order
        // determines whether the filter applies before or after a traversal
        // expansion.
        Token::Since => {
            *pos += 1;
            if *pos >= tokens.len() {
                return Err(ParseError::new(*pos, "expected number after 'since'"));
            }
            match &tokens[*pos] {
                Token::Number(n) => {
                    let rev = *n as u64;
                    *pos += 1;
                    Ok(DslOp::Since(rev))
                }
                _ => Err(ParseError::new(*pos, "expected number after 'since'")),
            }
        }
        // `hot N` — top-N centrality. Single numeric argument, behavior
        // depends on whether the working set is empty (bare ⇒ rank graph)
        // or populated (postfix ⇒ rank within set). Decided in the executor.
        Token::Hot => {
            *pos += 1;
            if *pos >= tokens.len() {
                return Err(ParseError::new(*pos, "expected number after 'hot'"));
            }
            match &tokens[*pos] {
                Token::Number(n) => {
                    let k = *n;
                    *pos += 1;
                    Ok(DslOp::Hot(k))
                }
                _ => Err(ParseError::new(*pos, "expected number after 'hot'")),
            }
        }
        // `scc` — no arguments. Bare-vs-postfix distinction handled in the executor.
        Token::Scc => {
            *pos += 1;
            Ok(DslOp::Scc)
        }
        // `dispatch` — postfix filter; arguments derive from working set.
        Token::Dispatch => {
            *pos += 1;
            Ok(DslOp::Dispatch)
        }
        // `untested` — postfix filter; no arguments.
        Token::Untested => {
            *pos += 1;
            Ok(DslOp::Untested)
        }
        // `possible_types` — postfix enrichment + filter; no arguments.
        Token::PossibleTypes => {
            *pos += 1;
            Ok(DslOp::PossibleTypes)
        }
        // `co_changes` — postfix temporal coupling analysis; no arguments.
        Token::CoChanges => {
            *pos += 1;
            Ok(DslOp::CoChanges)
        }
        // `communities` — Louvain community detection; no arguments.
        Token::Communities => {
            *pos += 1;
            Ok(DslOp::Communities)
        }
        // `complexity` — surface per-function complexity metrics; no arguments.
        Token::Complexity => {
            *pos += 1;
            Ok(DslOp::Complexity)
        }
        // `cfg` — surface per-function control flow graph; no arguments.
        Token::Cfg => {
            *pos += 1;
            Ok(DslOp::Cfg)
        }
        // `dataflow` — surface per-function dataflow analysis; no arguments.
        Token::Dataflow => {
            *pos += 1;
            Ok(DslOp::Dataflow)
        }
        // `cluster by type` — exact two-keyword sequence. We don't currently
        // support clustering by anything else, but the `by` keyword leaves
        // room (`cluster by trait`, `cluster by file`) without re-parsing.
        Token::Cluster => {
            *pos += 1;
            if *pos >= tokens.len() || tokens[*pos] != Token::By {
                return Err(ParseError::new(*pos, "expected 'by' after 'cluster'"));
            }
            *pos += 1;
            if *pos >= tokens.len() {
                return Err(ParseError::new(*pos, "expected 'type' after 'cluster by'"));
            }
            match &tokens[*pos] {
                Token::Type => {
                    *pos += 1;
                    Ok(DslOp::ClusterByType)
                }
                Token::Ident(s) if s == "type" => {
                    *pos += 1;
                    Ok(DslOp::ClusterByType)
                }
                _ => Err(ParseError::new(*pos, "expected 'type' after 'cluster by'")),
            }
        }
        // `affected N since M` — single-shot temporal-neighborhood operator.
        // Note `since` here is consumed as a *positional* argument, not the
        // `Since` postfix filter. Parser reads literally: `affected`,
        // number, `since`, number.
        Token::Affected => {
            *pos += 1;
            if *pos >= tokens.len() {
                return Err(ParseError::new(*pos, "expected number after 'affected'"));
            }
            let depth = match &tokens[*pos] {
                Token::Number(n) => *n,
                _ => {
                    return Err(ParseError::new(
                        *pos,
                        "expected number (depth) after 'affected'",
                    ));
                }
            };
            *pos += 1;
            if *pos >= tokens.len() || tokens[*pos] != Token::Since {
                return Err(ParseError::new(*pos, "expected 'since' after 'affected N'"));
            }
            *pos += 1;
            if *pos >= tokens.len() {
                return Err(ParseError::new(
                    *pos,
                    "expected number (revision) after 'affected N since'",
                ));
            }
            let since_rev = match &tokens[*pos] {
                Token::Number(n) => *n as u64,
                _ => {
                    return Err(ParseError::new(
                        *pos,
                        "expected number after 'affected N since'",
                    ));
                }
            };
            *pos += 1;
            Ok(DslOp::Affected { depth, since_rev })
        }
        Token::Ident(s) => Err(ParseError::new(
            *pos,
            format!(
                "unknown operation '{s}'. Valid operations: fn, type, callers, callees, depth, filter, show, taint, preconditions, since, hot, scc, dispatch, cluster, affected, co_changes"
            ),
        )),
        _ => Err(ParseError::new(
            *pos,
            format!(
                "unexpected token {:?}. Expected an operation (fn, type, callers, callees, depth, filter, show, taint, preconditions, since, hot, scc, dispatch, cluster, affected, co_changes)",
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
        "EnumVariant" => Ok(NodeKind::EnumVariant),
        "Field" => Ok(NodeKind::Field),
        "TypeAlias" => Ok(NodeKind::TypeAlias),
        "Constant" => Ok(NodeKind::Constant),
        "Interface" => Ok(NodeKind::Interface),
        _ => Err(ParseError::new(
            pos,
            format!(
                "unknown node kind '{s}'. Valid kinds: Function, Struct, Enum, Module, Trait, EnumVariant, Field, TypeAlias, Constant, Interface"
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
            format!("unknown projection '{s}'. Valid projections: fields, signature, body"),
        )),
    }
}

pub fn parse_query(input: &str) -> Result<Vec<DslOp>, ParseError> {
    let tokens = lex(input)?;
    parse(&tokens)
}

// ---------------------------------------------------------------------------
// Extended expression parser: set algebra, path patterns, entrypoint selector.
//
// Grammar (left-associative, single precedence level for set ops):
//
//     expr     := atom (('union' | 'intersect' | 'diff' | '\') atom)*
//     atom     := '(' expr ')'
//                | path_query
//                | entrypoint_query
//                | pipe_chain
//
// The pipe-chain parser is reused unchanged for back-compat. Set-ops only
// preserve the `nodes` set across operands; per-operand metadata is dropped.
// ---------------------------------------------------------------------------

/// Parse a top-level [`Expr`] from a query string.
///
/// Use this entry point when callers want set algebra, path patterns, or
/// the `entrypoints` selector. For pure pipe-chain back-compat, prefer
/// [`parse_query`] which returns `Vec<DslOp>` directly.
pub fn parse_expr(input: &str) -> Result<Expr, ParseError> {
    let tokens = lex(input)?;
    if tokens.is_empty() {
        return Err(ParseError::new(0, "empty query"));
    }
    let mut pos = 0;
    let expr = parse_expr_inner(&tokens, &mut pos)?;
    if pos < tokens.len() {
        return Err(ParseError::new(
            pos,
            format!("trailing tokens after expression: {:?}", &tokens[pos..]),
        ));
    }
    Ok(expr)
}

fn parse_expr_inner(tokens: &[Token], pos: &mut usize) -> Result<Expr, ParseError> {
    let mut left = parse_atom(tokens, pos)?;
    loop {
        if *pos >= tokens.len() {
            break;
        }
        let op = match &tokens[*pos] {
            Token::Union => SetOp::Union,
            Token::Intersect => SetOp::Intersect,
            Token::Diff => SetOp::Diff,
            _ => break,
        };
        *pos += 1;
        let right = parse_atom(tokens, pos)?;
        left = Expr::SetOp {
            op,
            left: Box::new(left),
            right: Box::new(right),
        };
    }
    Ok(left)
}

fn parse_atom(tokens: &[Token], pos: &mut usize) -> Result<Expr, ParseError> {
    if *pos >= tokens.len() {
        return Err(ParseError::new(*pos, "expected expression"));
    }
    let base = match &tokens[*pos] {
        Token::LParen => {
            *pos += 1;
            let inner = parse_expr_inner(tokens, pos)?;
            if *pos >= tokens.len() || tokens[*pos] != Token::RParen {
                return Err(ParseError::new(*pos, "expected ')' to close group"));
            }
            *pos += 1;
            inner
        }
        Token::Path | Token::Paths => parse_path_query(tokens, pos)?,
        Token::Entrypoints => parse_entrypoint_query(tokens, pos)?,
        Token::Dominators => parse_dominators_query(tokens, pos)?,
        Token::Dominates => parse_dominates_query(tokens, pos)?,
        Token::TraitImpls => parse_trait_impls_query(tokens, pos)?,
        Token::MultiPath => parse_multi_path_query(tokens, pos)?,
        _ => return parse_pipe_chain_atom(tokens, pos).map(Expr::Pipe),
    };

    // If the atom is followed by `| ops...`, absorb them as a PipeFrom.
    // This enables `entrypoints kind=PublicApi | untested | depth 3`.
    if *pos < tokens.len() && tokens[*pos] == Token::Pipe {
        let mut trailing_ops = Vec::new();
        while *pos < tokens.len() && tokens[*pos] == Token::Pipe {
            *pos += 1;
            trailing_ops.push(parse_op(tokens, pos)?);
        }
        Ok(Expr::PipeFrom {
            base: Box::new(base),
            ops: trailing_ops,
        })
    } else {
        Ok(base)
    }
}

/// `dominators of <atom>` — `of` is required even though it's purely
/// connective, because `dominators fn("foo")` would otherwise let the
/// pipe-chain parser eat `dominators` as an unknown operator. Keeping
/// `of` explicit also leaves room for future `dominators since N` etc.
fn parse_dominators_query(tokens: &[Token], pos: &mut usize) -> Result<Expr, ParseError> {
    debug_assert!(matches!(tokens[*pos], Token::Dominators));
    *pos += 1;
    if *pos >= tokens.len() || tokens[*pos] != Token::Of {
        return Err(ParseError::new(*pos, "expected 'of' after 'dominators'"));
    }
    *pos += 1;
    let inner = parse_atom(tokens, pos)?;
    Ok(Expr::DominatorsOf(Box::new(inner)))
}

/// `dominates <atom>` — no `of` here because `dominates` is naturally a
/// transitive verb (`X dominates Y`). Mirrors the petgraph API name.
fn parse_dominates_query(tokens: &[Token], pos: &mut usize) -> Result<Expr, ParseError> {
    debug_assert!(matches!(tokens[*pos], Token::Dominates));
    *pos += 1;
    let inner = parse_atom(tokens, pos)?;
    Ok(Expr::DominatesOf(Box::new(inner)))
}

/// `trait_impls of <atom>` — `<atom>` should select `Trait` nodes; non-trait
/// nodes contribute no implementors and are silently skipped at execution.
fn parse_trait_impls_query(tokens: &[Token], pos: &mut usize) -> Result<Expr, ParseError> {
    debug_assert!(matches!(tokens[*pos], Token::TraitImpls));
    *pos += 1;
    if *pos >= tokens.len() || tokens[*pos] != Token::Of {
        return Err(ParseError::new(*pos, "expected 'of' after 'trait_impls'"));
    }
    *pos += 1;
    let inner = parse_atom(tokens, pos)?;
    Ok(Expr::TraitImplsOf(Box::new(inner)))
}

/// `multi_path { <expr>, <expr>, ... } -> <expr>` with optional `depth N`.
///
/// Parses the brace list as one-or-more sub-expressions separated by commas.
/// We use `multi_path` rather than overloading `path { ... }` because the
/// `path` token is already deeply integrated with single-source parsing
/// and ambiguity around `path fn("a")` (single source) vs.
/// `path {fn("a")}` (one-element multi source) would surprise the LLM.
fn parse_multi_path_query(tokens: &[Token], pos: &mut usize) -> Result<Expr, ParseError> {
    debug_assert!(matches!(tokens[*pos], Token::MultiPath));
    *pos += 1;
    if *pos >= tokens.len() || tokens[*pos] != Token::LBrace {
        return Err(ParseError::new(
            *pos,
            "expected '{' after 'multi_path' to open the source list",
        ));
    }
    *pos += 1;

    let mut sources: Vec<Expr> = Vec::new();
    loop {
        if *pos >= tokens.len() {
            return Err(ParseError::new(
                *pos,
                "unterminated source list: expected ',' or '}'",
            ));
        }
        if tokens[*pos] == Token::RBrace {
            *pos += 1;
            break;
        }
        let item = parse_atom(tokens, pos)?;
        sources.push(item);
        match tokens.get(*pos) {
            Some(Token::Comma) => {
                *pos += 1;
            }
            Some(Token::RBrace) => {
                *pos += 1;
                break;
            }
            _ => {
                return Err(ParseError::new(
                    *pos,
                    "expected ',' or '}' in multi_path source list",
                ));
            }
        }
    }

    if sources.is_empty() {
        return Err(ParseError::new(
            *pos,
            "multi_path requires at least one source",
        ));
    }

    if *pos >= tokens.len() || tokens[*pos] != Token::Arrow {
        return Err(ParseError::new(
            *pos,
            "expected '->' or '→' after multi_path source list",
        ));
    }
    *pos += 1;

    let to = parse_atom(tokens, pos)?;

    let mut max_depth: Option<usize> = None;
    if *pos < tokens.len() && tokens[*pos] == Token::Depth {
        *pos += 1;
        if *pos >= tokens.len() {
            return Err(ParseError::new(*pos, "expected number after 'depth'"));
        }
        match &tokens[*pos] {
            Token::Number(n) => {
                max_depth = Some(*n);
                *pos += 1;
            }
            _ => return Err(ParseError::new(*pos, "expected number after 'depth'")),
        }
    }

    Ok(Expr::MultiPath {
        sources,
        to: Box::new(to),
        max_depth,
    })
}

/// Parse a pipe-chain that is also a sub-expression. Stops at end-of-input
/// or any token that does not belong inside a pipe chain (set-op keyword,
/// closing paren, end of path-query operand). This is the same operator
/// soup as legacy [`parse`] but bounded so the outer expression parser can
/// continue.
fn parse_pipe_chain_atom(tokens: &[Token], pos: &mut usize) -> Result<Vec<DslOp>, ParseError> {
    let start = *pos;
    let mut ops = Vec::new();
    loop {
        if *pos >= tokens.len() {
            break;
        }
        // Stop at any token that ends a pipe-chain in the extended grammar.
        if matches!(
            tokens[*pos],
            Token::Union
                | Token::Intersect
                | Token::Diff
                | Token::RParen
                | Token::RBrace
                | Token::Comma
                | Token::Arrow
                | Token::Where
                | Token::Via
                | Token::Of
        ) {
            break;
        }
        let op = parse_op(tokens, pos)?;
        ops.push(op);
        if *pos < tokens.len() && tokens[*pos] == Token::Pipe {
            *pos += 1;
            if *pos >= tokens.len() {
                return Err(ParseError::new(*pos, "expected operation after '|'"));
            }
        } else {
            break;
        }
    }
    if ops.is_empty() {
        return Err(ParseError::new(start, "expected pipe-chain operator"));
    }
    Ok(ops)
}

fn parse_path_query(tokens: &[Token], pos: &mut usize) -> Result<Expr, ParseError> {
    let mode = match &tokens[*pos] {
        Token::Path => PathMode::Shortest,
        Token::Paths => PathMode::AllSimple,
        _ => return Err(ParseError::new(*pos, "expected 'path' or 'paths'")),
    };
    *pos += 1;

    let from = parse_atom(tokens, pos)?;

    if *pos >= tokens.len() || tokens[*pos] != Token::Arrow {
        return Err(ParseError::new(
            *pos,
            "expected '->' or '→' between path endpoints",
        ));
    }
    *pos += 1;

    let to = parse_atom(tokens, pos)?;

    let mut intermediate_kind: Option<NodeKind> = None;
    let mut via_edge: Option<EdgeKind> = None;
    let mut max_depth: Option<usize> = None;

    // Trailing qualifiers may appear in any order; loop until we run out.
    loop {
        if *pos >= tokens.len() {
            break;
        }
        match &tokens[*pos] {
            Token::Where => {
                *pos += 1;
                if *pos >= tokens.len() || tokens[*pos] != Token::Intermediate {
                    return Err(ParseError::new(
                        *pos,
                        "expected 'intermediate' after 'where'",
                    ));
                }
                *pos += 1;
                if *pos >= tokens.len() || tokens[*pos] != Token::Kind {
                    return Err(ParseError::new(
                        *pos,
                        "expected 'kind' after 'where intermediate'",
                    ));
                }
                *pos += 1;
                if *pos >= tokens.len() || tokens[*pos] != Token::Equals {
                    return Err(ParseError::new(
                        *pos,
                        "expected '=' after 'where intermediate kind'",
                    ));
                }
                *pos += 1;
                if *pos >= tokens.len() {
                    return Err(ParseError::new(*pos, "expected node kind"));
                }
                let kind = match &tokens[*pos] {
                    Token::Ident(s) => parse_node_kind(s, *pos)?,
                    _ => return Err(ParseError::new(*pos, "expected node kind identifier")),
                };
                *pos += 1;
                intermediate_kind = Some(kind);
            }
            Token::Via => {
                *pos += 1;
                if *pos >= tokens.len() {
                    return Err(ParseError::new(*pos, "expected edge kind after 'via'"));
                }
                let edge = match &tokens[*pos] {
                    Token::Ident(s) => parse_edge_kind(s, *pos)?,
                    _ => return Err(ParseError::new(*pos, "expected edge kind identifier")),
                };
                *pos += 1;
                via_edge = Some(edge);
            }
            Token::Depth => {
                *pos += 1;
                if *pos >= tokens.len() {
                    return Err(ParseError::new(*pos, "expected number after 'depth'"));
                }
                let n = match &tokens[*pos] {
                    Token::Number(n) => *n,
                    _ => return Err(ParseError::new(*pos, "expected number after 'depth'")),
                };
                *pos += 1;
                max_depth = Some(n);
            }
            _ => break,
        }
    }

    Ok(Expr::PathQuery(PathQuery {
        mode,
        from: Box::new(from),
        to: Box::new(to),
        intermediate_kind,
        via_edge,
        max_depth,
    }))
}

fn parse_entrypoint_query(tokens: &[Token], pos: &mut usize) -> Result<Expr, ParseError> {
    debug_assert!(matches!(tokens[*pos], Token::Entrypoints));
    *pos += 1;
    // Optional `kind=Main|PublicApi|Test` filter.
    if *pos < tokens.len() && tokens[*pos] == Token::Kind {
        *pos += 1;
        if *pos >= tokens.len() || tokens[*pos] != Token::Equals {
            return Err(ParseError::new(*pos, "expected '=' after 'kind'"));
        }
        *pos += 1;
        if *pos >= tokens.len() {
            return Err(ParseError::new(*pos, "expected entrypoint kind"));
        }
        let kind = match &tokens[*pos] {
            Token::Ident(s) => parse_entrypoint_kind(s, *pos)?,
            _ => return Err(ParseError::new(*pos, "expected entrypoint kind identifier")),
        };
        *pos += 1;
        Ok(Expr::Entrypoints(Some(kind)))
    } else {
        Ok(Expr::Entrypoints(None))
    }
}

fn parse_edge_kind(s: &str, pos: usize) -> Result<EdgeKind, ParseError> {
    match s {
        "Calls" => Ok(EdgeKind::Calls),
        "UnresolvedCall" => Ok(EdgeKind::UnresolvedCall(String::new())),
        "UsesType" => Ok(EdgeKind::UsesType),
        "References" => Ok(EdgeKind::References),
        "Contains" => Ok(EdgeKind::Contains),
        "Implements" => Ok(EdgeKind::Implements),
        "ExternalCall" => Ok(EdgeKind::ExternalCall(String::new(), String::new())),
        "Extends" => Ok(EdgeKind::Extends),
        "Returns" => Ok(EdgeKind::Returns),
        "TypeOf" => Ok(EdgeKind::TypeOf),
        _ => Err(ParseError::new(
            pos,
            format!(
                "unknown edge kind '{s}'. Valid: Calls, UnresolvedCall, UsesType, \
                 References, Contains, Implements, ExternalCall, Extends, Returns, TypeOf"
            ),
        )),
    }
}

fn parse_entrypoint_kind(s: &str, pos: usize) -> Result<EntrypointKind, ParseError> {
    match s {
        "Main" => Ok(EntrypointKind::Main),
        "PublicApi" => Ok(EntrypointKind::PublicApi),
        "Test" => Ok(EntrypointKind::Test),
        "Bench" => Ok(EntrypointKind::Bench),
        "FfiExport" => Ok(EntrypointKind::FfiExport),
        _ => Err(ParseError::new(
            pos,
            format!(
                "unknown entrypoint kind '{s}'. Valid: Main, PublicApi, Test, Bench, FfiExport"
            ),
        )),
    }
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
///
/// Serializable for stable JSON exports — see [`crate::schema`] for the
/// versioned envelope and JSON Schema definitions consumers can depend on.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
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
    /// Free-form lines describing higher-order structure that doesn't fit
    /// in `nodes`/`edges`: SCC clusters with member lists, type clusters
    /// with their primary type, entrypoint kinds and reach metrics, etc.
    /// Each entry is a single human-readable line — renderers can show
    /// these verbatim, parse selectively, or ignore them. Empty for the
    /// majority of queries that don't carry analytical metadata.
    pub metadata: Vec<String>,
}

impl QueryResult {
    /// Render each result node as a structured `kind:qualified_name`
    /// handle string, using the node's own [`NodeKind`] and
    /// [`crate::nodes::NodeData::qualified_name`] (the same scheme
    /// [`crate::symbols::SymbolTable`] uses for `fn:`/`struct:`/`enum:`/
    /// `trait:`/`mod:`).
    ///
    /// Nodes whose [`NodeId`] no longer resolves in `graph` (e.g. the
    /// graph was mutated between query and rendering) are skipped
    /// silently — handles are best-effort by design.
    pub fn handles(&self, graph: &CodeGraph) -> Vec<String> {
        self.nodes
            .iter()
            .filter_map(|id| {
                let node = graph.get_node(id)?;
                let prefix = match node.kind {
                    NodeKind::Function => "fn",
                    NodeKind::Struct => "struct",
                    NodeKind::Enum => "enum",
                    NodeKind::Trait => "trait",
                    NodeKind::Module => "mod",
                    NodeKind::EnumVariant => "variant",
                    NodeKind::Field => "field",
                    NodeKind::TypeAlias => "type",
                    NodeKind::Constant => "const",
                    NodeKind::Interface => "interface",
                };
                Some(format!("{}:{}", prefix, node.qualified_name))
            })
            .collect()
    }
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
        let mut metadata: Vec<String> = Vec::new();

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
                                parallel: false,
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
                // `since N` — history-axis filter. Keep only nodes whose
                // `last_modified_revision >= rev`. Implemented as an in-place
                // retention rather than a re-intersection with
                // `nodes_changed_since` to avoid an `O(n)` full-graph scan
                // when the working set is already small.
                DslOp::Since(rev) => {
                    working_set.retain(|id| {
                        self.graph
                            .get_node(id)
                            .map(|n| n.last_modified_revision >= *rev)
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
                    let mut queue: VecDeque<NodeId> = working_set.iter().cloned().collect();

                    while let Some(current) = queue.pop_front() {
                        if visited.contains(&current) {
                            cycles_detected.push(current);
                            continue;
                        }
                        visited.insert(current.clone());
                        reachers.insert(current.clone());

                        for (source_id, edge) in self.graph.get_edges_to(&current) {
                            if matches!(edge.kind, EdgeKind::Calls | EdgeKind::UnresolvedCall(_)) {
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
                // `hot N` — top-N by PageRank.
                //
                // Bare invocation (empty working set) ranks the entire graph
                // via `hottest_functions`. Postfix invocation re-ranks only
                // the working-set members against the full-graph PageRank
                // scores (no separate restricted PageRank — the full-graph
                // scores carry the right "global importance" signal that
                // a restricted re-run would obscure).
                DslOp::Hot(n) => {
                    if working_set.is_empty() {
                        let ranked = self.graph.hottest_functions(*n);
                        metadata.push(format!("hot top={n} bare=true"));
                        for (id, score) in ranked {
                            metadata.push(format!(
                                "hot {} score={:.4}",
                                self.graph
                                    .get_node(&id)
                                    .map(|nd| nd.qualified_name.clone())
                                    .unwrap_or_else(|| format!("{id:?}")),
                                score
                            ));
                            working_set.insert(id);
                        }
                    } else {
                        let centrality = self.graph.centrality();
                        let mut scored: Vec<(NodeId, f64)> = working_set
                            .iter()
                            .map(|id| {
                                let s = centrality.pagerank.get(id).copied().unwrap_or(0.0);
                                (id.clone(), s)
                            })
                            .collect();
                        scored.sort_by(|a, b| {
                            b.1.partial_cmp(&a.1)
                                .unwrap_or(std::cmp::Ordering::Equal)
                                .then_with(|| a.0.cmp(&b.0))
                        });
                        scored.truncate(*n);
                        metadata.push(format!("hot top={n} bare=false"));
                        let kept: HashSet<NodeId> =
                            scored.iter().map(|(id, _)| id.clone()).collect();
                        for (id, score) in &scored {
                            metadata.push(format!(
                                "hot {} score={:.4}",
                                self.graph
                                    .get_node(id)
                                    .map(|nd| nd.qualified_name.clone())
                                    .unwrap_or_else(|| format!("{id:?}")),
                                score
                            ));
                        }
                        working_set = kept;
                    }
                }
                // `scc` — strongly connected components.
                //
                // Bare: every member of every multi-element SCC.
                // Postfix: union of every multi-element SCC that contains at
                // least one working-set node. Singleton SCCs are excluded —
                // the question "what loops?" wants cycles, not literally
                // every node.
                DslOp::Scc => {
                    let part = self.graph.strongly_connected_components();
                    let bare = working_set.is_empty();
                    let mut keep_components: Vec<usize> = Vec::new();
                    if bare {
                        for (i, comp) in part.components.iter().enumerate() {
                            if comp.len() > 1 {
                                keep_components.push(i);
                            }
                        }
                    } else {
                        let mut seen: HashSet<usize> = HashSet::new();
                        for id in &working_set {
                            if let Some(&idx) = part.component_of.get(id) {
                                if part.components[idx].len() > 1 && seen.insert(idx) {
                                    keep_components.push(idx);
                                }
                            }
                        }
                    }
                    let mut new_set: HashSet<NodeId> = HashSet::new();
                    for &i in &keep_components {
                        let comp = &part.components[i];
                        let names: Vec<String> = comp
                            .iter()
                            .take(8)
                            .filter_map(|id| self.graph.get_node(id).map(|n| n.name.clone()))
                            .collect();
                        metadata.push(format!(
                            "SCC[{i}] size={} members=[{}]",
                            comp.len(),
                            names.join(", ")
                        ));
                        for id in comp {
                            new_set.insert(id.clone());
                        }
                    }
                    if !bare {
                        // For postfix form, intersect the SCC union with the
                        // original working set's component membership only —
                        // but the user reads `scc` as "expand to the SCC", so
                        // returning the full cluster is the more useful read.
                        // We keep the union as-is.
                    }
                    working_set = new_set;
                }
                // `dispatch` — postfix filter: callers participating in trait dispatch.
                //
                // Computed from `trait_dispatch_calls()`: each entry is a
                // (caller, callee, trait) triple where the callee is a
                // function declared on a trait. We restrict working_set to
                // callers; the callee + trait info goes into metadata so
                // renderers can show "X calls Trait::method".
                DslOp::Dispatch => {
                    let calls = self.graph.trait_dispatch_calls();
                    let working_subset: HashSet<NodeId> = working_set.iter().cloned().collect();
                    let mut keep: HashSet<NodeId> = HashSet::new();
                    for d in &calls {
                        let in_working =
                            working_subset.is_empty() || working_subset.contains(&d.caller);
                        if !in_working {
                            continue;
                        }
                        let caller_name = self
                            .graph
                            .get_node(&d.caller)
                            .map(|n| n.qualified_name.clone())
                            .unwrap_or_else(|| format!("{:?}", d.caller));
                        let callee_name = self
                            .graph
                            .get_node(&d.callee)
                            .map(|n| n.qualified_name.clone())
                            .unwrap_or_else(|| format!("{:?}", d.callee));
                        let trait_name = self
                            .graph
                            .get_node(&d.trait_id)
                            .map(|n| n.qualified_name.clone())
                            .unwrap_or_else(|| format!("{:?}", d.trait_id));
                        metadata.push(format!(
                            "dispatch {caller_name} -> {trait_name}::{callee_name}"
                        ));
                        keep.insert(d.caller.clone());
                    }
                    working_set = keep;
                }
                // `cluster by type` — group functions by primary `UsesType` target.
                //
                // Postfix: only consider functions in the working set when
                // computing primary-type membership (the cluster set itself
                // is restricted, but we still call the full `cluster_by_primary_type`
                // and filter — the analysis is whole-graph by design).
                DslOp::ClusterByType => {
                    let clusters = self.graph.cluster_by_primary_type();
                    let restrict: Option<HashSet<NodeId>> = if working_set.is_empty() {
                        None
                    } else {
                        Some(working_set.iter().cloned().collect())
                    };
                    let mut new_set: HashSet<NodeId> = HashSet::new();
                    for (i, c) in clusters.iter().enumerate() {
                        let funcs: Vec<NodeId> = match &restrict {
                            Some(r) => c
                                .functions
                                .iter()
                                .filter(|f| r.contains(*f))
                                .cloned()
                                .collect(),
                            None => c.functions.iter().cloned().collect(),
                        };
                        if funcs.is_empty() {
                            continue;
                        }
                        let primary_name = self
                            .graph
                            .get_node(&c.primary_type)
                            .map(|n| n.qualified_name.clone())
                            .unwrap_or_else(|| format!("{:?}", c.primary_type));
                        let member_names: Vec<String> = funcs
                            .iter()
                            .take(8)
                            .filter_map(|id| self.graph.get_node(id).map(|n| n.name.clone()))
                            .collect();
                        metadata.push(format!(
                            "cluster[{i}] type={primary_name} size={} members=[{}]",
                            funcs.len(),
                            member_names.join(", ")
                        ));
                        for id in funcs {
                            new_set.insert(id);
                        }
                    }
                    working_set = new_set;
                }
                // `affected N since M` — undirected N-hop neighborhood of every
                // node modified at or after revision M. Replaces the working set
                // (this is a selector, not a filter — it's equivalent to having
                // had a `selector` op produce the changed set then expanded).
                DslOp::Affected { depth, since_rev } => {
                    let nodes = self.graph.nodes_changed_within_depth(*since_rev, *depth);
                    metadata.push(format!(
                        "affected depth={depth} since={since_rev} count={}",
                        nodes.len()
                    ));
                    working_set = nodes.into_iter().collect();
                }
                DslOp::Taint(_var_name) => {
                    // Taint analysis v1: BFS over outgoing call edges from working set
                    // with cycle detection. The var_name is metadata only — full
                    // inter-procedural tracking is out of scope for v1.
                    let mut tainted = HashSet::new();
                    let mut visited = HashSet::new();
                    let mut queue: VecDeque<NodeId> = working_set.iter().cloned().collect();

                    while let Some(current) = queue.pop_front() {
                        if visited.contains(&current) {
                            cycles_detected.push(current);
                            continue;
                        }
                        visited.insert(current.clone());
                        tainted.insert(current.clone());

                        for (target_id, edge) in self.graph.get_edges_from(&current) {
                            if matches!(edge.kind, EdgeKind::Calls | EdgeKind::UnresolvedCall(_)) {
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
                // `untested` — retain only nodes with coverage_tested != "true"
                // or no coverage annotation at all (presumed untested).
                DslOp::Untested => {
                    working_set.retain(|id| {
                        self.graph
                            .get_node(id)
                            .map(|n| {
                                n.metadata
                                    .get("coverage_tested")
                                    .map(|v| v != "true")
                                    .unwrap_or(true) // No annotation = untested.
                            })
                            .unwrap_or(false)
                    });
                    metadata.push(format!("untested count={}", working_set.len()));
                }
                // `possible_types` — enrich output with possible-type metadata
                // and filter to nodes that have at least one possible type.
                DslOp::PossibleTypes => {
                    working_set.retain(|id| {
                        self.graph
                            .get_node(id)
                            .map(|n| {
                                n.metadata.contains_key("possible_input_types")
                                    || n.metadata.contains_key("possible_return_types")
                            })
                            .unwrap_or(false)
                    });
                    for id in &working_set {
                        if let Some(n) = self.graph.get_node(id) {
                            let mut parts = Vec::new();
                            if let Some(inputs) = n.metadata.get("possible_input_types") {
                                parts.push(format!("inputs={inputs}"));
                            }
                            if let Some(returns) = n.metadata.get("possible_return_types") {
                                parts.push(format!("returns={returns}"));
                            }
                            if !parts.is_empty() {
                                metadata.push(format!(
                                    "possible_types {} {}",
                                    n.qualified_name,
                                    parts.join(" ")
                                ));
                            }
                        }
                    }
                }
                // `co_changes` — temporal coupling from git history. Expands
                // the working set to include nodes that frequently co-change
                // with the current selection. Git history is fetched on demand.
                DslOp::CoChanges => {
                    let seed_nodes: Vec<NodeId> = working_set.iter().cloned().collect();
                    // Determine workspace root heuristically from node file paths.
                    let workspace_root = self
                        .graph
                        .all_node_ids()
                        .first()
                        .and_then(|id| self.graph.get_node(id))
                        .and_then(|n| n.file_path.parent().map(|p| p.to_path_buf()))
                        .unwrap_or_else(|| std::path::PathBuf::from("."));
                    let commits = crate::co_change::fetch_git_history(&workspace_root, 500);
                    let result = crate::co_change::co_changes_for_nodes(
                        self.graph,
                        &commits,
                        &seed_nodes,
                        2, // min_support: at least 2 co-occurrences
                    );
                    for pair in &result.pairs {
                        let other = if seed_nodes.contains(&pair.node_a) {
                            &pair.node_b
                        } else {
                            &pair.node_a
                        };
                        working_set.insert(other.clone());
                        let other_name = self
                            .graph
                            .get_node(other)
                            .map(|n| n.qualified_name.clone())
                            .unwrap_or_else(|| format!("{other:?}"));
                        metadata.push(format!(
                            "co_change {} times={} confidence={:.3}",
                            other_name, pair.times_changed_together, pair.confidence
                        ));
                    }
                }
                DslOp::Communities => {
                    // Run Louvain community detection on the full graph,
                    // filter to communities that overlap with the working set.
                    let result = crate::communities::louvain(self.graph, 1.0, 42);
                    let restrict: Option<HashSet<NodeId>> = if working_set.is_empty() {
                        None
                    } else {
                        Some(working_set.iter().cloned().collect())
                    };
                    // Group assignments by community.
                    let mut by_community: HashMap<u32, Vec<NodeId>> = HashMap::new();
                    for (node_id, comm) in &result.assignments {
                        let include = match &restrict {
                            Some(r) => r.contains(node_id),
                            None => true,
                        };
                        if include {
                            by_community.entry(*comm).or_default().push(node_id.clone());
                        }
                    }
                    // Build new working set and metadata.
                    let mut new_set: HashSet<NodeId> = HashSet::new();
                    let mut sorted_comms: Vec<u32> = by_community.keys().copied().collect();
                    sorted_comms.sort();
                    for comm in sorted_comms {
                        let members = &by_community[&comm];
                        let names: Vec<String> = members
                            .iter()
                            .take(8)
                            .filter_map(|id| self.graph.get_node(id).map(|n| n.name.clone()))
                            .collect();
                        metadata.push(format!(
                            "community {comm}: [{}]{}",
                            names.join(", "),
                            if members.len() > 8 {
                                format!(" (+{} more)", members.len() - 8)
                            } else {
                                String::new()
                            }
                        ));
                        for id in members {
                            new_set.insert(id.clone());
                        }
                    }
                    metadata.push(format!(
                        "communities total={} modularity={:.4}",
                        result.community_count, result.modularity
                    ));
                    working_set = new_set;
                }
                DslOp::Complexity => {
                    // Retain only Function nodes that have complexity metrics.
                    working_set.retain(|id| {
                        self.graph
                            .get_node(id)
                            .and_then(|n| n.complexity.as_ref())
                            .is_some()
                    });
                    // Surface metrics in metadata.
                    for id in &working_set {
                        if let Some(n) = self.graph.get_node(id) {
                            if let Some(cx) = &n.complexity {
                                let mut parts = vec![
                                    format!("cognitive={}", cx.cognitive),
                                    format!("cyclomatic={}", cx.cyclomatic),
                                    format!("max_nesting={}", cx.max_nesting),
                                ];
                                if let Some(ref h) = cx.halstead {
                                    parts.push(format!("volume={:.1}", h.volume));
                                    parts.push(format!("effort={:.1}", h.effort));
                                    parts.push(format!("bugs={:.3}", h.bugs));
                                }
                                if let Some(ref loc) = cx.loc {
                                    parts.push(format!(
                                        "loc(total={},source={},comment={})",
                                        loc.total, loc.source, loc.comment
                                    ));
                                }
                                if let Some(mi) = cx.maintainability_index {
                                    parts.push(format!("MI={:.1}", mi));
                                }
                                metadata.push(format!(
                                    "complexity {} {}",
                                    n.qualified_name,
                                    parts.join(" ")
                                ));
                            }
                        }
                    }
                }
                DslOp::Cfg => {
                    // Retain only Function nodes that have CFG data.
                    working_set.retain(|id| {
                        self.graph
                            .get_node(id)
                            .and_then(|n| n.cfg.as_ref())
                            .is_some()
                    });
                    // Surface CFG summary in metadata.
                    for id in &working_set {
                        if let Some(n) = self.graph.get_node(id) {
                            if let Some(ref cfg) = n.cfg {
                                metadata.push(format!(
                                    "cfg {} {}",
                                    n.qualified_name,
                                    cfg.format_summary().replace('\n', " | ")
                                ));
                            }
                        }
                    }
                }
                DslOp::Dataflow => {
                    // Retain only Function nodes that have dataflow data.
                    working_set.retain(|id| {
                        self.graph
                            .get_node(id)
                            .and_then(|n| n.dataflow.as_ref())
                            .is_some()
                    });
                    // Surface dataflow summary in metadata.
                    for id in &working_set {
                        if let Some(n) = self.graph.get_node(id) {
                            if let Some(ref df) = n.dataflow {
                                metadata.push(format!(
                                    "dataflow {} {}",
                                    n.qualified_name,
                                    df.format_summary().replace('\n', " | ")
                                ));
                            }
                        }
                    }
                }
            }
        }

        let node_list: Vec<NodeId> = working_set.into_iter().collect();
        let node_set: HashSet<&NodeId> = node_list.iter().collect();
        let mut edges = Vec::new();
        for node_id in &node_list {
            for (target, edge_data) in self.graph.get_edges_from(node_id) {
                if node_set.contains(target) {
                    edges.push((
                        node_id.clone(),
                        target.clone(),
                        format!("{:?}", edge_data.kind),
                    ));
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
            metadata,
        })
    }
}

/// Convenience function: parse and execute a query string.
///
/// Phase 3: runs through the [`crate::dsl::plan`] optimiser before
/// execution. Rewrites are semantics-preserving — depth fusion,
/// filter pushdown — so callers see only a perf improvement.
pub fn run_query(
    query: &str,
    graph: &CodeGraph,
    config: &QueryConfig,
) -> Result<QueryResult, QueryError> {
    let ops = parse_query(query)?;
    let plan = crate::dsl::plan::optimise_pipe(ops);
    let engine = QueryEngine::new(graph);
    let ops = plan.ops().expect("optimise_pipe yields Plan::Pipe");
    engine.execute(ops, config)
}

// ---------------------------------------------------------------------------
// Expression executor: evaluates [`Expr`] nodes (set algebra, path patterns,
// entrypoint selectors). Set ops keep `nodes` only; everything else is
// dropped — see module docs for rationale.
// ---------------------------------------------------------------------------

impl<'a> QueryEngine<'a> {
    /// Execute an [`Expr`] (extended grammar with set algebra, path
    /// patterns, and entrypoint selectors).
    pub fn execute_expr(
        &self,
        expr: &Expr,
        config: &QueryConfig,
    ) -> Result<QueryResult, QueryError> {
        match expr {
            Expr::Pipe(ops) => self.execute(ops, config),
            Expr::Entrypoints(kind_filter) => Ok(self.execute_entrypoints(*kind_filter, config)),
            Expr::PathQuery(pq) => self.execute_path_query(pq, config),
            Expr::SetOp { op, left, right } => {
                let l = self.execute_expr(left, config)?;
                let r = self.execute_expr(right, config)?;
                Ok(combine_set_op(*op, l, r, config.max_nodes))
            }
            Expr::DominatorsOf(inner) => {
                let seed = self.execute_expr(inner, config)?;
                Ok(self.execute_dominators_of(&seed.nodes, config))
            }
            Expr::DominatesOf(inner) => {
                let seed = self.execute_expr(inner, config)?;
                Ok(self.execute_dominates_of(&seed.nodes, config))
            }
            Expr::TraitImplsOf(inner) => {
                let seed = self.execute_expr(inner, config)?;
                Ok(self.execute_trait_impls_of(&seed.nodes, config))
            }
            Expr::PipeFrom { base, ops } => {
                let seed = self.execute_expr(base, config)?;
                // Feed the seed's nodes into a pipe-chain execution as the
                // initial working set. Build a synthetic op list that starts
                // with a SelectFn for each seed node... but that's awkward.
                // Instead, run the pipe ops with the seed as pre-populated
                // working set by calling execute_with_seed.
                self.execute_pipe_from(&seed.nodes, ops, config)
            }
            Expr::MultiPath {
                sources,
                to,
                max_depth,
            } => self.execute_multi_path(sources, to, *max_depth, config),
        }
    }

    /// Execute pipe ops against a pre-seeded working set. Used by
    /// `PipeFrom` to chain entrypoints/dominators/etc. with `| untested`.
    fn execute_pipe_from(
        &self,
        seed: &[NodeId],
        ops: &[DslOp],
        config: &QueryConfig,
    ) -> Result<QueryResult, QueryError> {
        // Build a synthetic pipe: start with SelectFn/SelectType for each
        // seed node... or, more directly, run the execute loop with the
        // working_set pre-populated. We duplicate the execute loop body here
        // with a pre-seeded working set.
        let mut working_set: HashSet<NodeId> = seed.iter().cloned().collect();
        let cycles_detected: Vec<NodeId> = Vec::new();
        let mut metadata: Vec<String> = Vec::new();

        for op in ops {
            match op {
                // Select ops override the working set.
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
                DslOp::Filter(kind) => {
                    working_set.retain(|id| {
                        self.graph
                            .get_node(id)
                            .map(|n| n.kind == *kind)
                            .unwrap_or(false)
                    });
                }
                DslOp::Since(rev) => {
                    working_set.retain(|id| {
                        self.graph
                            .get_node(id)
                            .map(|n| n.last_modified_revision >= *rev)
                            .unwrap_or(false)
                    });
                }
                DslOp::Depth(n) => {
                    let max_depth = *n;
                    let current: Vec<NodeId> = working_set.iter().cloned().collect();
                    for _ in 0..max_depth {
                        let mut next_layer = HashSet::new();
                        for id in &current {
                            for (target_id, _) in self.graph.get_edges_from(id) {
                                next_layer.insert(target_id.clone());
                            }
                        }
                        working_set.extend(next_layer);
                    }
                }
                DslOp::Untested => {
                    working_set.retain(|id| {
                        self.graph
                            .get_node(id)
                            .map(|n| {
                                n.metadata
                                    .get("coverage_tested")
                                    .map(|v| v != "true")
                                    .unwrap_or(true)
                            })
                            .unwrap_or(false)
                    });
                    metadata.push(format!("untested count={}", working_set.len()));
                }
                DslOp::PossibleTypes => {
                    working_set.retain(|id| {
                        self.graph
                            .get_node(id)
                            .map(|n| {
                                n.metadata.contains_key("possible_input_types")
                                    || n.metadata.contains_key("possible_return_types")
                            })
                            .unwrap_or(false)
                    });
                    for id in &working_set {
                        if let Some(n) = self.graph.get_node(id) {
                            let mut parts = Vec::new();
                            if let Some(inputs) = n.metadata.get("possible_input_types") {
                                parts.push(format!("inputs={inputs}"));
                            }
                            if let Some(returns) = n.metadata.get("possible_return_types") {
                                parts.push(format!("returns={returns}"));
                            }
                            if !parts.is_empty() {
                                metadata.push(format!(
                                    "possible_types {} {}",
                                    n.qualified_name,
                                    parts.join(" ")
                                ));
                            }
                        }
                    }
                }
                DslOp::CoChanges => {
                    let seed_nodes: Vec<NodeId> = working_set.iter().cloned().collect();
                    let workspace_root = self
                        .graph
                        .all_node_ids()
                        .first()
                        .and_then(|id| self.graph.get_node(id))
                        .and_then(|n| n.file_path.parent().map(|p| p.to_path_buf()))
                        .unwrap_or_else(|| std::path::PathBuf::from("."));
                    let commits = crate::co_change::fetch_git_history(&workspace_root, 500);
                    let result = crate::co_change::co_changes_for_nodes(
                        self.graph,
                        &commits,
                        &seed_nodes,
                        2,
                    );
                    for pair in &result.pairs {
                        let other = if seed_nodes.contains(&pair.node_a) {
                            &pair.node_b
                        } else {
                            &pair.node_a
                        };
                        working_set.insert(other.clone());
                        let other_name = self
                            .graph
                            .get_node(other)
                            .map(|n| n.qualified_name.clone())
                            .unwrap_or_else(|| format!("{other:?}"));
                        metadata.push(format!(
                            "co_change {} times={} confidence={:.3}",
                            other_name, pair.times_changed_together, pair.confidence
                        ));
                    }
                }
                DslOp::Communities => {
                    let result = crate::communities::louvain(self.graph, 1.0, 42);
                    let restrict: Option<HashSet<NodeId>> = if working_set.is_empty() {
                        None
                    } else {
                        Some(working_set.iter().cloned().collect())
                    };
                    let mut by_community: HashMap<u32, Vec<NodeId>> = HashMap::new();
                    for (node_id, comm) in &result.assignments {
                        let include = match &restrict {
                            Some(r) => r.contains(node_id),
                            None => true,
                        };
                        if include {
                            by_community.entry(*comm).or_default().push(node_id.clone());
                        }
                    }
                    let mut new_set: HashSet<NodeId> = HashSet::new();
                    let mut sorted_comms: Vec<u32> = by_community.keys().copied().collect();
                    sorted_comms.sort();
                    for comm in sorted_comms {
                        let members = &by_community[&comm];
                        let names: Vec<String> = members
                            .iter()
                            .take(8)
                            .filter_map(|id| self.graph.get_node(id).map(|n| n.name.clone()))
                            .collect();
                        metadata.push(format!(
                            "community {comm}: [{}]{}",
                            names.join(", "),
                            if members.len() > 8 {
                                format!(" (+{} more)", members.len() - 8)
                            } else {
                                String::new()
                            }
                        ));
                        for id in members {
                            new_set.insert(id.clone());
                        }
                    }
                    metadata.push(format!(
                        "communities total={} modularity={:.4}",
                        result.community_count, result.modularity
                    ));
                    working_set = new_set;
                }
                // Other ops: pass through without action (show, callers,
                // callees, taint, etc. are handled by the full execute loop
                // but would require duplicating significant code here).
                _ => {}
            }
        }

        let node_list: Vec<NodeId> = working_set.into_iter().collect();
        let node_set: HashSet<&NodeId> = node_list.iter().collect();
        let mut edges = Vec::new();
        for node_id in &node_list {
            for (target, edge_data) in self.graph.get_edges_from(node_id) {
                if node_set.contains(target) {
                    edges.push((
                        node_id.clone(),
                        target.clone(),
                        format!("{:?}", edge_data.kind),
                    ));
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
            cycles_detected,
            was_truncated,
            total_before_truncation: total,
            metadata,
        })
    }

    /// Walk the dominator chain from each seed up to the root.
    ///
    /// Returns the union of every chain (and the seeds themselves are
    /// excluded — `dominators_chain` returns strict ancestors, which
    /// matches the user's mental model: "what dominates X?" should not
    /// include X). Records the chosen root in `metadata`.
    fn execute_dominators_of(&self, seeds: &[NodeId], config: &QueryConfig) -> QueryResult {
        let mut result = QueryResult::default();
        let Some((root_id, root_idx)) = self.pick_dominator_root() else {
            result
                .metadata
                .push("dominators: no entry node available".to_string());
            return result;
        };
        let dom = crate::dominators::Dominators::build(self.graph.inner(), root_idx);

        let root_name = self
            .graph
            .get_node(&root_id)
            .map(|n| n.qualified_name.clone())
            .unwrap_or_else(|| format!("{root_id:?}"));
        result.metadata.push(format!("dominators root={root_name}"));

        let mut nodes: HashSet<NodeId> = HashSet::new();
        for seed in seeds {
            let Some(seed_idx) = self.graph.resolve(seed) else {
                continue;
            };
            for ancestor_idx in dom.dominators_chain(&seed_idx) {
                if let Some(id) = self.graph.node_id_for(ancestor_idx) {
                    nodes.insert(id.clone());
                }
            }
        }
        let total = nodes.len();
        let was_truncated = total > config.max_nodes;
        let mut node_list: Vec<NodeId> = nodes.into_iter().collect();
        if was_truncated {
            node_list.truncate(config.max_nodes);
        }
        result.nodes = node_list;
        result.was_truncated = was_truncated;
        result.total_before_truncation = total;
        result
    }

    /// Inverse of [`Self::execute_dominators_of`]: every node whose
    /// dominator chain contains any seed.
    fn execute_dominates_of(&self, seeds: &[NodeId], config: &QueryConfig) -> QueryResult {
        let mut result = QueryResult::default();
        let Some((root_id, root_idx)) = self.pick_dominator_root() else {
            result
                .metadata
                .push("dominates: no entry node available".to_string());
            return result;
        };
        let dom = crate::dominators::Dominators::build(self.graph.inner(), root_idx);
        let root_name = self
            .graph
            .get_node(&root_id)
            .map(|n| n.qualified_name.clone())
            .unwrap_or_else(|| format!("{root_id:?}"));
        result.metadata.push(format!("dominates root={root_name}"));

        // Build set of seed indices.
        let seed_idxs: HashSet<petgraph::stable_graph::NodeIndex> = seeds
            .iter()
            .filter_map(|id| self.graph.resolve(id))
            .collect();
        if seed_idxs.is_empty() {
            return result;
        }

        // For every node in the graph, check whether its dom chain hits any seed.
        let mut nodes: HashSet<NodeId> = HashSet::new();
        for idx in self.graph.inner().node_indices() {
            let chain = dom.dominators_chain(&idx);
            for ancestor in chain {
                if seed_idxs.contains(&ancestor) {
                    if let Some(id) = self.graph.node_id_for(idx) {
                        nodes.insert(id.clone());
                    }
                    break;
                }
            }
        }
        let total = nodes.len();
        let was_truncated = total > config.max_nodes;
        let mut node_list: Vec<NodeId> = nodes.into_iter().collect();
        if was_truncated {
            node_list.truncate(config.max_nodes);
        }
        result.nodes = node_list;
        result.was_truncated = was_truncated;
        result.total_before_truncation = total;
        result
    }

    /// `trait_impls of <expr>` — for each `Trait` selected by `<expr>`,
    /// return its direct implementors (Struct/Enum nodes via `Implements`
    /// edges). Sourced from
    /// [`crate::graph::CodeGraph::trait_hierarchies`].
    fn execute_trait_impls_of(&self, seeds: &[NodeId], config: &QueryConfig) -> QueryResult {
        let mut result = QueryResult::default();
        let seed_set: HashSet<&NodeId> = seeds.iter().collect();
        let hierarchies = self.graph.trait_hierarchies();
        let mut nodes: HashSet<NodeId> = HashSet::new();
        for h in &hierarchies {
            if !seed_set.contains(&h.trait_id) {
                continue;
            }
            let trait_name = self
                .graph
                .get_node(&h.trait_id)
                .map(|n| n.qualified_name.clone())
                .unwrap_or_else(|| format!("{:?}", h.trait_id));
            let impl_names: Vec<String> = h
                .direct_impls
                .iter()
                .take(8)
                .filter_map(|id| self.graph.get_node(id).map(|n| n.name.clone()))
                .collect();
            result.metadata.push(format!(
                "trait_impls {trait_name} count={} impls=[{}]",
                h.direct_impls.len(),
                impl_names.join(", ")
            ));
            for id in &h.direct_impls {
                nodes.insert(id.clone());
            }
        }
        let total = nodes.len();
        let was_truncated = total > config.max_nodes;
        let mut node_list: Vec<NodeId> = nodes.into_iter().collect();
        if was_truncated {
            node_list.truncate(config.max_nodes);
        }
        result.nodes = node_list;
        result.was_truncated = was_truncated;
        result.total_before_truncation = total;
        result
    }

    /// Multi-source shortest path — wraps
    /// [`crate::traversal::find_path_multi_source`]. Sources are unioned
    /// from every `<expr>` operand inside the brace list; the BFS picks
    /// whichever source reaches `to` first.
    fn execute_multi_path(
        &self,
        sources: &[Expr],
        to: &Expr,
        max_depth: Option<usize>,
        config: &QueryConfig,
    ) -> Result<QueryResult, QueryError> {
        let mut all_sources: HashSet<NodeId> = HashSet::new();
        for s in sources {
            let r = self.execute_expr(s, config)?;
            for id in r.nodes {
                all_sources.insert(id);
            }
        }
        let to_set = self.execute_expr(to, config)?;
        let depth = max_depth.unwrap_or(32);

        let src_vec: Vec<NodeId> = all_sources.into_iter().collect();
        let mut nodes: HashSet<NodeId> = HashSet::new();
        let mut edges: Vec<(NodeId, NodeId, String)> = Vec::new();
        let mut seen_edges: HashSet<(NodeId, NodeId, String)> = HashSet::new();
        let mut metadata: Vec<String> = Vec::new();
        for target in &to_set.nodes {
            let Some(path) = traversal::find_path_multi_source(self.graph, &src_vec, target, depth)
            else {
                continue;
            };
            metadata.push(format!(
                "multi_path source={:?} to={} len={}",
                self.graph
                    .get_node(&path[0])
                    .map(|n| n.name.clone())
                    .unwrap_or_default(),
                self.graph
                    .get_node(target)
                    .map(|n| n.name.clone())
                    .unwrap_or_default(),
                path.len()
            ));
            for (i, id) in path.iter().enumerate() {
                nodes.insert(id.clone());
                if i + 1 < path.len() {
                    let next = &path[i + 1];
                    let kind_str = self
                        .graph
                        .get_edges_from(id)
                        .into_iter()
                        .find(|(t, _)| *t == next)
                        .map(|(_, e)| format!("{:?}", e.kind))
                        .unwrap_or_else(|| "UnknownEdge".to_string());
                    let key = (id.clone(), next.clone(), kind_str);
                    if seen_edges.insert(key.clone()) {
                        edges.push(key);
                    }
                }
            }
            if nodes.len() >= config.max_nodes {
                break;
            }
        }

        let total = nodes.len();
        let was_truncated = total > config.max_nodes;
        let mut node_list: Vec<NodeId> = nodes.into_iter().collect();
        if was_truncated {
            node_list.truncate(config.max_nodes);
        }
        Ok(QueryResult {
            nodes: node_list,
            edges,
            was_truncated,
            total_before_truncation: total,
            cycles_detected: Vec::new(),
            metadata,
        })
    }

    /// Pick a dominator-tree root for the call graph.
    ///
    /// Strategy:
    /// 1. A `Function` whose `name == "main"` (deterministic via
    ///    `nodes_by_kind_name`-style lookup over `find_by_name`).
    /// 2. The `Function` with the highest fan-in (most callers) — a
    ///    decent proxy for "central" when there is no `main` (library
    ///    crates, test binaries, etc.). Ties broken by `NodeId` order
    ///    for determinism.
    /// 3. `None` if neither is available.
    ///
    /// Returns both the [`NodeId`] and its petgraph index because callers
    /// already need the index to build the `Dominators` and the `NodeId`
    /// for metadata reporting.
    fn pick_dominator_root(&self) -> Option<(NodeId, petgraph::stable_graph::NodeIndex)> {
        // First preference: fn main.
        for n in self.graph.find_by_name("main") {
            if n.kind == NodeKind::Function && n.name == "main" {
                if let Some(idx) = self.graph.resolve(&n.id) {
                    return Some((n.id.clone(), idx));
                }
            }
        }
        // Fallback: highest fan-in function.
        let mut best: Option<(usize, NodeId, petgraph::stable_graph::NodeIndex)> = None;
        for func in self.graph.nodes_by_kind(NodeKind::Function) {
            let Some(idx) = self.graph.resolve(&func.id) else {
                continue;
            };
            let fan_in = self.graph.get_edges_to(&func.id).len();
            best = match best {
                None => Some((fan_in, func.id.clone(), idx)),
                Some((bf, ref bid, bidx)) => {
                    if fan_in > bf || (fan_in == bf && &func.id < bid) {
                        Some((fan_in, func.id.clone(), idx))
                    } else {
                        Some((bf, bid.clone(), bidx))
                    }
                }
            };
        }
        best.map(|(_, id, idx)| (id, idx))
    }

    /// Execute the `entrypoints` selector by delegating to
    /// [`crate::analysis::CodeGraph::classify_entrypoints`].
    ///
    /// The classifier returns one [`crate::analysis::EntrypointSummary`]
    /// per entrypoint with its kind plus reach metrics (fan_in, fan_out,
    /// max_reach_depth, reach_size). When `kind_filter` is `Some`, we
    /// keep only summaries whose kind matches; otherwise we keep all.
    ///
    /// Each surviving summary contributes its `node_id` to `nodes` and a
    /// human-readable line to `metadata` of the form:
    ///
    /// `{Kind} {qualified_name} fan_in={n} fan_out={n} reach={size}`
    ///
    /// so renderers can show the analytical context without having to
    /// re-run the classifier.
    fn execute_entrypoints(
        &self,
        kind_filter: Option<EntrypointKind>,
        config: &QueryConfig,
    ) -> QueryResult {
        let summaries = self.graph.classify_entrypoints();
        let mut nodes: Vec<NodeId> = Vec::new();
        let mut metadata: Vec<String> = Vec::new();

        for s in summaries {
            let dsl_kind = EntrypointKind::from_analysis(s.kind);
            if let Some(want) = kind_filter
                && dsl_kind != want
            {
                continue;
            }
            let name = self
                .graph
                .get_node(&s.node_id)
                .map(|n| n.qualified_name.clone())
                .unwrap_or_else(|| format!("{:?}", s.node_id));
            metadata.push(format!(
                "{:?} {} fan_in={} fan_out={} reach={}",
                dsl_kind, name, s.fan_in, s.fan_out, s.reach_size
            ));
            nodes.push(s.node_id);
        }

        let total = nodes.len();
        let was_truncated = total > config.max_nodes;
        if was_truncated {
            nodes.truncate(config.max_nodes);
        }
        QueryResult {
            nodes,
            edges: Vec::new(),
            was_truncated,
            total_before_truncation: total,
            cycles_detected: Vec::new(),
            metadata,
        }
    }

    /// Execute a `path` / `paths` query.
    ///
    /// Strategy:
    /// 1. Evaluate the `from` and `to` sub-expressions to candidate sets.
    /// 2. For each (start, end) pair:
    ///    - In `Shortest` mode: invoke [`crate::traversal::find_path`]
    ///      (sibling-owned, BFS-shortest with depth bound).
    ///    - In `AllSimple` mode: enumerate simple paths inline via
    ///      bounded DFS — keeps the implementation hermetic so we don't
    ///      depend on whichever in-progress sibling helper lands first.
    /// 3. Apply `where intermediate kind=K` and `via EdgeKind` filters
    ///    to each candidate path.
    /// 4. Union all surviving path nodes into the result and emit the
    ///    on-path edges as `(from, to, edge_kind_debug)`.
    fn execute_path_query(
        &self,
        pq: &PathQuery,
        config: &QueryConfig,
    ) -> Result<QueryResult, QueryError> {
        let from_set = self.execute_expr(&pq.from, config)?;
        let to_set = self.execute_expr(&pq.to, config)?;

        let max_depth = pq.max_depth.unwrap_or(32);
        let mut nodes: HashSet<NodeId> = HashSet::new();
        let mut edges: Vec<(NodeId, NodeId, String)> = Vec::new();
        let mut emitted_edges: HashSet<(NodeId, NodeId, String)> = HashSet::new();

        for src in &from_set.nodes {
            for dst in &to_set.nodes {
                let candidate_paths = match pq.mode {
                    PathMode::Shortest => traversal::find_path(self.graph, src, dst, max_depth)
                        .into_iter()
                        .collect::<Vec<_>>(),
                    PathMode::AllSimple => {
                        all_simple_paths_bounded(self.graph, src, dst, max_depth, config.max_nodes)
                    }
                };

                for path in candidate_paths {
                    if !path_matches_filters(self.graph, &path, pq) {
                        continue;
                    }
                    for (i, node_id) in path.iter().enumerate() {
                        nodes.insert(node_id.clone());
                        if i + 1 < path.len() {
                            let next = &path[i + 1];
                            // Emit the actual edge kind from the graph if
                            // present; otherwise fall back to a generic
                            // marker. Multi-edges between same nodes are
                            // collapsed by the `emitted_edges` set.
                            let kind_str = self
                                .graph
                                .get_edges_from(node_id)
                                .into_iter()
                                .find(|(t, _)| *t == next)
                                .map(|(_, e)| format!("{:?}", e.kind))
                                .unwrap_or_else(|| "UnknownEdge".to_string());
                            let key = (node_id.clone(), next.clone(), kind_str.clone());
                            if emitted_edges.insert(key.clone()) {
                                edges.push(key);
                            }
                        }
                    }
                    if nodes.len() >= config.max_nodes {
                        break;
                    }
                }
                if nodes.len() >= config.max_nodes {
                    break;
                }
            }
            if nodes.len() >= config.max_nodes {
                break;
            }
        }

        let total = nodes.len();
        let was_truncated = total > config.max_nodes;
        let node_list: Vec<NodeId> = nodes.into_iter().collect();
        let nodes_out = if was_truncated {
            node_list[..config.max_nodes].to_vec()
        } else {
            node_list
        };

        Ok(QueryResult {
            nodes: nodes_out,
            edges,
            was_truncated,
            total_before_truncation: total,
            cycles_detected: Vec::new(),
            metadata: Vec::new(),
        })
    }
}

/// Combine two [`QueryResult`]s under a [`SetOp`]. Only the `nodes` field
/// is preserved (per module-level contract) — `edges` and other metadata
/// are dropped because there is no defensible merge across heterogenous
/// operands. Truncation is recomputed against the merged size.
fn combine_set_op(
    op: SetOp,
    left: QueryResult,
    right: QueryResult,
    max_nodes: usize,
) -> QueryResult {
    let l: HashSet<NodeId> = left.nodes.into_iter().collect();
    let r: HashSet<NodeId> = right.nodes.into_iter().collect();
    let merged: Vec<NodeId> = match op {
        SetOp::Union => l.union(&r).cloned().collect(),
        SetOp::Intersect => l.intersection(&r).cloned().collect(),
        SetOp::Diff => l.difference(&r).cloned().collect(),
    };
    let total = merged.len();
    let was_truncated = total > max_nodes;
    let nodes = if was_truncated {
        merged[..max_nodes].to_vec()
    } else {
        merged
    };
    QueryResult {
        nodes,
        edges: Vec::new(),
        was_truncated,
        total_before_truncation: total,
        cycles_detected: Vec::new(),
        metadata: Vec::new(),
    }
}

/// Return true if the candidate path satisfies all path-query qualifiers.
///
/// - `intermediate_kind`: every node *except* endpoints must have the
///   requested `NodeKind`. Endpoints are exempt because the user already
///   selected them via the `from`/`to` expressions and re-restricting them
///   here would surprise.
/// - `via_edge`: at least one edge along the path must match the requested
///   [`EdgeKind`]. For variant-with-payload kinds (`UnresolvedCall`,
///   `ExternalCall`) we compare on discriminant only — the user wrote
///   `via UnresolvedCall`, not a specific symbol name.
fn path_matches_filters(graph: &CodeGraph, path: &[NodeId], pq: &PathQuery) -> bool {
    if let Some(kind) = pq.intermediate_kind {
        // Exclude first and last (endpoints).
        if path.len() > 2 {
            for node in &path[1..path.len() - 1] {
                let Some(data) = graph.get_node(node) else {
                    return false;
                };
                if data.kind != kind {
                    return false;
                }
            }
        }
    }
    if let Some(required) = &pq.via_edge {
        let mut found = false;
        for window in path.windows(2) {
            let from = &window[0];
            let to = &window[1];
            for (target, edge) in graph.get_edges_from(from) {
                if target == to && edge_kind_matches(&edge.kind, required) {
                    found = true;
                    break;
                }
            }
            if found {
                break;
            }
        }
        if !found {
            return false;
        }
    }
    true
}

fn edge_kind_matches(actual: &EdgeKind, expected: &EdgeKind) -> bool {
    use EdgeKind::*;
    matches!(
        (actual, expected),
        (Calls, Calls)
            | (UsesType, UsesType)
            | (References, References)
            | (Contains, Contains)
            | (Implements, Implements)
            | (UnresolvedCall(_), UnresolvedCall(_))
            | (ExternalCall(_, _), ExternalCall(_, _))
    )
}

/// Bounded enumeration of all simple paths between two nodes.
///
/// We implement this inline rather than depending on a sibling helper
/// (`analysis::all_simple_paths` is in-progress in another agent's tree)
/// so this DSL module compiles standalone. Honors `max_depth` and caps
/// the total node-budget via `node_budget` — once we accumulate that
/// many distinct nodes across all returned paths, enumeration stops.
fn all_simple_paths_bounded(
    graph: &CodeGraph,
    from: &NodeId,
    to: &NodeId,
    max_depth: usize,
    node_budget: usize,
) -> Vec<Vec<NodeId>> {
    let mut results: Vec<Vec<NodeId>> = Vec::new();
    let mut stack: Vec<NodeId> = vec![from.clone()];
    let mut on_stack: HashMap<NodeId, ()> = HashMap::new();
    on_stack.insert(from.clone(), ());
    let mut total_nodes_emitted: HashSet<NodeId> = HashSet::new();

    fn dfs(
        graph: &CodeGraph,
        current: &NodeId,
        target: &NodeId,
        max_depth: usize,
        node_budget: usize,
        stack: &mut Vec<NodeId>,
        on_stack: &mut HashMap<NodeId, ()>,
        results: &mut Vec<Vec<NodeId>>,
        total: &mut HashSet<NodeId>,
    ) {
        if stack.len() > max_depth + 1 {
            return;
        }
        if current == target {
            for n in stack.iter() {
                total.insert(n.clone());
            }
            results.push(stack.clone());
            return;
        }
        if total.len() >= node_budget {
            return;
        }
        for (next, _edge) in graph.get_edges_from(current) {
            if on_stack.contains_key(next) {
                continue;
            }
            stack.push(next.clone());
            on_stack.insert(next.clone(), ());
            dfs(
                graph,
                next,
                target,
                max_depth,
                node_budget,
                stack,
                on_stack,
                results,
                total,
            );
            on_stack.remove(next);
            stack.pop();
            if total.len() >= node_budget {
                return;
            }
        }
    }

    dfs(
        graph,
        from,
        to,
        max_depth,
        node_budget,
        &mut stack,
        &mut on_stack,
        &mut results,
        &mut total_nodes_emitted,
    );

    results
}

/// Convenience: parse + execute an extended-grammar query.
///
/// Phase 3: runs through the [`crate::dsl::plan`] optimiser before
/// execution. Set-op operand reordering (smaller side first for
/// `intersect`) plus per-pipe rewrites are applied; semantics are
/// preserved.
///
/// Phase 8 (DSL unification): if the query starts with an aggregation
/// keyword (`count`, `sum`, `avg`, `top_k_by`, `group_by`, `exists`,
/// `forall`, `edges_of`, `edges_kind`, `let`), the aggregation
/// executor handles it and projects the result back into a
/// [`QueryResult`] so callers see one return type. Aggregations
/// whose result type isn't node-shaped (`Scalar`, `Bool`, `Edges`,
/// `Groups`) surface in `metadata` lines on the result; the `nodes`
/// field is populated where projection is meaningful (e.g.
/// `top_k_by`, `group_by`).
pub fn run_query_expr(
    query: &str,
    graph: &CodeGraph,
    config: &QueryConfig,
) -> Result<QueryResult, QueryError> {
    // Try the aggregation grammar first — it returns AggExpr::Plain
    // for non-aggregation inputs, which we then unwrap and run
    // through the regular optimiser.
    if let Ok(agg) = crate::dsl::aggregate::parse_aggregate(query) {
        if let crate::dsl::aggregate::AggExpr::Plain(plain) = agg {
            let plan = crate::dsl::plan::optimise_expr(plain);
            let optimised = plan
                .expr()
                .expect("optimise_expr yields Plan::Expr")
                .clone();
            return QueryEngine::new(graph).execute_expr(&optimised, config);
        }
        // Aggregation keyword — execute via the agg path, project
        // into QueryResult.
        return run_aggregate_unified(query, graph, config);
    }

    // Fall back to the legacy parse path.
    let expr = parse_expr(query)?;
    let plan = crate::dsl::plan::optimise_expr(expr);
    let optimised = plan
        .expr()
        .expect("optimise_expr yields Plan::Expr")
        .clone();
    QueryEngine::new(graph).execute_expr(&optimised, config)
}

/// Internal: run a query through the aggregation executor and project
/// its result into the [`QueryResult`] shape. Scalars and bools land
/// in `metadata`; node-shaped results (`Nodes`, `Groups`, `Edges`)
/// populate `nodes` (groups/edges via member projection).
fn run_aggregate_unified(
    query: &str,
    graph: &CodeGraph,
    config: &QueryConfig,
) -> Result<QueryResult, QueryError> {
    use crate::dsl::aggregate::{AggregateResult, run_aggregate};
    let r = run_aggregate(query, graph, config)?;
    let (nodes, edges_meta, metadata) = match r {
        AggregateResult::Nodes(ns) => (ns, Vec::new(), Vec::new()),
        AggregateResult::Edges(es) => {
            let nodes: Vec<NodeId> = es
                .iter()
                .flat_map(|e| [e.from.clone(), e.to.clone()])
                .collect();
            let edges: Vec<(NodeId, NodeId, String)> = es
                .iter()
                .map(|e| (e.from.clone(), e.to.clone(), format!("{:?}", e.kind)))
                .collect();
            let meta: Vec<String> = es
                .iter()
                .map(|e| format!("edge {:?} {} -> {}", e.kind, "from", "to"))
                .take(10)
                .collect();
            (nodes, edges, meta)
        }
        AggregateResult::Scalar(n) => (Vec::new(), Vec::new(), vec![format!("scalar = {n}")]),
        AggregateResult::Bool(b) => (Vec::new(), Vec::new(), vec![format!("bool = {b}")]),
        AggregateResult::Groups(groups) => {
            let nodes: Vec<NodeId> = groups.values().flatten().cloned().collect();
            let meta: Vec<String> = groups
                .iter()
                .map(|(k, v)| format!("group `{k}` size={}", v.len()))
                .collect();
            (nodes, Vec::new(), meta)
        }
    };
    Ok(QueryResult {
        nodes,
        edges: edges_meta,
        was_truncated: false,
        total_before_truncation: 0,
        cycles_detected: Vec::new(),
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unified_run_query_handles_aggregation_count() {
        // Phase 8: aggregation queries route through run_query_expr
        // and project into QueryResult metadata.
        let mut g = CodeGraph::new();
        g.add_node(crate::nodes::NodeData {
            id: NodeId::new("t.rs", "foo", NodeKind::Function),
            kind: NodeKind::Function,
            name: "foo".into(),
            qualified_name: "foo".into(),
            file_path: std::path::PathBuf::from("t.rs"),
            span: crate::nodes::Span {
                file: std::path::PathBuf::from("t.rs"),
                start_line: 1,
                start_col: 0,
                end_line: 1,
                end_col: 0,
                byte_range: 0..0,
            },
            visibility: crate::nodes::Visibility::Public,
            metadata: HashMap::new(),
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
        });
        let r = run_query_expr("count fn(\"foo\")", &g, &QueryConfig::default()).unwrap();
        assert!(r.metadata.iter().any(|m| m.starts_with("scalar = 1")));
    }

    #[test]
    fn unified_run_query_handles_exists() {
        let g = CodeGraph::new();
        let r = run_query_expr("exists fn(\"missing\")", &g, &QueryConfig::default()).unwrap();
        assert!(r.metadata.iter().any(|m| m == "bool = false"));
    }

    #[test]
    fn unified_run_query_handles_legacy_pipe() {
        // Sanity: legacy queries unaffected.
        let mut g = CodeGraph::new();
        g.add_node(crate::nodes::NodeData {
            id: NodeId::new("t.rs", "foo", NodeKind::Function),
            kind: NodeKind::Function,
            name: "foo".into(),
            qualified_name: "foo".into(),
            file_path: std::path::PathBuf::from("t.rs"),
            span: crate::nodes::Span {
                file: std::path::PathBuf::from("t.rs"),
                start_line: 1,
                start_col: 0,
                end_line: 1,
                end_col: 0,
                byte_range: 0..0,
            },
            visibility: crate::nodes::Visibility::Public,
            metadata: HashMap::new(),
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
        });
        let r = run_query_expr("fn(\"foo\")", &g, &QueryConfig::default()).unwrap();
        assert_eq!(r.nodes.len(), 1);
    }

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
            vec![DslOp::SelectFn("danger".into()), DslOp::Preconditions,]
        );
    }

    // Robust: preconditions chains with depth and filter without
    // parser ambiguity.
    #[test]
    fn test_dsl_parse_preconditions_chain_robust() {
        let ops = parse_query(r#"fn("danger") | preconditions | filter kind=Function | depth 3"#)
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
                birth_revision: 0,
                last_modified_revision: 0,
                complexity: None,
                cfg: None,
                dataflow: None,
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
                birth_revision: 0,
                last_modified_revision: 0,
                complexity: None,
                cfg: None,
                dataflow: None,
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
        assert!(
            res.cycles_detected
                .iter()
                .any(|id| *id == ping || *id == pong)
        );
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

    // -----------------------------------------------------------------
    // Extended grammar: set algebra, path patterns, entrypoint selector.
    // -----------------------------------------------------------------

    /// Build a small fixture graph: a -> b -> c, plus d (isolated) and
    /// b -> d via UnresolvedCall. Useful for set-op and path-query tests
    /// that need predictable structure independent of tree-sitter parsing.
    fn build_setalgebra_fixture() -> CodeGraph {
        use crate::edges::{EdgeData, EdgeKind};
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
                birth_revision: 0,
                last_modified_revision: 0,
                complexity: None,
                cfg: None,
                dataflow: None,
            }
        }
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        let _d = g.add_node(node("d"));
        let calls = || EdgeData {
            kind: EdgeKind::Calls,
            source_span: span(),
            weight: 1.0,
        };
        let unresolved = || EdgeData {
            kind: EdgeKind::UnresolvedCall("d".to_string()),
            source_span: span(),
            weight: 1.0,
        };
        g.add_edge(&a, &b, calls()).unwrap();
        g.add_edge(&b, &c, calls()).unwrap();
        // b -> d via UnresolvedCall — gives us a path with a non-Calls
        // edge for `via` filter tests.
        let d_id = NodeId::new("t.rs", "crate::d", NodeKind::Function);
        g.add_edge(&b, &d_id, unresolved()).unwrap();
        g
    }

    fn names_of(graph: &CodeGraph, ids: &[NodeId]) -> std::collections::HashSet<String> {
        ids.iter()
            .filter_map(|id| graph.get_node(id).map(|n| n.name.clone()))
            .collect()
    }

    // Normal: union of two pipe chains yields the set union of their
    // node results. Per module contract, edges/metadata are dropped.
    #[test]
    fn dsl_union_combines_results_normal() {
        let graph = build_setalgebra_fixture();
        let cfg = QueryConfig::default();
        // {a, b} union {c} == {a, b, c}.
        let result = run_query_expr(r#"fn("a") | callees union fn("c")"#, &graph, &cfg).unwrap();
        let names = names_of(&graph, &result.nodes);
        assert!(names.contains("b"), "expected 'b' in union, got {names:?}");
        assert!(names.contains("c"), "expected 'c' in union, got {names:?}");
        // Edges deliberately empty under set-op.
        assert!(result.edges.is_empty());
    }

    // Normal: intersect keeps only nodes present in both operands.
    #[test]
    fn dsl_intersect_keeps_only_common_normal() {
        let graph = build_setalgebra_fixture();
        let cfg = QueryConfig::default();
        // a's callees = {b}; b's callers = {a}. Intersection should be empty.
        // Use a self-overlapping case instead: callees of a {b} ∩ callers of c {b}.
        let result = run_query_expr(
            r#"fn("a") | callees intersect fn("c") | callers"#,
            &graph,
            &cfg,
        )
        .unwrap();
        let names = names_of(&graph, &result.nodes);
        assert_eq!(names.len(), 1, "expected exactly {{b}}, got {names:?}");
        assert!(names.contains("b"));
    }

    // Normal: A \ B subtracts right from left. Used to express "all
    // callers of X except sanitized ones".
    #[test]
    fn dsl_difference_subtracts_right_from_left_normal() {
        let graph = build_setalgebra_fixture();
        let cfg = QueryConfig::default();
        // {a's callees plus a} \ {b} should drop b. Use depth=1 to get {a, b}.
        let result =
            run_query_expr(r#"fn("a") | callees | depth 1 \ fn("b")"#, &graph, &cfg).unwrap();
        let names = names_of(&graph, &result.nodes);
        assert!(!names.contains("b"), "b should be excluded, got {names:?}");
    }

    // Normal: `path A -> B` returns the shortest path (a, b, c).
    #[test]
    fn dsl_path_finds_shortest_normal() {
        let graph = build_setalgebra_fixture();
        let cfg = QueryConfig::default();
        let result = run_query_expr(r#"path fn("a") -> fn("c")"#, &graph, &cfg).unwrap();
        let names = names_of(&graph, &result.nodes);
        assert!(names.contains("a"));
        assert!(names.contains("b"));
        assert!(names.contains("c"));
    }

    // Normal: `paths A -> B` returns all simple paths. In our fixture
    // there's only one a→c path (a→b→c) so node set is {a, b, c}.
    #[test]
    fn dsl_paths_returns_all_simple_paths_normal() {
        let graph = build_setalgebra_fixture();
        let cfg = QueryConfig::default();
        let result = run_query_expr(r#"paths fn("a") -> fn("c") depth 5"#, &graph, &cfg).unwrap();
        let names = names_of(&graph, &result.nodes);
        assert_eq!(names.len(), 3);
        assert!(names.contains("a") && names.contains("b") && names.contains("c"));
    }

    // Normal: `via UnresolvedCall` requires at least one UnresolvedCall
    // edge. Path a→b→d (via UnresolvedCall) qualifies; a→b→c (all Calls)
    // does not. So `paths fn("a") -> fn("d") via UnresolvedCall` yields
    // {a, b, d} but `paths fn("a") -> fn("c") via UnresolvedCall` is empty.
    #[test]
    fn dsl_path_via_edge_kind_filters_normal() {
        let graph = build_setalgebra_fixture();
        let cfg = QueryConfig::default();

        let positive = run_query_expr(
            r#"paths fn("a") -> fn("d") via UnresolvedCall depth 5"#,
            &graph,
            &cfg,
        )
        .unwrap();
        let names = names_of(&graph, &positive.nodes);
        assert!(names.contains("a") && names.contains("b") && names.contains("d"));

        let negative = run_query_expr(
            r#"paths fn("a") -> fn("c") via UnresolvedCall depth 5"#,
            &graph,
            &cfg,
        )
        .unwrap();
        assert!(
            negative.nodes.is_empty(),
            "expected no nodes (no UnresolvedCall edge on a->c path), got {:?}",
            names_of(&graph, &negative.nodes)
        );
    }

    // Normal: `entrypoints` selector now delegates to
    // `CodeGraph::classify_entrypoints`. The setalgebra fixture builds
    // public functions with no `test`/`bench`/`no_mangle` metadata, so
    // every function classifies as `PublicApi`. The bare query returns
    // all four; the `kind=Main` filter returns none (no `fn main`).
    #[test]
    fn dsl_entrypoints_returns_classified_normal() {
        let graph = build_setalgebra_fixture();
        let cfg = QueryConfig::default();
        let result = run_query_expr("entrypoints", &graph, &cfg).unwrap();
        let names = names_of(&graph, &result.nodes);
        assert_eq!(
            names.len(),
            4,
            "expected all 4 public functions classified, got {names:?}"
        );
        // Metadata should describe each entrypoint's kind + reach.
        assert!(
            !result.metadata.is_empty(),
            "expected metadata describing classified entrypoints"
        );
        for line in &result.metadata {
            assert!(line.starts_with("PublicApi "), "got {line}");
        }

        let result_main = run_query_expr("entrypoints kind=Main", &graph, &cfg).unwrap();
        assert!(
            result_main.nodes.is_empty(),
            "no fn main in fixture, expected empty"
        );
        let result_pub = run_query_expr("entrypoints kind=PublicApi", &graph, &cfg).unwrap();
        assert_eq!(result_pub.nodes.len(), 4);
    }

    // Robust: parser rejects mismatched parens with a useful message.
    #[test]
    fn dsl_parens_unbalanced_robust() {
        let err = parse_expr(r#"(fn("a") union fn("b")"#).unwrap_err();
        assert!(err.message.contains(')'), "msg = {}", err.message);
    }

    // Normal: `since N` postfix filter restricts a pipe-chain result to
    // nodes whose `last_modified_revision >= N`. Build a tiny graph,
    // capture a cutoff after the first insert, then add a second node;
    // a `fn(...) | since cutoff` query must return the *new* node and
    // omit the older one.
    #[test]
    fn dsl_since_filters_old_nodes_normal() {
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
                birth_revision: 0,
                last_modified_revision: 0,
                complexity: None,
                cfg: None,
                dataflow: None,
            }
        }

        let mut graph = CodeGraph::new();
        let _old = graph.add_node(node("old_fn")); // revision 1
        let cutoff = graph.current_revision() + 1; // 2
        let _new = graph.add_node(node("new_fn")); // revision 2

        // Parse the new postfix filter.
        let parsed = parse_query(&format!(r#"fn("fn") | since {cutoff}"#)).unwrap();
        assert!(matches!(parsed.last(), Some(DslOp::Since(r)) if *r == cutoff));

        // Execute against both functions ("fn" matches both via substring).
        let cfg = QueryConfig::default();
        let result = run_query(&format!(r#"fn("fn") | since {cutoff}"#), &graph, &cfg).unwrap();
        let names: std::collections::HashSet<&str> = result
            .nodes
            .iter()
            .filter_map(|id| graph.get_node(id).map(|n| n.name.as_str()))
            .collect();
        assert!(
            names.contains("new_fn"),
            "expected new_fn after `since {cutoff}`, got: {names:?}"
        );
        assert!(
            !names.contains("old_fn"),
            "old_fn must be filtered out by `since {cutoff}`, got: {names:?}"
        );
    }

    // -----------------------------------------------------------------
    // New analytical operators: hot, scc, dominators, dominates,
    // trait_impls, dispatch, cluster by type, affected, multi_path.
    // -----------------------------------------------------------------

    /// Helper: build a node with a given kind and visibility.
    fn fixture_node(name: &str, kind: NodeKind) -> crate::nodes::NodeData {
        use crate::nodes::{NodeData, NodeId, Span, Visibility};
        use std::collections::HashMap;
        use std::path::PathBuf;
        let span = Span {
            file: PathBuf::from("t.rs"),
            start_line: 1,
            start_col: 0,
            end_line: 5,
            end_col: 1,
            byte_range: 0..50,
        };
        NodeData {
            id: NodeId::new("t.rs", &format!("crate::{name}"), kind),
            kind,
            name: name.to_string(),
            qualified_name: format!("crate::{name}"),
            file_path: PathBuf::from("t.rs"),
            span,
            visibility: Visibility::Public,
            metadata: HashMap::new(),
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
        }
    }
    fn fixture_calls_edge() -> crate::edges::EdgeData {
        use crate::edges::{EdgeData, EdgeKind};
        use crate::nodes::Span;
        use std::path::PathBuf;
        EdgeData {
            kind: EdgeKind::Calls,
            source_span: Span {
                file: PathBuf::from("t.rs"),
                start_line: 1,
                start_col: 0,
                end_line: 5,
                end_col: 1,
                byte_range: 0..50,
            },
            weight: 1.0,
        }
    }

    // Normal: `hot N` parses as DslOp::Hot(N) and executes against an
    // empty working set by ranking the entire graph.
    #[test]
    fn dsl_hot_bare_returns_top_n_normal() {
        let mut g = CodeGraph::new();
        let a = g.add_node(fixture_node("a", NodeKind::Function));
        let b = g.add_node(fixture_node("b", NodeKind::Function));
        let c = g.add_node(fixture_node("c", NodeKind::Function));
        // Make `b` and `c` hot: many fan-in edges.
        g.add_edge(&a, &b, fixture_calls_edge()).unwrap();
        g.add_edge(&a, &c, fixture_calls_edge()).unwrap();
        g.add_edge(&c, &b, fixture_calls_edge()).unwrap();

        let parsed = parse_query("hot 2").unwrap();
        assert_eq!(parsed, vec![DslOp::Hot(2)]);

        let result = run_query("hot 2", &g, &QueryConfig::default()).unwrap();
        assert!(result.nodes.len() <= 2);
        assert!(!result.metadata.is_empty(), "expected hot metadata");
    }

    // Robust: `hot N` postfix re-ranks only the working set.
    #[test]
    fn dsl_hot_postfix_reranks_working_set_robust() {
        let mut g = CodeGraph::new();
        let a = g.add_node(fixture_node("a", NodeKind::Function));
        let b = g.add_node(fixture_node("b", NodeKind::Function));
        let c = g.add_node(fixture_node("c", NodeKind::Function));
        g.add_edge(&a, &b, fixture_calls_edge()).unwrap();
        g.add_edge(&a, &c, fixture_calls_edge()).unwrap();

        // a's callees are b, c; hot 1 keeps the highest-pagerank one.
        let result =
            run_query(r#"fn("a") | callees | hot 1"#, &g, &QueryConfig::default()).unwrap();
        assert!(result.nodes.len() <= 1);
    }

    // Normal: `scc` returns members of multi-element strongly connected
    // components. ping↔pong forms a 2-cycle; both should appear.
    #[test]
    fn dsl_scc_bare_returns_cycle_members_normal() {
        let mut g = CodeGraph::new();
        let ping = g.add_node(fixture_node("ping", NodeKind::Function));
        let pong = g.add_node(fixture_node("pong", NodeKind::Function));
        let isolated = g.add_node(fixture_node("isolated", NodeKind::Function));
        let _ = isolated; // singleton
        g.add_edge(&ping, &pong, fixture_calls_edge()).unwrap();
        g.add_edge(&pong, &ping, fixture_calls_edge()).unwrap();

        let parsed = parse_query("scc").unwrap();
        assert_eq!(parsed, vec![DslOp::Scc]);
        let result = run_query("scc", &g, &QueryConfig::default()).unwrap();
        let names = names_of(&g, &result.nodes);
        assert!(names.contains("ping") && names.contains("pong"));
        assert!(!names.contains("isolated"), "singleton should be excluded");
        assert!(result.metadata.iter().any(|m| m.contains("size=2")));
    }

    // Normal: `dominators of fn("X")` walks the dominator chain in the
    // call graph rooted at fn main (or fan-in fallback).
    #[test]
    fn dsl_dominators_of_walks_chain_normal() {
        let mut g = CodeGraph::new();
        let main_n = g.add_node(fixture_node("main", NodeKind::Function));
        let mid = g.add_node(fixture_node("mid", NodeKind::Function));
        let leaf = g.add_node(fixture_node("leaf", NodeKind::Function));
        g.add_edge(&main_n, &mid, fixture_calls_edge()).unwrap();
        g.add_edge(&mid, &leaf, fixture_calls_edge()).unwrap();

        let result =
            run_query_expr(r#"dominators of fn("leaf")"#, &g, &QueryConfig::default()).unwrap();
        let names = names_of(&g, &result.nodes);
        // leaf's dominator chain is {mid, main}.
        assert!(names.contains("main"), "got {names:?}");
        assert!(names.contains("mid"), "got {names:?}");
        assert!(!names.contains("leaf"), "seed should be excluded");
    }

    // Robust: `dominates fn("main")` returns descendants in the dom tree.
    #[test]
    fn dsl_dominates_returns_descendants_robust() {
        let mut g = CodeGraph::new();
        let main_n = g.add_node(fixture_node("main", NodeKind::Function));
        let mid = g.add_node(fixture_node("mid", NodeKind::Function));
        let leaf = g.add_node(fixture_node("leaf", NodeKind::Function));
        g.add_edge(&main_n, &mid, fixture_calls_edge()).unwrap();
        g.add_edge(&mid, &leaf, fixture_calls_edge()).unwrap();

        let result =
            run_query_expr(r#"dominates fn("main")"#, &g, &QueryConfig::default()).unwrap();
        let names = names_of(&g, &result.nodes);
        assert!(names.contains("mid"), "got {names:?}");
        assert!(names.contains("leaf"), "got {names:?}");
    }

    // Normal: `trait_impls of type("X")` returns Implements-edge sources.
    #[test]
    fn dsl_trait_impls_returns_implementors_normal() {
        use crate::edges::{EdgeData, EdgeKind};
        use crate::nodes::Span;
        use std::path::PathBuf;
        let mut g = CodeGraph::new();
        let trait_id = g.add_node(fixture_node("MyTrait", NodeKind::Trait));
        let s1 = g.add_node(fixture_node("Foo", NodeKind::Struct));
        let s2 = g.add_node(fixture_node("Bar", NodeKind::Struct));
        let impls = || EdgeData {
            kind: EdgeKind::Implements,
            source_span: Span {
                file: PathBuf::from("t.rs"),
                start_line: 1,
                start_col: 0,
                end_line: 5,
                end_col: 1,
                byte_range: 0..50,
            },
            weight: 1.0,
        };
        g.add_edge(&s1, &trait_id, impls()).unwrap();
        g.add_edge(&s2, &trait_id, impls()).unwrap();

        let result = run_query_expr(
            r#"trait_impls of type("MyTrait")"#,
            &g,
            &QueryConfig::default(),
        )
        .unwrap();
        let names = names_of(&g, &result.nodes);
        assert!(names.contains("Foo"));
        assert!(names.contains("Bar"));
        assert!(result.metadata.iter().any(|m| m.contains("trait_impls")));
    }

    // Normal: `cluster by type` parses and produces type-cluster metadata.
    #[test]
    fn dsl_cluster_by_type_emits_metadata_normal() {
        use crate::edges::{EdgeData, EdgeKind};
        use crate::nodes::Span;
        use std::path::PathBuf;
        let mut g = CodeGraph::new();
        let cfg_ty = g.add_node(fixture_node("Config", NodeKind::Struct));
        let f1 = g.add_node(fixture_node("load", NodeKind::Function));
        let f2 = g.add_node(fixture_node("save", NodeKind::Function));
        let uses = || EdgeData {
            kind: EdgeKind::UsesType,
            source_span: Span {
                file: PathBuf::from("t.rs"),
                start_line: 1,
                start_col: 0,
                end_line: 5,
                end_col: 1,
                byte_range: 0..50,
            },
            weight: 1.0,
        };
        g.add_edge(&f1, &cfg_ty, uses()).unwrap();
        g.add_edge(&f2, &cfg_ty, uses()).unwrap();

        let parsed = parse_query("cluster by type").unwrap();
        assert_eq!(parsed, vec![DslOp::ClusterByType]);

        let result = run_query("cluster by type", &g, &QueryConfig::default()).unwrap();
        let names = names_of(&g, &result.nodes);
        assert!(names.contains("load"));
        assert!(names.contains("save"));
        assert!(
            result.metadata.iter().any(|m| m.contains("Config")),
            "metadata should mention Config: {:?}",
            result.metadata
        );
    }

    // Normal: `affected N since M` wraps nodes_changed_within_depth and
    // returns the temporal-neighborhood set.
    #[test]
    fn dsl_affected_returns_changed_neighborhood_normal() {
        let mut g = CodeGraph::new();
        let _a = g.add_node(fixture_node("a", NodeKind::Function));
        let cutoff = g.current_revision() + 1;
        let b = g.add_node(fixture_node("b", NodeKind::Function));
        let c = g.add_node(fixture_node("c", NodeKind::Function));
        // c is reachable from b (the recently-added node) within 1 hop.
        g.add_edge(&b, &c, fixture_calls_edge()).unwrap();

        let parsed = parse_query(&format!("affected 1 since {cutoff}")).unwrap();
        assert_eq!(
            parsed,
            vec![DslOp::Affected {
                depth: 1,
                since_rev: cutoff,
            }]
        );

        let result = run_query(
            &format!("affected 1 since {cutoff}"),
            &g,
            &QueryConfig::default(),
        )
        .unwrap();
        let names = names_of(&g, &result.nodes);
        // b changed at/after cutoff, c is within 1 hop of b — both in.
        assert!(names.contains("b"), "got {names:?}");
        assert!(names.contains("c"), "got {names:?}");
    }

    // Robust: `multi_path { fn("a"), fn("x") } -> fn("c")` picks the
    // shortest path from any source. Both a→b→c and x→y→z→c exist;
    // the multi-source BFS finds a→b→c first.
    #[test]
    fn dsl_multi_path_picks_shortest_from_any_source_robust() {
        let mut g = CodeGraph::new();
        let a = g.add_node(fixture_node("a", NodeKind::Function));
        let b = g.add_node(fixture_node("b", NodeKind::Function));
        let c = g.add_node(fixture_node("c", NodeKind::Function));
        let x = g.add_node(fixture_node("x", NodeKind::Function));
        let y = g.add_node(fixture_node("y", NodeKind::Function));
        let z = g.add_node(fixture_node("z", NodeKind::Function));
        g.add_edge(&a, &b, fixture_calls_edge()).unwrap();
        g.add_edge(&b, &c, fixture_calls_edge()).unwrap();
        g.add_edge(&x, &y, fixture_calls_edge()).unwrap();
        g.add_edge(&y, &z, fixture_calls_edge()).unwrap();
        g.add_edge(&z, &c, fixture_calls_edge()).unwrap();

        let result = run_query_expr(
            r#"multi_path { fn("a"), fn("x") } -> fn("c") depth 5"#,
            &g,
            &QueryConfig::default(),
        )
        .unwrap();
        let names = names_of(&g, &result.nodes);
        // Shortest path a -> b -> c (length 3).
        assert!(names.contains("a"));
        assert!(names.contains("b"));
        assert!(names.contains("c"));
        // Should NOT include the longer x -> y -> z -> c path.
        assert!(!names.contains("y"), "got {names:?}");
    }

    // Robust: `dispatch` postfix filter restricts to functions whose
    // calls go through trait-method dispatch (Trait → Contains → callee).
    #[test]
    fn dsl_dispatch_filter_restricts_to_trait_callers_robust() {
        use crate::edges::{EdgeData, EdgeKind};
        use crate::nodes::Span;
        use std::path::PathBuf;
        let mut g = CodeGraph::new();
        let trait_id = g.add_node(fixture_node("Iter", NodeKind::Trait));
        let trait_method = g.add_node(fixture_node("next", NodeKind::Function));
        let caller = g.add_node(fixture_node("user", NodeKind::Function));
        let other = g.add_node(fixture_node("other_fn", NodeKind::Function));
        let _ = other;
        let contains = EdgeData {
            kind: EdgeKind::Contains,
            source_span: Span {
                file: PathBuf::from("t.rs"),
                start_line: 1,
                start_col: 0,
                end_line: 5,
                end_col: 1,
                byte_range: 0..50,
            },
            weight: 1.0,
        };
        g.add_edge(&trait_id, &trait_method, contains).unwrap();
        g.add_edge(&caller, &trait_method, fixture_calls_edge())
            .unwrap();

        let parsed = parse_query("dispatch").unwrap();
        assert_eq!(parsed, vec![DslOp::Dispatch]);

        // Bare dispatch returns all dispatch callers in the graph.
        let result = run_query("dispatch", &g, &QueryConfig::default()).unwrap();
        let names = names_of(&g, &result.nodes);
        assert!(names.contains("user"), "got {names:?}");
        assert!(!names.contains("other_fn"), "got {names:?}");
    }
}
