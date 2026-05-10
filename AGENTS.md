- Rust workspace: 4 crates under `crates/` (jfc-anthropic-sdk, jfc-economy, jfc-graph, jfc-ui).
- Edition 2024, resolver 2.
- Run `cargo build` from workspace root. Run `cargo test` from workspace root.
- Run `cargo clippy --workspace` for linting.
- Use `thiserror` for error types, `tracing` for instrumentation, `tokio` for async runtime.
- Prefer traits over functions, enums over strings, impl blocks over derive clones.
- Zero-cost abstractions. No unnecessary allocations.
- Use snake_case for all identifiers (Rust standard).
- Keep modules focused — one responsibility per module.
- The default branch is `main`.

## Crate Overview

- `jfc-anthropic-sdk` — Anthropic Messages API client (streaming, tool use, thinking)
- `jfc-economy` — Token/cost tracking, budget enforcement, usage metering
- `jfc-graph` — Agent graph execution engine (DAG-based multi-agent orchestration)
- `jfc-ui` — Terminal UI (ratatui-based)
