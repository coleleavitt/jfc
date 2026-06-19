# Rust Profiling & Debugging Tools

## JFC quickstart

Installed locally for this workspace:

- `cargo flamegraph` / `flamegraph` 0.6.12
- Linux `perf` 6.14
- `cargo-call-stack` 0.1.16
- Rust `nightly-2023-11-13` with `rust-src` for `cargo-call-stack`
- VS Code extensions: `chanhx.crabviz`, `jebbs.plantuml`

System Graphviz / PlantUML CLIs are not installed by Cargo. Install them with
your package manager when you need command-line rendering:

```bash
sudo apt-get install graphviz plantuml
```

If `sudo` is unavailable, use the VS Code PlantUML extension for PlantUML and
Crabviz's built-in HTML/SVG export for LSP call graphs.

### CPU flamegraphs for JFC

Use the workspace profiling profile; it keeps release optimizations and full
debug symbols:

```bash
cargo flamegraph --profile profiling -p jfc --bin jfc -- \
  --help
```

For an interactive TUI session, run the binary under `flamegraph` directly so
you can pass environment toggles cleanly:

```bash
cargo build --profile profiling -p jfc
JFC_DISABLE_MCP=1 JFC_DISABLE_LSP=1 flamegraph \
  -o /tmp/jfc-tui.svg -- ./target/profiling/jfc
```

Linux may require lowering `perf_event_paranoid` before user-space sampling:

```bash
sudo sysctl kernel.perf_event_paranoid=1
sudo sysctl kernel.kptr_restrict=0
```

If `sudo` is unavailable, `cargo flamegraph` may fail even though it is installed.

### Static call graphs and stack usage

`cargo-call-stack` is installed, but it is a narrow embedded/no-std-oriented tool.
JFC is a large std/Tokio/TUI program with dynamic dispatch, so use it for focused
call-graph exploration, not authoritative stack bounds.

Use the tested nightly from the crate docs:

```bash
CARGO_PROFILE_RELEASE_LTO=fat \
  cargo +nightly-2023-11-13 call-stack --target x86_64-unknown-linux-gnu \
  --bin jfc --format dot > /tmp/jfc-call-stack.dot
```

Render the dot file when Graphviz is available:

```bash
dot -Tsvg /tmp/jfc-call-stack.dot > /tmp/jfc-call-stack.svg
```

Expected caveats for this repo:

- std formatting and panicking paths create many indirect/dynamic edges.
- Tokio and trait-object provider/tool dispatch mean stack bounds are lower-bound
  estimates, not complete safety proofs.
- Dynamic linking is unsupported by `cargo-call-stack`; prefer a narrow `START`
  symbol or small example if the full `jfc` binary is noisy.

### Crabviz and PlantUML

- Crabviz is a VS Code extension, not a Cargo CLI. Use it from VS Code's command
  palette to generate file/function call graphs from rust-analyzer.
- PlantUML is available through the VS Code extension. CLI rendering still needs
  the `plantuml` command or a PlantUML server.

### Existing JFC benchmark coverage

Current tracked performance surfaces:

- `crates/jfc/benches/hooks.rs` — hook dispatch latency.
- `crates/jfc/benches/markdown.rs` — streamed markdown sanitization/wrapping.
- `crates/jfc-changeset/benches/changeset.rs` — change-set and ledger Criterion
  benchmarks for CodSpeed.
- `crates/jfc-changeset/tests/perf_baseline.rs` — CI regression gate for
  change-set/ledger persistence.
- `.github/workflows/codspeed.yml` — CodSpeed simulation runs for `jfc` and
  `jfc-changeset` benches.
- `.github/workflows/ci.yml` — generous performance baseline gate.

Run locally:

```bash
cargo bench -p jfc
cargo bench -p jfc-changeset
JFC_PERF_SLACK=3 cargo test -p jfc-changeset --test perf_baseline
```

### Missing high-value benchmarks

Add these before relying on subjective TUI speed comparisons:

1. **TUI render frame benchmark**: build a transcript with many text, tool, and
   reasoning messages, render into `ratatui::backend::TestBackend`, and measure
   `frame::draw_synchronized` / message-window rendering.
2. **Height index / render cache benchmark**: repeated streaming appends with a
   fixed viewport; assert changed-message-only remeasure stays near O(window).
3. **SSE translator benchmark**: feed representative Anthropic event JSON into
   `jfc_providers::sse` parsing/translation, including thinking deltas,
   `signature_delta`, tool JSON deltas, keepalives, and `message_delta` usage.
4. **Request-prep benchmark**: `prepare_stream_request` with large history,
   tool catalog, CLAUDE.md/project context, memory recall miss, and MCP metadata.
5. **OAuth request hot-path benchmark**: body/header construction and account
   rotation selection with mocked accounts, excluding network.
6. **Voice VAD benchmark**: energy VAD over silence, fan noise, and voiced
   synthetic samples; separate neural VAD behind the runtime opt-in because ONNX
   construction can crash before Rust can recover.
7. **Token-rate/render spinner benchmark**: rapid `StreamEvent` bursts through
   runtime handlers plus tick sampling, validating no dropped thinking-token or
   text reveal regressions.

Suggested initial files:

```text
crates/jfc/benches/render_frame.rs
crates/jfc-providers/benches/sse.rs
crates/jfc-engine/benches/request_prep.rs
crates/jfc-voice/benches/vad.rs
```

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
