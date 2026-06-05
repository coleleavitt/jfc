//! t444 — performance baseline with a regression gate.
//!
//! A cheap, deterministic throughput check on the change-set + ledger stores
//! (the hot persistence paths agents hit on every isolated run). The gate is
//! intentionally generous — ~20x slack over a normal local run — so it fires
//! only on a real order-of-magnitude regression (e.g. an accidental O(n^2)
//! rewrite or a per-event fsync storm), never on CI jitter. Mirrors Dolt's
//! "performance is a gated CI signal" discipline without flaky microbenchmarks.

use std::time::Instant;

use jfc_changeset::{
    AgentChangeSet, ChangeStore, EventKind, LedgerEvent, LedgerFilter, LedgerStore,
};
use tempfile::TempDir;

/// Allow an env override so a slow shared CI runner can relax the gate without
/// a code change: `JFC_PERF_SLACK=4` quadruples every budget.
fn slack() -> u128 {
    std::env::var("JFC_PERF_SLACK")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
        .max(1)
}

// Gate: 200 change-set upserts (each a full locked rewrite) complete well
// under budget. Budget 10s * slack — a normal local run is ~0.3s, so this only
// trips on a catastrophic regression.
#[test]
fn changeset_upsert_throughput_gate() {
    let dir = TempDir::new().unwrap();
    let mut store = ChangeStore::open_project(dir.path()).unwrap();

    let start = Instant::now();
    for i in 0..200u64 {
        let cs = AgentChangeSet::open("base", format!("jfc/b{i}"), format!("/tmp/w{i}"), i);
        store.upsert(cs).unwrap();
    }
    let elapsed_ms = start.elapsed().as_millis();

    assert_eq!(store.len(), 200, "all upserts landed");
    let budget = 10_000 * slack();
    assert!(
        elapsed_ms < budget,
        "changeset upsert regression: 200 upserts took {elapsed_ms}ms (budget {budget}ms)"
    );
}

// Gate: 2000 ledger appends + a full filtered query stay under budget. The
// ledger is append-only (O(1) per write) so this should be fast; a regression
// to per-append rewrite would blow the budget.
#[test]
fn ledger_append_and_query_throughput_gate() {
    let dir = TempDir::new().unwrap();
    let store = LedgerStore::open_project(dir.path()).unwrap();

    let start = Instant::now();
    for i in 0..2000u64 {
        let ev = LedgerEvent::new(i, EventKind::ToolCall, "Bash")
            .with_change_id(Some(format!("cs-{}", i % 10)));
        store.append(&ev).unwrap();
    }
    let append_ms = start.elapsed().as_millis();

    let q_start = Instant::now();
    let filter = LedgerFilter {
        change_id: Some("cs-0".to_string()),
        ..Default::default()
    };
    let hits = store.query(&filter).unwrap().len();
    let query_ms = q_start.elapsed().as_millis();

    assert_eq!(hits, 200, "10% of 2000 events match cs-0");
    let budget = 10_000 * slack();
    assert!(
        append_ms < budget,
        "ledger append regression: 2000 appends took {append_ms}ms (budget {budget}ms)"
    );
    assert!(
        query_ms < budget,
        "ledger query regression: took {query_ms}ms (budget {budget}ms)"
    );
}
