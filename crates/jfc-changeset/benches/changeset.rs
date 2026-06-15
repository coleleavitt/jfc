//! Criterion benchmarks for the changeset + ledger hot persistence paths.
//!
//! These are the same workloads as the `perf_baseline` regression-gate tests
//! (crates/jfc-changeset/tests/perf_baseline.rs) but wired through criterion so
//! CodSpeed can track per-commit throughput and surface regressions visually.
//!
//! Workloads:
//!   - `changeset_upsert` — 200 isolated `ChangeStore::upsert` calls (each is a
//!     full locked JSONL rewrite), the write hot-path every agent run exercises.
//!   - `ledger_append` — 2 000 `LedgerStore::append` calls, the append-only O(1)
//!     per-event write path.
//!   - `ledger_query` — filtered query over a 2 000-event store (10% selectivity).

use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use jfc_changeset::{
    AgentChangeSet, ChangeStore, EventKind, LedgerEvent, LedgerFilter, LedgerStore,
};
use tempfile::TempDir;

fn bench_changeset_upsert(c: &mut Criterion) {
    c.bench_function("changeset_upsert/200", |b| {
        b.iter_batched(
            // Setup: fresh temp dir for each batch iteration.
            || TempDir::new().expect("tempdir"),
            |dir| {
                let mut store = ChangeStore::open_project(dir.path()).expect("open store");
                for i in 0..200u64 {
                    let cs =
                        AgentChangeSet::open("base", format!("jfc/b{i}"), format!("/tmp/w{i}"), i);
                    store.upsert(cs).expect("upsert");
                }
                black_box(store.len());
                dir // keep tempdir alive until drop
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_ledger_append(c: &mut Criterion) {
    c.bench_function("ledger_append/2000", |b| {
        b.iter_batched(
            || TempDir::new().expect("tempdir"),
            |dir| {
                let store = LedgerStore::open_project(dir.path()).expect("open ledger");
                for i in 0..2000u64 {
                    let ev = LedgerEvent::new(i, EventKind::ToolCall, "Bash")
                        .with_change_id(Some(format!("cs-{}", i % 10)));
                    store.append(&ev).expect("append");
                }
                black_box(0u64); // prevent dead-code elim of the loop
                dir
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_ledger_query(c: &mut Criterion) {
    // Separate setup: build a fully-populated store once, then measure query only.
    let dir = TempDir::new().expect("tempdir");
    let store = LedgerStore::open_project(dir.path()).expect("open ledger");
    for i in 0..2000u64 {
        let ev = LedgerEvent::new(i, EventKind::ToolCall, "Bash")
            .with_change_id(Some(format!("cs-{}", i % 10)));
        store.append(&ev).expect("append");
    }
    let filter = LedgerFilter {
        change_id: Some("cs-0".to_string()),
        ..Default::default()
    };

    c.bench_function("ledger_query/filter_10pct", |b| {
        b.iter(|| black_box(store.query(black_box(&filter)).expect("query").len()));
    });

    // Keep dir alive until the benchmark group finishes.
    drop(dir);
}

criterion_group!(
    benches,
    bench_changeset_upsert,
    bench_ledger_append,
    bench_ledger_query
);
criterion_main!(benches);
