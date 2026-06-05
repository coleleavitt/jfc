//! Cross-language boundary detection and edge resolution.
//!
//! Detects API boundaries (HTTP routes, gRPC services, FFI bindings, WASM exports)
//! and emits edges that connect nodes across language subgraphs. The reference
//! resolver in [`crate::resolver`] only resolves calls within a single language;
//! this module bridges the gap by treating language boundaries as a typed,
//! match-able surface.
//!
//! # Model
//!
//! A [`Boundary`] is a "provider" side of a cross-language contract:
//! a Rust `#[get("/users/:id")]` handler, a `pub extern "C"` symbol, a
//! `#[wasm_bindgen]` export, etc. Each boundary names *what* it provides
//! ([`BoundaryKind`]) and *which* node provides it (`provider_node`).
//!
//! Boundary detection is a read-only pass: it scans node metadata for
//! convention-driven keys (see "Metadata contract" below) and returns a list
//! of boundaries. It never mutates the graph.
//!
//! Edge emission is a separate step performed by
//! [`resolve_cross_language_calls`]. For each detected boundary, that pass
//! finds call sites in *other* languages that target the same logical contract
//! (e.g. a TypeScript `fetch("/users/42")` call to a Rust `#[get("/users/:id")]`
//! handler) and emits one [`EdgeKind::ExternalCall`] edge per match.
//!
//! # Metadata contract
//!
//! Language adapters communicate boundary information by writing well-known
//! keys into [`NodeData::metadata`]. The keys are intentionally stringly-typed
//! so adapters across crates can populate them without depending on this
//! module's enum:
//!
//! | Key                  | Populated when …                                       | Example value         |
//! |----------------------|--------------------------------------------------------|-----------------------|
//! | `http_route`         | Function is an HTTP route handler                      | `/api/users/:id`      |
//! | `http_method`        | Companion to `http_route`                              | `GET`                 |
//! | `http_client_target` | Function makes an HTTP request to the listed path      | `/api/users/42`       |
//! | `http_client_method` | Companion to `http_client_target` (defaults to `GET`)  | `POST`                |
//! | `ffi_export`         | Function is exported via `#[no_mangle] extern "C"`     | `mylib_add`           |
//! | `wasm_export`        | Function is exported via `#[wasm_bindgen]` or TS `export` | `greet`            |
//! | `wasm_import_module` + `wasm_import_name` | Function is imported from a WASM module | `env`, `console_log` |
//!
//! Detectors in this module never re-parse source — they read metadata only.
//! That keeps the polyglot layer language-agnostic and testable with
//! hand-built graphs (see the unit tests at the bottom of this file).
//!
//! # Path matching
//!
//! HTTP route matching is structural: `/users/:id` matches `/users/42`,
//! `/users/abc`, etc. Query strings on the client target are ignored. The
//! matcher is deliberately conservative — exact segment count, literal
//! segments must match, and a route segment of the form `:name`, `{name}`,
//! or `<name>` matches any non-empty client segment. This keeps false
//! positives low for the foundational pass; richer schemes (regex, wildcard
//! tails) can come later.

use std::path::PathBuf;

use tracing::debug;

use crate::edges::{EdgeData, EdgeKind};
use crate::graph::CodeGraph;
use crate::nodes::{NodeData, NodeId, NodeKind, Span};

/// What kind of cross-language API boundary a node provides.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BoundaryKind {
    /// An HTTP route handler. `path` may contain `:param` / `{param}` / `<param>` placeholders.
    HttpRoute { method: String, path: String },
    /// A gRPC service method (reserved — detection not yet implemented).
    GrpcService { service: String, method: String },
    /// A function exported via the C ABI (`#[no_mangle] extern "C"`).
    FfiExport { symbol: String },
    /// A function exported to WebAssembly (`#[wasm_bindgen]` or TS `export`).
    WasmExport { name: String },
    /// A function imported from another WASM module.
    WasmImport { module: String, name: String },
}

/// A detected cross-language boundary plus the graph node that provides it.
#[derive(Debug, Clone)]
pub struct Boundary {
    pub kind: BoundaryKind,
    /// The graph node that implements / exports this boundary.
    pub provider_node: NodeId,
    /// File path where the boundary lives — kept alongside `provider_node` so
    /// callers can format diagnostics without re-reading the graph.
    pub file_path: PathBuf,
}

// ─── Detection ──────────────────────────────────────────────────────────────

/// Scan every Function node for HTTP-route metadata and return one
/// [`Boundary`] per match.
///
/// Reads `http_route` (path pattern, required) and `http_method`
/// (defaults to `GET` if absent). Method strings are normalised to upper
/// case so detector + matcher always compare on the same casing.
pub fn detect_http_routes(graph: &CodeGraph) -> Vec<Boundary> {
    let mut out = Vec::new();
    for id in graph.all_node_ids() {
        let Some(node) = graph.get_node(id) else {
            continue;
        };
        if node.kind != NodeKind::Function {
            continue;
        }
        let Some(path) = node.metadata.get("http_route") else {
            continue;
        };
        let method = node
            .metadata
            .get("http_method")
            .map(|m| m.to_uppercase())
            .unwrap_or_else(|| "GET".to_string());
        out.push(Boundary {
            kind: BoundaryKind::HttpRoute {
                method,
                path: path.clone(),
            },
            provider_node: id.clone(),
            file_path: node.file_path.clone(),
        });
    }
    debug!(count = out.len(), "detect_http_routes");
    out
}

/// Scan every Function node for FFI-export metadata. A boundary is emitted
/// for any function whose metadata contains `ffi_export` (value = exported
/// symbol name).
pub fn detect_ffi_exports(graph: &CodeGraph) -> Vec<Boundary> {
    let mut out = Vec::new();
    for id in graph.all_node_ids() {
        let Some(node) = graph.get_node(id) else {
            continue;
        };
        if node.kind != NodeKind::Function {
            continue;
        }
        let Some(sym) = node.metadata.get("ffi_export") else {
            continue;
        };
        out.push(Boundary {
            kind: BoundaryKind::FfiExport {
                symbol: sym.clone(),
            },
            provider_node: id.clone(),
            file_path: node.file_path.clone(),
        });
    }
    debug!(count = out.len(), "detect_ffi_exports");
    out
}

/// Scan every Function node for WASM export/import metadata.
///
/// - `wasm_export` ⇒ [`BoundaryKind::WasmExport`].
/// - `wasm_import_module` + `wasm_import_name` ⇒ [`BoundaryKind::WasmImport`].
///   (Both keys required; if only one is present the node is ignored — an
///   import without a module is meaningless.)
pub fn detect_wasm_exports(graph: &CodeGraph) -> Vec<Boundary> {
    let mut out = Vec::new();
    for id in graph.all_node_ids() {
        let Some(node) = graph.get_node(id) else {
            continue;
        };
        if node.kind != NodeKind::Function {
            continue;
        }
        if let Some(name) = node.metadata.get("wasm_export") {
            out.push(Boundary {
                kind: BoundaryKind::WasmExport { name: name.clone() },
                provider_node: id.clone(),
                file_path: node.file_path.clone(),
            });
        }
        if let (Some(module), Some(name)) = (
            node.metadata.get("wasm_import_module"),
            node.metadata.get("wasm_import_name"),
        ) {
            out.push(Boundary {
                kind: BoundaryKind::WasmImport {
                    module: module.clone(),
                    name: name.clone(),
                },
                provider_node: id.clone(),
                file_path: node.file_path.clone(),
            });
        }
    }
    debug!(count = out.len(), "detect_wasm_exports");
    out
}

// ─── Path matching ──────────────────────────────────────────────────────────

/// Returns true if `route_pattern` (e.g. `/users/:id`) matches `client_path`
/// (e.g. `/users/42`). Matching is segment-wise: both must have the same
/// number of `/`-separated parts; literal segments must compare equal; a
/// pattern segment of `:name`, `{name}`, or `<name>` matches any non-empty
/// client segment. Query strings on the client side are stripped before
/// comparison.
fn route_matches(route_pattern: &str, client_path: &str) -> bool {
    // Strip query / fragment from the client path — `?foo=bar` and
    // `#anchor` aren't part of the route surface.
    let client = client_path.split(['?', '#']).next().unwrap_or(client_path);

    // Normalise leading slashes — treat `users/:id` and `/users/:id` the same.
    let pat = route_pattern.trim_start_matches('/');
    let cli = client.trim_start_matches('/');

    let pat_segs: Vec<&str> = pat.split('/').filter(|s| !s.is_empty()).collect();
    let cli_segs: Vec<&str> = cli.split('/').filter(|s| !s.is_empty()).collect();

    if pat_segs.len() != cli_segs.len() {
        return false;
    }

    for (p, c) in pat_segs.iter().zip(cli_segs.iter()) {
        let is_param = (p.starts_with(':'))
            || (p.starts_with('{') && p.ends_with('}'))
            || (p.starts_with('<') && p.ends_with('>'));
        if is_param {
            if c.is_empty() {
                return false;
            }
            continue;
        }
        if p != c {
            return false;
        }
    }
    true
}

// ─── Cross-language edge emission ───────────────────────────────────────────

/// Outcome of one [`resolve_cross_language_calls`] run.
#[derive(Debug, Default, Clone, Copy)]
pub struct PolyglotReport {
    pub boundaries_seen: usize,
    pub clients_seen: usize,
    pub edges_emitted: usize,
    /// Edge insertions that the graph rejected (invariant violation, missing
    /// endpoint, etc.). Logged but not fatal.
    pub edge_errors: usize,
}

/// Snapshot of one HTTP client call site, ready for matching against routes.
struct HttpClientSite {
    caller: NodeId,
    method: String,
    target: String,
    span: Span,
}

/// Collect every Function node carrying `http_client_target` metadata into a
/// snapshot so the matcher can iterate without aliasing the graph.
fn collect_http_clients(graph: &CodeGraph) -> Vec<HttpClientSite> {
    graph
        .all_node_ids()
        .into_iter()
        .filter_map(|id| {
            let node = graph.get_node(id)?;
            if node.kind != NodeKind::Function {
                return None;
            }
            let target = node.metadata.get("http_client_target")?.clone();
            let method = node
                .metadata
                .get("http_client_method")
                .map(|m| m.to_uppercase())
                .unwrap_or_else(|| "GET".to_string());
            Some(HttpClientSite {
                caller: id.clone(),
                method,
                target,
                span: node.span.clone(),
            })
        })
        .collect()
}

/// Walk every Function node carrying `http_client_target` metadata and, for
/// each [`BoundaryKind::HttpRoute`] boundary whose path pattern matches the
/// client target, emit an [`EdgeKind::ExternalCall`] from the client to the
/// boundary's provider.
///
/// The edge's `ExternalCall(crate_name, qualified_name)` payload encodes
/// `crate_name = "http"` and `qualified_name = "{METHOD} {path}"` so
/// downstream consumers can recognise polyglot HTTP edges without having to
/// re-inspect the endpoints.
///
/// Non-HTTP boundary kinds are currently ignored — they will follow once the
/// detectors emit them at scale.
pub fn resolve_cross_language_calls(
    graph: &mut CodeGraph,
    boundaries: &[Boundary],
) -> PolyglotReport {
    let mut report = PolyglotReport {
        boundaries_seen: boundaries.len(),
        ..Default::default()
    };

    // Snapshot HTTP route boundaries so we don't borrow the graph mutably and
    // immutably at the same time.
    let routes: Vec<(String, String, NodeId)> = boundaries
        .iter()
        .filter_map(|b| match &b.kind {
            BoundaryKind::HttpRoute { method, path } => {
                Some((method.to_uppercase(), path.clone(), b.provider_node.clone()))
            }
            _ => None,
        })
        .collect();

    if routes.is_empty() {
        return report;
    }

    let clients = collect_http_clients(graph);
    report.clients_seen = clients.len();

    for client in clients {
        for (route_method, route_path, provider_id) in &routes {
            if &client.method != route_method {
                continue;
            }
            // Don't link a node to itself — a route handler that also
            // annotates `http_client_target` would otherwise self-edge.
            if &client.caller == provider_id {
                continue;
            }
            if !route_matches(route_path, &client.target) {
                continue;
            }

            let edge = EdgeData {
                kind: EdgeKind::ExternalCall(
                    "http".to_string(),
                    format!("{route_method} {route_path}"),
                ),
                source_span: client.span.clone(),
                weight: 1.0,
            };
            match graph.add_edge(&client.caller, provider_id, edge) {
                Ok(()) => report.edges_emitted += 1,
                Err(e) => {
                    debug!(error = %e, "polyglot edge insert failed");
                    report.edge_errors += 1;
                }
            }
        }
    }

    report
}

// `NodeData` is used in doc-link only; suppress unused-import lint if doc
// links collapse.
#[allow(dead_code)]
fn _doc_anchor(_: &NodeData) {}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::nodes::{NodeData, NodeId, NodeKind, Visibility};

    fn span_for(file: &str) -> Span {
        Span {
            file: PathBuf::from(file),
            start_line: 1,
            start_col: 0,
            end_line: 10,
            end_col: 0,
            byte_range: 0..100,
        }
    }

    fn make_fn(file: &str, qname: &str, metadata: HashMap<String, String>) -> NodeData {
        let id = NodeId::new(file, qname, NodeKind::Function);
        NodeData {
            id,
            kind: NodeKind::Function,
            name: qname.rsplit("::").next().unwrap_or(qname).to_string(),
            qualified_name: qname.to_string(),
            file_path: PathBuf::from(file),
            span: span_for(file),
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
    fn route_matcher_handles_params_and_literals() {
        assert!(route_matches("/users/:id", "/users/42"));
        assert!(route_matches("/users/{id}", "/users/abc"));
        assert!(route_matches("/users/<id>", "/users/xyz"));
        assert!(route_matches("/a/:x/b/:y", "/a/1/b/2"));
        // Query string stripped.
        assert!(route_matches("/api/items", "/api/items?page=2"));
        // Trailing slashes normalised.
        assert!(route_matches("users/:id", "/users/9"));

        // Negative cases.
        assert!(!route_matches("/users/:id", "/users"));
        assert!(!route_matches("/users/:id", "/users/42/extra"));
        assert!(!route_matches("/users/:id", "/orders/42"));
        // Empty client segment doesn't match a param.
        assert!(!route_matches("/users/:id", "/users/"));
    }

    #[test]
    fn detect_http_routes_finds_metadata_tagged_handlers() {
        let mut g = CodeGraph::new();
        let mut md = HashMap::new();
        md.insert("http_route".to_string(), "/users/:id".to_string());
        md.insert("http_method".to_string(), "get".to_string());
        let handler = make_fn("src/api.rs", "crate::api::get_user", md);
        let handler_id = g.add_node(handler);

        // A non-handler function should be ignored.
        g.add_node(make_fn(
            "src/util.rs",
            "crate::util::helper",
            HashMap::new(),
        ));

        let boundaries = detect_http_routes(&g);
        assert_eq!(boundaries.len(), 1);
        let b = &boundaries[0];
        assert_eq!(b.provider_node, handler_id);
        match &b.kind {
            BoundaryKind::HttpRoute { method, path } => {
                // Method must be normalised to uppercase.
                assert_eq!(method, "GET");
                assert_eq!(path, "/users/:id");
            }
            other => panic!("expected HttpRoute, got {other:?}"),
        }
    }

    #[test]
    fn resolve_cross_language_calls_emits_external_call_edge() {
        let mut g = CodeGraph::new();

        // Rust handler: GET /users/:id
        let mut handler_md = HashMap::new();
        handler_md.insert("http_route".to_string(), "/users/:id".to_string());
        handler_md.insert("http_method".to_string(), "GET".to_string());
        let handler_id = g.add_node(make_fn("src/api.rs", "crate::api::get_user", handler_md));

        // TypeScript client calling /users/42
        let mut client_md = HashMap::new();
        client_md.insert("http_client_target".to_string(), "/users/42".to_string());
        client_md.insert("http_client_method".to_string(), "GET".to_string());
        let client_id = g.add_node(make_fn("web/src/user.ts", "user::fetchUser", client_md));

        // Unrelated function — should not get an edge.
        let _other = g.add_node(make_fn(
            "src/other.rs",
            "crate::other::noop",
            HashMap::new(),
        ));

        let boundaries = detect_http_routes(&g);
        assert_eq!(boundaries.len(), 1);

        let report = resolve_cross_language_calls(&mut g, &boundaries);
        assert_eq!(report.boundaries_seen, 1);
        assert_eq!(report.clients_seen, 1);
        assert_eq!(report.edges_emitted, 1);
        assert_eq!(report.edge_errors, 0);

        // The edge must exist from client → handler with the encoded payload.
        let outgoing = g.get_edges_from(&client_id);
        let polyglot_edges: Vec<_> = outgoing
            .iter()
            .filter(|(_, e)| matches!(&e.kind, EdgeKind::ExternalCall(c, _) if c == "http"))
            .collect();
        assert_eq!(polyglot_edges.len(), 1);
        let (target, edge) = polyglot_edges[0];
        assert_eq!(**target, handler_id);
        match &edge.kind {
            EdgeKind::ExternalCall(c, q) => {
                assert_eq!(c, "http");
                assert_eq!(q, "GET /users/:id");
            }
            other => panic!("unexpected edge kind: {other:?}"),
        }
    }

    #[test]
    fn method_mismatch_blocks_edge_emission() {
        let mut g = CodeGraph::new();
        let mut handler_md = HashMap::new();
        handler_md.insert("http_route".to_string(), "/items".to_string());
        handler_md.insert("http_method".to_string(), "POST".to_string());
        g.add_node(make_fn("src/api.rs", "crate::api::create", handler_md));

        let mut client_md = HashMap::new();
        client_md.insert("http_client_target".to_string(), "/items".to_string());
        // Client uses GET, handler is POST — no edge expected.
        client_md.insert("http_client_method".to_string(), "GET".to_string());
        g.add_node(make_fn("web/src/list.ts", "list::loadItems", client_md));

        let boundaries = detect_http_routes(&g);
        let report = resolve_cross_language_calls(&mut g, &boundaries);
        assert_eq!(report.edges_emitted, 0);
    }
}
