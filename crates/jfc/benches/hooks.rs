//! Benchmarks for hook registry dispatch latency.
//!
//! Measures `HookRegistry::fire()` across three realistic registry shapes:
//!   - `continue_only`   — 100 Logger hooks, all return Continue (no short-circuit).
//!   - `early_abort`     — 10 Logger hooks then one Abort, short-circuits immediately.
//!   - `sparse_registry` — 10 hooks registered on a different point, exercising the
//!                         miss path (no matching hooks for the fired point).

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use jfc_engine::hooks::{HookAction, HookContext, HookHandler, HookPoint, HookRegistry};

fn make_ctx() -> HookContext {
    HookContext::for_tool("Bash", r#"{"command":"ls -la"}"#, "bench-session")
}

/// Build a registry with `n` Logger hooks on `BeforeToolDispatch`.
fn registry_n_loggers(n: usize) -> HookRegistry {
    let mut reg = HookRegistry::new();
    for _ in 0..n {
        reg.register(HookPoint::BeforeToolDispatch, HookHandler::Logger);
    }
    reg
}

/// Build a registry with `prefix` Logger hooks then one Abort hook.
fn registry_abort_after(prefix: usize) -> HookRegistry {
    let mut reg = HookRegistry::new();
    for _ in 0..prefix {
        reg.register(HookPoint::BeforeToolDispatch, HookHandler::Logger);
    }
    reg.register(
        HookPoint::BeforeToolDispatch,
        HookHandler::Custom {
            name: "abort".to_string(),
            action: HookAction::Abort("blocked by bench".to_string()),
        },
    );
    reg
}

/// Build a registry with hooks registered on a *different* point so
/// `fire(BeforeToolDispatch, …)` sees only misses.
fn registry_all_miss(n: usize) -> HookRegistry {
    let mut reg = HookRegistry::new();
    for _ in 0..n {
        reg.register(HookPoint::AfterToolDispatch, HookHandler::Logger);
    }
    reg
}

fn bench_continue_only(c: &mut Criterion) {
    let mut group = c.benchmark_group("hook_dispatch/continue_only");
    for n in [1usize, 10, 100] {
        let reg = registry_n_loggers(n);
        let ctx = make_ctx();
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                black_box(reg.fire(
                    black_box(HookPoint::BeforeToolDispatch),
                    black_box(&ctx),
                ))
            });
        });
    }
    group.finish();
}

fn bench_early_abort(c: &mut Criterion) {
    let mut group = c.benchmark_group("hook_dispatch/early_abort");
    for prefix in [0usize, 5, 10] {
        let reg = registry_abort_after(prefix);
        let ctx = make_ctx();
        group.bench_with_input(BenchmarkId::from_parameter(prefix), &prefix, |b, _| {
            b.iter(|| {
                black_box(reg.fire(
                    black_box(HookPoint::BeforeToolDispatch),
                    black_box(&ctx),
                ))
            });
        });
    }
    group.finish();
}

fn bench_miss_path(c: &mut Criterion) {
    // All hooks are on AfterToolDispatch; fire BeforeToolDispatch — pure miss scan.
    let reg = registry_all_miss(100);
    let ctx = make_ctx();
    c.bench_function("hook_dispatch/miss_100", |b| {
        b.iter(|| {
            black_box(reg.fire(
                black_box(HookPoint::BeforeToolDispatch),
                black_box(&ctx),
            ))
        });
    });
}

criterion_group!(benches, bench_continue_only, bench_early_abort, bench_miss_path);
criterion_main!(benches);
