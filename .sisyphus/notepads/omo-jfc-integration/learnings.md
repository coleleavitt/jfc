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
