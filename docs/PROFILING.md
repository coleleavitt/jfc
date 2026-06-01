# Rust Profiling & Debugging Tools

## 1. Memory Profiling (Heap / Allocations)

### `dhat` crate (pure Rust, cross-platform)

**What it measures**: Heap allocations — total bytes, block counts, per-callsite allocation tracking, peak usage, lifetime analysis

**Install**: Add to `Cargo.toml`:
```toml
[dependencies]
dhat = "0.3.3"

[profile.release]
debug = 1

[features]
dhat-heap = []
```

**Setup** (add to `main()`):
```rust
#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() {
    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::new_heap();
    // ...
}
```

**Run**: `cargo run --release --features dhat-heap`

**Output**: Writes `dhat-heap.json`, viewable in DHAT's online viewer at https://nnethercote.github.io/dh_view/dh_view.html

**Shows**: Per-callsite allocation counts/bytes, peak memory, allocation lifetimes, allocation trees

**Bonus**: Supports heap usage *testing* — write tests asserting allocation counts: `dhat::assert_eq!(stats.total_blocks, 3)`

---

### Heaptrack (Linux)

**Install**: `sudo apt install heaptrack heaptrack-gui`

**Run**: `heaptrack ./target/release/mybinary`

**View**: `heaptrack_gui heaptrack.mybinary.*.zst`

**Shows**: Heap allocations over time, per-function allocation counts, leak detection, flamegraph of allocators

---

### Bytehound (Linux)

**Install**: Download from https://github.com/nickcoutsos/bytehound (build from source)

**Run**: `LD_PRELOAD=./libbytehound.so ./target/release/mybinary`

**View**: Web-based viewer at `http://localhost:8100`

**Shows**: Similar to heaptrack — timeline of allocations, per-callsite breakdown, leak detection

---

### Valgrind DHAT / Massif (Linux x86_64 only)

**Install**: `sudo apt install valgrind`

**Run**: `valgrind --tool=dhat ./target/release/mybinary` or `valgrind --tool=massif ./target/release/mybinary`

**Note**: Does NOT work on aarch64/ARM. The Rust `dhat` crate is the cross-platform alternative.

---

## 2. CPU Profiling / Flamegraphs

### `cargo flamegraph` (Linux/macOS)

**Install**: `cargo install flamegraph`

**Setup** in `Cargo.toml`:
```toml
[profile.release]
debug = true
```

**Run**: `cargo flamegraph` (generates `flamegraph.svg`)

**Options**: `cargo flamegraph --bin mybinary -- --my-args`

**What it does**: Wraps `perf record` (Linux) or `dtrace` (macOS), generates interactive SVG flamegraph

**Tip**: Use `#[inline(never)]` on functions you want to see clearly in the flamegraph

---

### samply (Linux/macOS)

**Install**: `cargo install --locked samply`

**Run**: `samply record ./target/release/mybinary`

**View**: Opens Firefox Profiler automatically in browser

**Shows**: CPU time per function, call tree, flamegraph, timeline per thread

**Advantage over cargo-flamegraph**: Modern UI (Firefox Profiler), per-thread views, interactive

---

### `perf` (Linux)

**Setup**:
```bash
echo -1 > /proc/sys/kernel/perf_event_paranoid
RUSTFLAGS='-C force-frame-pointers=y' cargo build --release
```

**Run**:
```bash
perf record -F 997 -g ./target/release/mybinary
perf report -g graph,0.5,caller  # interactive TUI
perf script > profile.txt        # export for Firefox Profiler
```

**Advanced**: Profile specific events:
```bash
perf stat -e cache-misses,branch-misses ./target/release/mybinary
perf record -g -e cache-misses -c 100 ./target/release/mybinary
```

---

### DTrace + inferno (macOS)

**Install**: `cargo install inferno`

**Run**:
```bash
sudo dtrace -x ustackframes=100 -n "profile-97 /pid == $PID/ { @[ustack()] = count(); } tick-60s { exit(0); }" -o out.stacks
cat out.stacks | inferno-collapse-dtrace > stacks.folded
cat stacks.folded | inferno-flamegraph > flamegraph.svg
```

---

### pprof-rs (programmatic, in-process)

**Install**: Add `pprof = { version = "0.14", features = ["flamegraph"] }` to Cargo.toml

**Use**: Call pprof APIs programmatically to generate flamegraphs from within the running process

**Good for**: Embedding profiling in tests/benchmarks, CI integration

---

## 3. Tokio Async Runtime Debugging

### tokio-console (live TUI debugger for async tasks)

**Install CLI**: `cargo install tokio-console`

**Add to app** — `Cargo.toml`:
```toml
[dependencies]
console-subscriber = "0.4"
tokio = { version = "1", features = ["full", "tracing"] }
```

**Setup** (replace your tracing init):
```rust
console_subscriber::init(); // replaces tracing_subscriber::fmt::init()
```

**Run app**: `RUSTFLAGS="--cfg tokio_unstable" cargo run`

**Connect**: `tokio-console` (in another terminal)

**Shows**: Live task list, poll durations, waker counts, self-wakes, task states, resource contention

**Key spans emitted by Tokio**:
- `runtime.spawn` (green) — tasks
- `runtime.resource` (red) — resources (mutexes, channels)
- `runtime.resource.async_op` (blue) — async operations
- `tokio::task::waker` events — wake/clone/drop waker operations

---

### Tokio tracing (raw)

- Enable `RUST_LOG=tokio=trace` to see all internal tokio spans/events
- Tokio emits structured tracing events for: task spawn/enter/exit/close, waker operations, resource state changes

---

### Tokio Runtime Metrics (programmatic)

- Enable with `tokio_unstable` cfg flag
- Access via `tokio::runtime::Handle::current().metrics()`
- Provides: worker thread count, active tasks, poll count, poll duration, injection queue depth
- Good for: custom dashboards, detecting blocking-in-async

---

## 4. Benchmarking

### Divan (newer, simpler)

**Install**: Add `divan = "0.1"` to dev-dependencies

**Setup** (`benches/mybench.rs`):
```rust
fn main() { divan::main(); }

#[divan::bench]
fn my_function(bencher: divan::Bencher) {
    bencher.bench(|| { /* code */ });
}
```

**Cargo.toml**:
```toml
[[bench]]
name = "mybench"
harness = false
```

**Run**: `cargo bench --bench mybench`

**Output**: fastest/slowest/median/mean/samples/iters per benchmark

---

### Criterion (established standard)

**Install**: Add `criterion = { version = "0.5", features = ["html_reports"] }`

**Run**: `cargo bench` — generates HTML reports with statistical analysis

**Async support**: Use `criterion::async_executor::AsyncStdExecutor` or tokio runtime in bench functions

---

### Bencher.dev (continuous tracking)

- Free for open-source; tracks benchmark results over time
- `bencher run --adapter rust_criterion "cargo bench"`

---

## 5. Quick Reference: Which Tool for What

| Goal | Tool | Platform |
|------|------|----------|
| "Which functions allocate most?" | `dhat` crate | All |
| "Where are my memory leaks?" | Heaptrack / Bytehound | Linux |
| "Which functions are slowest?" | `cargo flamegraph` / samply | Linux/macOS |
| "Cache misses per function?" | `perf stat -e cache-misses` | Linux |
| "What are my async tasks doing?" | `tokio-console` | All |
| "Is something blocking the runtime?" | `tokio-console` + waker analysis | All |
| "How fast is this function?" | Divan / Criterion | All |
| "Did my refactor regress perf?" | Criterion + bencher.dev | All |
