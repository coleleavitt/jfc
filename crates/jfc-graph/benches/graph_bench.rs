//! Criterion benchmarks for `jfc-graph` (Phase 16).
//!
//! Run with: `cargo bench -p jfc-graph`
//!
//! Benchmarks:
//! - CSR snapshot build from a synthetic graph.
//! - BFS (petgraph vs CSR serial vs CSR parallel).
//! - Tarjan articulation points (linear-time).
//! - DSL eval with optimizer.
//! - QueryCache hit vs miss.

use std::collections::HashMap;
use std::path::PathBuf;

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use jfc_graph::csr::CsrSnapshot;
use jfc_graph::dsl::{QueryConfig, run_query_expr};
use jfc_graph::edges::{EdgeData, EdgeKind};
use jfc_graph::graph::CodeGraph;
use jfc_graph::nodes::{NodeData, NodeId, NodeKind, Span, Visibility};
use jfc_graph::traversal::{
    TraversalConfig, TraversalDirection, traverse, traverse_csr, traverse_petgraph,
};

fn span() -> Span {
    Span {
        file: PathBuf::from("bench.rs"),
        start_line: 1,
        start_col: 0,
        end_line: 1,
        end_col: 0,
        byte_range: 0..0,
    }
}

fn mk(name: &str) -> NodeData {
    NodeData {
        id: NodeId::new("bench.rs", name, NodeKind::Function),
        kind: NodeKind::Function,
        name: name.to_string(),
        qualified_name: name.to_string(),
        file_path: PathBuf::from("bench.rs"),
        span: span(),
        visibility: Visibility::Public,
        metadata: HashMap::new(),
        birth_revision: 0,
        last_modified_revision: 0,
    }
}

fn ed() -> EdgeData {
    EdgeData {
        kind: EdgeKind::Calls,
        source_span: span(),
        weight: 1.0,
    }
}

/// Build a chain graph of `n` nodes with edges: 0→1→2→…→(n-1).
fn chain_graph(n: usize) -> (CodeGraph, Vec<NodeId>) {
    let mut g = CodeGraph::new();
    let mut ids = Vec::with_capacity(n);
    for i in 0..n {
        ids.push(g.add_node(mk(&format!("n{i}"))));
    }
    for i in 0..n.saturating_sub(1) {
        let _ = g.add_edge(&ids[i], &ids[i + 1], ed());
    }
    (g, ids)
}

/// Build a fan-out graph: root → n leaves, each leaf → sink.
fn fan_graph(n: usize) -> (CodeGraph, NodeId) {
    let mut g = CodeGraph::new();
    let root = g.add_node(mk("root"));
    let sink = g.add_node(mk("sink"));
    for i in 0..n {
        let leaf = g.add_node(mk(&format!("leaf{i}")));
        let _ = g.add_edge(&root, &leaf, ed());
        let _ = g.add_edge(&leaf, &sink, ed());
    }
    (g, root)
}

fn bench_csr_build(c: &mut Criterion) {
    let (g, _) = chain_graph(5000);
    c.bench_function("csr_build_5k_chain", |b| {
        b.iter(|| black_box(CsrSnapshot::build(&g)));
    });
}

fn bench_bfs_petgraph(c: &mut Criterion) {
    let (g, ids) = chain_graph(2000);
    let cfg = TraversalConfig {
        max_depth: 100,
        max_nodes: 2000,
        direction: TraversalDirection::Outgoing,
        parallel: false,
    };
    c.bench_function("bfs_petgraph_2k_chain", |b| {
        b.iter(|| black_box(traverse_petgraph(&g, &ids[0], &cfg)));
    });
}

fn bench_bfs_csr_serial(c: &mut Criterion) {
    let (g, ids) = chain_graph(2000);
    let snap = g.snapshot();
    let cfg = TraversalConfig {
        max_depth: 100,
        max_nodes: 2000,
        direction: TraversalDirection::Outgoing,
        parallel: false,
    };
    c.bench_function("bfs_csr_serial_2k_chain", |b| {
        b.iter(|| black_box(traverse_csr(&snap, &ids[0], &cfg)));
    });
}

fn bench_bfs_csr_parallel(c: &mut Criterion) {
    let (g, root) = fan_graph(2000);
    let snap = g.snapshot();
    let cfg = TraversalConfig {
        max_depth: 10,
        max_nodes: 5000,
        direction: TraversalDirection::Outgoing,
        parallel: true,
    };
    c.bench_function("bfs_csr_parallel_2k_fan", |b| {
        b.iter(|| black_box(traverse_csr(&snap, &root, &cfg)));
    });
}

fn bench_tarjan_articulation(c: &mut Criterion) {
    let (g, _) = chain_graph(2000);
    c.bench_function("tarjan_articulation_2k_chain", |b| {
        b.iter(|| black_box(jfc_graph::analysis::critical_nodes(&g)));
    });
}

fn bench_dsl_eval(c: &mut Criterion) {
    let (g, _) = chain_graph(500);
    let cfg = QueryConfig::default();
    c.bench_function("dsl_eval_fn_callers_depth3", |b| {
        b.iter(|| {
            black_box(
                run_query_expr(r#"fn("n0") | callees | depth 3"#, &g, &cfg).unwrap(),
            )
        });
    });
}

fn bench_query_cache_hit(c: &mut Criterion) {
    use jfc_graph::incremental::{QueryCache, QueryKey, ReadSet};
    let cache: QueryCache<Vec<u32>> = QueryCache::new();
    let key = QueryKey::new("test_query", 1);
    cache.put(key.clone(), vec![1, 2, 3], ReadSet::new());
    c.bench_function("query_cache_hit", |b| {
        b.iter(|| black_box(cache.get(&key)));
    });
}

fn bench_query_cache_miss(c: &mut Criterion) {
    use jfc_graph::incremental::{QueryCache, QueryKey};
    let cache: QueryCache<Vec<u32>> = QueryCache::new();
    let key = QueryKey::new("nonexistent", 999);
    c.bench_function("query_cache_miss", |b| {
        b.iter(|| black_box(cache.get(&key)));
    });
}

criterion_group!(
    benches,
    bench_csr_build,
    bench_bfs_petgraph,
    bench_bfs_csr_serial,
    bench_bfs_csr_parallel,
    bench_tarjan_articulation,
    bench_dsl_eval,
    bench_query_cache_hit,
    bench_query_cache_miss,
);
criterion_main!(benches);
