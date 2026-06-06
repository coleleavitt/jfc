//! Benchmark skeleton for hook dispatch latency.
//!
//! Actual benchmarks will be added when hooks are implemented (Task 12).

use criterion::{Criterion, criterion_group, criterion_main};

fn hook_dispatch_noop(c: &mut Criterion) {
    c.bench_function("hook_dispatch_noop", |b| {
        b.iter(|| {
            // Placeholder: will benchmark hook registry fire() with 100 no-op hooks
            std::hint::black_box(42)
        });
    });
}

criterion_group!(benches, hook_dispatch_noop);
criterion_main!(benches);
