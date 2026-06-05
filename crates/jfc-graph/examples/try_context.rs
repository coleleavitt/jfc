//! Exercise every new context/overlay/schema entry point against the
//! real jfc workspace so we can see actual output (not just unit tests).
//!
//! Run with:
//!     cargo run -p jfc-graph --example try_context

use std::path::PathBuf;

use jfc_graph::context::ContextOptions;
use jfc_graph::session::GraphSession;

fn divider(title: &str) {
    println!("\n{}", "=".repeat(72));
    println!("  {title}");
    println!("{}\n", "=".repeat(72));
}

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf();
    println!("workspace_root = {}", root.display());

    divider("Building GraphSession (this indexes the workspace)");
    let session = GraphSession::from_directory(&root);
    println!(
        "  node_count  = {}\n  parse_errors = {}\n  files_skipped = {}",
        session.graph.node_count(),
        session.parse_errors.len(),
        session.files_skipped.len(),
    );
    if let Some(m) = &session.worktree_mismatch {
        println!("  ⚠ worktree mismatch detected:\n{}", m.message);
    } else {
        println!("  ✓ no worktree mismatch");
    }

    divider("graph_search — `execute_tool`");
    let out = session.search("execute_tool", 5);
    println!("{out}");

    divider("graph_search — qualified `dispatch::execute_tool`");
    let out = session.search("dispatch::execute_tool", 5);
    println!("{out}");

    divider("graph_callers — `execute_tool`");
    let out = session.callers("execute_tool", 8);
    println!("{out}");

    divider("graph_callees — `execute_tool`");
    let out = session.callees("execute_tool", 8);
    println!("{out}");

    divider("graph_impact — `ToolCall` (depth=2)");
    let out = session.impact("ToolCall", 2);
    println!("{out}");

    divider("graph_context — `how does the tool dispatch system work`");
    let opts = ContextOptions {
        max_nodes: 15,
        include_code: false,
        traversal_depth: 1,
        force_expand: false,
    };
    let result = session.context("how does the tool dispatch system work", opts);
    println!("intent       = {:?}", result.intent);
    println!("entry_points = {}", result.entry_points.len());
    println!("related      = {}", result.related.len());
    println!("--- markdown ---");
    println!("{}", result.markdown);

    divider("graph_context — `add a new caching layer`  (feature-intent reminder)");
    let opts = ContextOptions {
        max_nodes: 8,
        include_code: false,
        traversal_depth: 1,
        force_expand: false,
    };
    let result = session.context("add a new caching layer", opts);
    println!("intent       = {:?}", result.intent);
    println!(
        "budget       = max_chars={}, default_max_files={}",
        result.budget.max_output_chars, result.budget.default_max_files,
    );
    println!("--- markdown ---");
    println!("{}", result.markdown);

    divider("schema — wrap a query result");
    let raw = session
        .query_raw("fn(\"execute_tool\")")
        .expect("query_raw");
    let env = jfc_graph::schema::wrap_query_result(raw);
    let json = serde_json::to_string_pretty(&env).expect("serialize");
    let lines: Vec<&str> = json.lines().take(20).collect();
    println!("{}", lines.join("\n"));
    println!("...");

    divider("schema — published JSON Schema for QueryResult");
    let schema_text =
        jfc_graph::schema::json_schema_for(jfc_graph::schema::PayloadKind::QueryResult);
    println!("{schema_text}");

    divider("overlay — save base snapshot and reload");
    let snap_path = std::env::temp_dir().join("jfc-graph-try-base.json");
    session
        .save_for_overlay(&snap_path, &root, Some("HEAD"))
        .expect("save_for_overlay");
    let bytes = std::fs::metadata(&snap_path).map(|m| m.len()).unwrap_or(0);
    println!("wrote {} bytes to {}", bytes, snap_path.display());
    let loaded = jfc_graph::overlay::load_base_snapshot(&snap_path).expect("load");
    println!(
        "loaded: node_count={}, base_ref={:?}, workspace_root={}",
        loaded.graph.node_count(),
        loaded.base_ref,
        loaded.workspace_root.display(),
    );
    let _ = std::fs::remove_file(&snap_path);

    divider("data_dir — resolve for this workspace");
    let dir = jfc_graph::data_dir::resolve_data_dir(&root);
    println!("resolved = {}", dir.display());

    divider("worktree — explicit check (cwd vs workspace root)");
    let cwd = std::env::current_dir().unwrap();
    match jfc_graph::worktree::detect_worktree_index_mismatch(&cwd, &root) {
        Some(m) => println!("⚠ {}", m.message),
        None => println!("✓ caller and index agree on worktree"),
    }

    divider("done");
}
