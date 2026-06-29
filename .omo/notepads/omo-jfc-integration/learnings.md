## Feature flag scaffolding

- Added jfc-ui feature flags with `default = []`: `hashline`, `permission-automation`, `hooks`, `background-agents`, `intent-gate`, and `landlock-sandbox`.
- Added cfg-gated module declarations in `crates/jfc-ui/src/main.rs`; stubs are doc-comment only.
- `sandbox/mod.rs` gates `landlock` with `#[cfg(target_os = "linux")]`.

## jfc-ui test infrastructure

- Added `criterion` with `html_reports` and `proptest` to `crates/jfc-ui` dev-dependencies.
- Added the `hooks` benchmark target with `harness = false` after the feature flags.
- Created `crates/jfc-ui/benches/hooks.rs` as a placeholder criterion benchmark for future hook dispatch latency work.
- Verified with `cargo bench -p jfc-ui --all-features --no-run && cargo test -p jfc-ui --all-features`.

## Feature config loader

- `crates/jfc-ui/src/config.rs` now has a cfg-gated `feature_config` module enabled by `permission-automation`, `hooks`, `intent-gate`, or `background-agents`.
- `FeatureConfig::load(base_dir)` reads `.jfc/features.toml`, returns defaults for missing files, and warns then defaults for malformed TOML.
- Defaults include permission ceilings for destructive shell commands and background agent limits of `max_concurrent = 5`, `max_depth = 2`.

## Hashline content anchoring

- `crates/jfc-ui/src/hashline.rs` implements `LineId` as the first 4 bytes / 8 hex chars of SHA-256 over trimmed line content.
- `FileIndex::build` maps hashes to all matching 0-based line numbers; `resolve` prefers an exact hint and otherwise chooses the nearest duplicate.
- `HashlineCache::get_or_build` keys by path and rebuilds when filesystem `mtime` changes.

## Hashline edit resolution bridge

- `try_resolve_edit_target(content, old_string, hint_line)` builds a temporary `FileIndex` and returns an `EditResolution` line range for future Edit tool wiring.
- Resolution path: unique exact substring matches return immediately, duplicate matches use the first old-string line hash plus the hint, and missing matches fall back to fuzzy matching at confidence >= 0.9.

## Permission dispatch integration

- `permissions::check_tool_permission` is a thin public wrapper around `RuleSet::evaluate`, intended for per-invocation hot-reload checks before tool execution.
- `tools::execute_tool` now performs a cfg-gated permission check under `permission-automation`, loading `.jfc/features.toml` from the current working directory each invocation and returning a failed `ExecutionResult` for denied tools before dispatch.

## Phase 2 hook dispatch marker

- `HookHandler::IntentEnricher` remains non-mutating and returns `HookAction::Continue`; under `intent-gate` it now logs that enrichment was requested.
- `tools::execute_tool` has the cfg-gated `hooks` marker for `BeforeToolDispatch` before permission checks and actual tool execution, but still does not execute hooks in the hot path.

## Background agent state manager

- `crates/jfc-ui/src/background.rs` keeps background agents as a synchronous, testable state manager: callers own actual tokio task spawning while the manager tracks IDs, status, capacity, and collection.
- Completed or failed agents stop counting toward `max_concurrent`; collection takes the stored result once without removing the lifecycle entry.

## Phase 3 TUI summaries and sandbox marker

- `BackgroundManager::summaries()` returns stable ID-sorted `AgentSummary` values with cloned status, task description, and elapsed milliseconds so render code can display background agents without touching manager internals.
- Economy bounty mechanistic verification now defines and logs an `SandboxPolicy::economy_solver(worktree)` under `landlock-sandbox`, applying it to the verification command as a marker until real Landlock enforcement lands.

## Phase 4 orchestration primitives

- `.jfc/agents/argus.toml` is an example read-only review agent profile: Read/Grep/Glob/Lsp allowed, Edit/Write/Bash denied, P0-P3 structured review methodology.
- `background.rs` now exposes additive orchestration primitives only: Ralph-style continuation checks, tmux command/result data types without execution, and markdown handoff summaries.
- `HookHandler::CommentChecker` is advisory-only: it warns on known AI-slop comment patterns in tool input and always returns `HookAction::Continue`.

## E2E orchestration coverage

- `hooks.rs` contains `test_e2e_orchestration_pipeline`, cfg-gated on hooks, hashline, permission automation, intent gate, and background agents.
- The test exercises the orchestration path end-to-end: implementation intent classification, default permission Ask decision, logger/comment hooks, hashline edit target resolution, and background agent collection.
