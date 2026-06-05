# Contributing to jfc

Thank you for your interest in contributing! This document covers the build process, testing, and expectations for pull requests.

## Prerequisites

- **Rust nightly** (pinned via `rust-toolchain.toml`)
- **Git** for version control

## Building

```bash
# Full workspace build
cargo build --workspace

# Public build (no sensitive OAuth features)
cargo build -p jfc-ui --bin jfc --no-default-features --features hooks,permission-automation

# Release build
cargo build --release -p jfc-ui --bin jfc
```

## Testing

```bash
# Run the full test suite
cargo test --workspace

# Test a specific crate
cargo test -p jfc-graph
cargo test -p jfc-economy
cargo test -p jfc-ui

# Test a specific module
cargo test -p jfc-ui app::tests::
cargo test -p jfc-ui daemon::tests::
```

## Code Quality

Before submitting a PR, ensure:

```bash
# Formatting (must pass)
cargo fmt --all --check

# Clippy (must pass with no warnings)
cargo clippy --workspace --all-targets -- -D warnings
```

CI runs these checks automatically on every push and pull request.

## Pull Request Process

1. **Fork** the repository and create a feature branch from `master`.
2. **Write tests** for new functionality. The workspace has 3,500+ tests — maintain that bar.
3. **Run the full check suite** (`fmt`, `clippy`, `test`) locally before pushing.
4. **Keep commits focused** — one logical change per commit, using conventional commit prefixes (`feat:`, `fix:`, `refactor:`, `test:`, `docs:`, `perf:`, `chore:`).
5. **Update the CHANGELOG** under `[Unreleased]` if your change is user-facing.

## Coding Style

- Follow idiomatic Rust patterns. The project uses edition 2024.
- Prefer traits over functions, enums over strings.
- No comments unless the *why* is non-obvious (hidden constraint, workaround, subtle invariant).
- No premature abstractions — three similar lines is better than a generic.
- Only validate at system boundaries; trust internal code and framework guarantees.
- Use `floor_char_boundary()` for any user-facing string truncation to avoid UTF-8 panics.

## Project Structure

The workspace has 20 crates under `crates/`. See the [README](README.md) architecture section for the full map. Key boundaries:

| Crate | Responsibility |
| --- | --- |
| `jfc-ui` | Main binary: TUI, event loop, tools, providers |
| `jfc-graph` | Code graph, tree-sitter adapters, DSL engine |
| `jfc-economy` | Bounty market, solvers, validators |
| `jfc-core` | Shared types used across crates |
| `jfc-provider` | Provider trait + model spec |
| `jfc-providers` | Concrete provider implementations |

## Feature Flags

`jfc-ui` has several feature gates:

- `anthropic-oauth-sensitive` — Anthropic OAuth with billing/CCH (default on, excluded from public builds)
- `hooks` — Pre/post tool hooks
- `permission-automation` — TOML-based auto-approval
- `intent-gate` — Intent classification for graph context injection
- `landlock-sandbox` — Kernel-level sandboxing (Linux only)

## License

By contributing, you agree that your contributions will be licensed under the [AGPL-3.0](LICENSE).
