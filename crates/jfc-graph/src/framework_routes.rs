//! Web-framework route detection — jfc-graph parity with codegraph's `route`
//! nodes.
//!
//! codegraph emits dedicated `route` nodes for web-framework route
//! registrations (k8s 57, terraform 34 in the reference study); jfc-graph had
//! none. Rather than add a `Route` variant to [`crate::nodes::NodeKind`] —
//! which would ripple through the ~950 exhaustive matches on that enum across
//! the crate — this follows the convention already used by
//! [`crate::coverage::CoveragePass`] and
//! [`crate::possible_types::PossibleTypesPass`]: a pass scans source for route
//! declarations and annotates the *handler* `Function` node with `route.*`
//! metadata. The pure [`detect_routes`] function returns the structured
//! [`Route`] list for callers that want the routing table directly.
//!
//! Detection is line-based and dependency-free (no regex), covering the common
//! declaration shapes across ecosystems:
//!
//! - **Rust attribute macros** (actix-web / rocket): `#[get("/users")]`
//! - **Rust axum**: `.route("/users", get(handler))`
//! - **Python decorators** (FastAPI / Flask): `@app.get("/users")`,
//!   `@app.route("/users", methods=["POST"])`
//! - **JS/TS express**: `app.get("/users", handler)`, `router.post("/x", h)`
//!
//! It is deliberately a heuristic — it reads declarations, not semantics — so
//! it favours precision (a quoted path argument is required) over recall.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::graph::CodeGraph;
use crate::nodes::NodeKind;

/// HTTP method of a detected route. `Any` is used for declarations that match
/// a route but don't pin a verb (e.g. Flask `@app.route("/x")` with no
/// `methods=`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
    Head,
    Options,
    Any,
}

impl HttpMethod {
    fn parse(s: &str) -> Option<HttpMethod> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "get" => HttpMethod::Get,
            "post" => HttpMethod::Post,
            "put" => HttpMethod::Put,
            "delete" => HttpMethod::Delete,
            "patch" => HttpMethod::Patch,
            "head" => HttpMethod::Head,
            "options" => HttpMethod::Options,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Delete => "DELETE",
            HttpMethod::Patch => "PATCH",
            HttpMethod::Head => "HEAD",
            HttpMethod::Options => "OPTIONS",
            HttpMethod::Any => "ANY",
        }
    }
}

/// The declaration *style* a route was detected from. We name by style rather
/// than guessing the exact crate/library, since several share a shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteStyle {
    /// Rust attribute macro: `#[get("/p")]` (actix-web, rocket).
    RustAttr,
    /// Rust axum builder: `.route("/p", get(handler))`.
    Axum,
    /// Python decorator: `@app.get("/p")` / `@app.route("/p", methods=[..])`.
    PyDecorator,
    /// JS/TS express-style: `app.get("/p", handler)`.
    Express,
}

impl RouteStyle {
    pub fn as_str(self) -> &'static str {
        match self {
            RouteStyle::RustAttr => "rust-attr",
            RouteStyle::Axum => "axum",
            RouteStyle::PyDecorator => "python-decorator",
            RouteStyle::Express => "express",
        }
    }
}

/// A detected route declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub style: RouteStyle,
    pub method: HttpMethod,
    pub path: String,
    pub file: PathBuf,
    /// 1-based line of the declaration.
    pub line: u32,
}

const VERBS: [&str; 7] = ["get", "post", "put", "delete", "patch", "head", "options"];

/// Return the first single- or double-quoted string literal in `s`, plus the
/// byte offset just past its closing quote. Escapes are not interpreted (route
/// paths don't contain escaped quotes); slice boundaries land on the ASCII
/// quote bytes, so UTF-8 in the path is preserved correctly.
fn first_string_literal(s: &str) -> Option<(&str, usize)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'"' || c == b'\'' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() {
                if bytes[j] == c {
                    return Some((&s[start..j], j + 1));
                }
                j += 1;
            }
            return None; // unterminated literal
        }
        i += 1;
    }
    None
}

/// Earliest `verb(` occurrence in `s` (used to recover the method from an axum
/// `.route("/p", get(handler))` tail).
fn first_method_call(s: &str) -> Option<HttpMethod> {
    let lower = s.to_ascii_lowercase();
    let mut best: Option<(usize, HttpMethod)> = None;
    for v in VERBS {
        if let Some(pos) = lower.find(&format!("{v}(")) {
            if best.is_none_or(|(b, _)| pos < b) {
                best = Some((pos, HttpMethod::parse(v).unwrap()));
            }
        }
    }
    best.map(|(_, m)| m)
}

/// Flask `methods=["POST", ...]` → the first listed method, if any.
fn flask_method(body: &str) -> Option<HttpMethod> {
    let pos = body.find("methods")?;
    let (m, _) = first_string_literal(&body[pos..])?;
    HttpMethod::parse(m)
}

/// express `app.get("/p", ...)` → `(method, path)` when a method-call with a
/// leading quoted-string argument is present.
fn express_route(t: &str) -> Option<(HttpMethod, String)> {
    let lower = t.to_ascii_lowercase();
    let mut best: Option<(usize, &str)> = None;
    for v in VERBS {
        if let Some(pos) = lower.find(&format!(".{v}(")) {
            if best.is_none_or(|(b, _)| pos < b) {
                best = Some((pos, v));
            }
        }
    }
    let (dot_pos, verb) = best?;
    // Skip past ".verb(" to the argument list.
    let args = &t[dot_pos + 1 + verb.len() + 1..];
    let trimmed = args.trim_start();
    if !(trimmed.starts_with('"') || trimmed.starts_with('\'')) {
        // Not a route registration — e.g. `map.get(0)` or `vec.get(i)`.
        return None;
    }
    let (path, _) = first_string_literal(trimmed)?;
    Some((HttpMethod::parse(verb)?, path.to_string()))
}

/// Detect web-framework route declarations in a single source file. Pure and
/// dependency-free; the heavy lifting / correctness lives here and is unit
/// tested directly, while [`annotate_graph_with_routes`] wires it to the graph.
pub fn detect_routes(file: &Path, source: &str) -> Vec<Route> {
    let mut out = Vec::new();
    for (idx, raw) in source.lines().enumerate() {
        let line = (idx + 1) as u32;
        let t = raw.trim();
        if t.is_empty() {
            continue;
        }
        // Skip comments. `#[` is a Rust attribute, not a Python comment.
        if t.starts_with("//") || t.starts_with('*') || (t.starts_with('#') && !t.starts_with("#["))
        {
            continue;
        }

        // 1. Rust attribute macro: `#[get("/p")]`, `#[actix_web::post("/p")]`.
        if let Some(inner) = t.strip_prefix("#[") {
            let head = inner.split('(').next().unwrap_or("");
            let name_seg = head.rsplit("::").next().unwrap_or(head);
            if let Some(method) = HttpMethod::parse(name_seg) {
                if let Some((path, _)) = first_string_literal(inner) {
                    out.push(Route {
                        style: RouteStyle::RustAttr,
                        method,
                        path: path.to_string(),
                        file: file.to_path_buf(),
                        line,
                    });
                    continue;
                }
            }
        }

        // 2. Python decorator: `@app.get("/p")`, `@app.route("/p", methods=[..])`.
        if let Some(body) = t.strip_prefix('@') {
            if let Some(paren) = body.find('(') {
                let target = &body[..paren];
                let method_seg = target.rsplit('.').next().unwrap_or(target);
                let parsed = HttpMethod::parse(method_seg);
                if parsed.is_some() || method_seg == "route" {
                    if let Some((path, _)) = first_string_literal(body) {
                        let method = parsed.or_else(|| flask_method(body)).unwrap_or(HttpMethod::Any);
                        out.push(Route {
                            style: RouteStyle::PyDecorator,
                            method,
                            path: path.to_string(),
                            file: file.to_path_buf(),
                            line,
                        });
                        continue;
                    }
                }
            }
        }

        // 3. Rust axum: `.route("/p", get(handler))`.
        if let Some(rpos) = t.find(".route(") {
            let after = &t[rpos + ".route(".len()..];
            if let Some((path, end)) = first_string_literal(after) {
                let method = first_method_call(&after[end..]).unwrap_or(HttpMethod::Any);
                out.push(Route {
                    style: RouteStyle::Axum,
                    method,
                    path: path.to_string(),
                    file: file.to_path_buf(),
                    line,
                });
                continue;
            }
        }

        // 4. express: `app.get("/p", handler)` (requires a quoted path arg).
        if let Some((method, path)) = express_route(t) {
            out.push(Route {
                style: RouteStyle::Express,
                method,
                path,
                file: file.to_path_buf(),
                line,
            });
            continue;
        }
    }
    out
}

/// Whether a style declares the route *above* its handler function (decorators
/// / attribute macros) vs. *inside* a builder call (axum / express, where the
/// handler is referenced, not adjacent).
fn handler_is_below(style: RouteStyle) -> bool {
    matches!(style, RouteStyle::RustAttr | RouteStyle::PyDecorator)
}

/// Detect routes across every source file referenced by the graph's `Function`
/// nodes, and annotate the handler node of each route with `route.method`,
/// `route.path`, and `route.framework` metadata. Returns the number of routes
/// successfully attached to a handler node.
///
/// File paths are resolved exactly like [`crate::coverage`]: tried as-is, then
/// under `project_root`. Files that can't be read are skipped (never panics on
/// missing source).
pub fn annotate_graph_with_routes(graph: &mut CodeGraph, project_root: &Path) -> usize {
    // Group function spans by file so each file is read at most once.
    let mut by_file: HashMap<PathBuf, Vec<(crate::nodes::NodeId, u32, u32)>> = HashMap::new();
    for n in graph.nodes_by_kind(NodeKind::Function) {
        by_file
            .entry(n.file_path.clone())
            .or_default()
            .push((n.id.clone(), n.span.start_line, n.span.end_line));
    }

    let mut annotated = 0usize;
    for (file, mut fns) in by_file {
        // Stable order so "nearest handler" ties resolve deterministically.
        fns.sort_by_key(|(_, start, _)| *start);

        let Some(source) = read_source(&file, project_root) else {
            continue;
        };

        for route in detect_routes(&file, &source) {
            let handler = if handler_is_below(route.style) {
                // The function declared at or just after the route line.
                fns.iter()
                    .filter(|(_, start, _)| *start >= route.line)
                    .min_by_key(|(_, start, _)| *start - route.line)
                    .map(|(id, _, _)| id.clone())
            } else {
                // The function whose span encloses the registration call.
                fns.iter()
                    .find(|(_, start, end)| *start <= route.line && route.line <= *end)
                    .map(|(id, _, _)| id.clone())
            };

            if let Some(id) = handler {
                graph.update_node_metadata(&id, |meta| {
                    meta.insert("route.method".into(), route.method.as_str().into());
                    meta.insert("route.path".into(), route.path.clone());
                    meta.insert("route.framework".into(), route.style.as_str().into());
                });
                annotated += 1;
            }
        }
    }

    tracing::info!(annotated, "framework-routes pass complete");
    annotated
}

fn read_source(file: &Path, project_root: &Path) -> Option<String> {
    std::fs::read_to_string(file)
        .or_else(|_| std::fs::read_to_string(project_root.join(file)))
        .ok()
}

/// [`crate::pass::Pass`] wrapper for [`annotate_graph_with_routes`].
pub struct FrameworkRoutesPass {
    project_root: PathBuf,
}

impl FrameworkRoutesPass {
    pub fn new(project_root: PathBuf) -> Self {
        Self { project_root }
    }
}

impl crate::pass::Pass for FrameworkRoutesPass {
    fn name(&self) -> &'static str {
        "framework-routes-detect"
    }

    fn requires(&self) -> &'static [crate::pass::GraphFlag] {
        &[crate::pass::GraphFlag::TreeParsed]
    }

    fn establishes(&self) -> &'static [crate::pass::GraphFlag] {
        &[crate::pass::GraphFlag::FrameworkRoutesDetected]
    }

    fn run(&self, graph: &mut CodeGraph) -> Result<(), crate::pass::PassError> {
        annotate_graph_with_routes(graph, &self.project_root);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::{NodeData, NodeId, Span, Visibility};

    fn paths(routes: &[Route]) -> Vec<(&str, &str, &str)> {
        routes
            .iter()
            .map(|r| (r.style.as_str(), r.method.as_str(), r.path.as_str()))
            .collect()
    }

    // Normal: Rust attribute macros (actix/rocket), including a path-qualified
    // form, are detected with the right verb + path.
    #[test]
    fn detects_rust_attribute_routes_normal() {
        let src = r#"
#[get("/users")]
async fn list_users() {}

#[actix_web::post("/users")]
async fn create_user() {}
"#;
        let routes = detect_routes(Path::new("h.rs"), src);
        assert_eq!(
            paths(&routes),
            vec![
                ("rust-attr", "GET", "/users"),
                ("rust-attr", "POST", "/users"),
            ]
        );
    }

    // Normal: axum builder routes recover the verb from the `get(handler)` arg.
    #[test]
    fn detects_axum_routes_normal() {
        let src = r#"
let app = Router::new()
    .route("/health", get(health))
    .route("/items", post(create_item));
"#;
        let routes = detect_routes(Path::new("main.rs"), src);
        assert_eq!(
            paths(&routes),
            vec![("axum", "GET", "/health"), ("axum", "POST", "/items")]
        );
    }

    // Normal + robust: FastAPI/Flask decorators, including `@app.route` with a
    // `methods=` list and the bare form that falls back to ANY.
    #[test]
    fn detects_python_decorator_routes_robust() {
        let src = r#"
@app.get("/ping")
def ping(): ...

@app.route("/submit", methods=["POST"])
def submit(): ...

@app.route("/any")
def any_handler(): ...
"#;
        let routes = detect_routes(Path::new("api.py"), src);
        assert_eq!(
            paths(&routes),
            vec![
                ("python-decorator", "GET", "/ping"),
                ("python-decorator", "POST", "/submit"),
                ("python-decorator", "ANY", "/any"),
            ]
        );
    }

    // Normal: express-style registrations on `app` and `router`.
    #[test]
    fn detects_express_routes_normal() {
        let src = r#"
app.get("/", home);
router.post('/login', login);
"#;
        let routes = detect_routes(Path::new("server.js"), src);
        assert_eq!(
            paths(&routes),
            vec![("express", "GET", "/"), ("express", "POST", "/login")]
        );
    }

    // Robust: heuristic must NOT flag non-route method calls or commented code.
    #[test]
    fn ignores_non_route_calls_and_comments_robust() {
        let src = r#"
let x = map.get(0);          // numeric arg, not a path
let y = cache.delete(key);   // identifier arg, not a string
// app.get("/commented-out", h);
# @app.get("/python-comment")
let v = items.get("key");    // map lookup with a string — acceptable edge
"#;
        let routes = detect_routes(Path::new("x.rs"), src);
        // The only string-arg method call is `items.get("key")`, which the
        // heuristic does flag (documented limitation); the numeric/identifier
        // calls and both comment forms are correctly ignored.
        assert_eq!(paths(&routes), vec![("express", "GET", "key")]);
    }

    fn mk_fn(name: &str, file: &str, start: u32, end: u32) -> NodeData {
        NodeData {
            id: NodeId::new(file, name, NodeKind::Function),
            kind: NodeKind::Function,
            name: name.to_string(),
            qualified_name: name.to_string(),
            file_path: PathBuf::from(file),
            span: Span {
                file: PathBuf::from(file),
                start_line: start,
                start_col: 0,
                end_line: end,
                end_col: 0,
                byte_range: 0..0,
            },
            visibility: Visibility::Public,
            metadata: HashMap::new(),
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
        }
    }

    // Normal: end-to-end — a decorator above a handler annotates that handler
    // node's metadata with the route fields.
    #[test]
    fn annotates_handler_node_metadata_normal() {
        let dir = std::env::temp_dir().join(format!("jfc_routes_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("api.py");
        std::fs::write(
            &file,
            "@app.get(\"/ping\")\ndef ping():\n    return 'pong'\n",
        )
        .unwrap();

        let mut graph = CodeGraph::new();
        // Handler `ping` starts on line 2, right under the decorator on line 1.
        graph.add_node(mk_fn("ping", file.to_str().unwrap(), 2, 3));

        let annotated = annotate_graph_with_routes(&mut graph, Path::new("/"));
        assert_eq!(annotated, 1);

        let id = NodeId::new(file.to_str().unwrap(), "ping", NodeKind::Function);
        let node = graph.get_node(&id).unwrap();
        assert_eq!(node.metadata.get("route.method").unwrap(), "GET");
        assert_eq!(node.metadata.get("route.path").unwrap(), "/ping");
        assert_eq!(node.metadata.get("route.framework").unwrap(), "python-decorator");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
