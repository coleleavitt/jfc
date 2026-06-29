---
slug: jfc-all-plugin-refactor
status: planned
intent: architecture-refactor
pending-action: execute .omo/plans/jfc-all-plugin-refactor.md Gate 0
approach: all-plugin target architecture executed through timeboxed gates, not one unverified rewrite
---

# Draft: jfc-all-plugin-refactor

## Components (topology ledger)
<!-- Lock the SHAPE before depth. One row per top-level component that can succeed or fail independently. -->
<!-- id | outcome (one line) | status: active|deferred | evidence path -->
- C0 | Reversible checkpoint and dependency guardrails before code movement | active | `Cargo.toml`, `cargo metadata`, git commits `d05c187`, `3c1bb2f`
- C1 | Bare JFC kernel owns only lifecycle/state/event/cancellation/permission/plugin-host orchestration | active | `crates/jfc-engine/src/lib.rs`, `crates/jfc-engine/src/runtime/events.rs`, `crates/jfc-engine/src/app/engine_state.rs`
- C2 | Stable plugin SDK owns manifests, typed hooks, descriptors, provenance, compatibility, and bridge DTOs | active | opencode `packages/plugin/src/index.ts`, `packages/core/src/plugin.ts`; Pi `packages/coding-agent/src/core/extensions/types.ts`
- C3 | Plugin host owns discovery, activation, ordering, lifecycle/finalizers, errors, bridge adapters, source info | active | opencode `packages/opencode/src/plugin/index.ts`, `packages/opencode/src/plugin/loader.ts`; Pi `packages/coding-agent/src/core/extensions/loader.ts`, `runner.ts`
- C4 | Built-in plugin pack migrates every current JFC capability behind descriptors | active | JFC crates under `crates/`; Pi package model in `packages/coding-agent/README.md`
- C5 | Frontend/TUI/API surfaces become clients of the kernel + plugin host | active | JFC `crates/jfc`; opencode `packages/tui`, `packages/server`, `packages/sdk/js`; Pi `packages/coding-agent`, `packages/tui`
- C6 | External plugin bridge is process JSONL in v1; native Rust dylib ABI is deferred, and WASM/MCP adapters are post-v1 follow-up work | active | Oracle review; JFC sealed `jfc_provider::Provider`

## Open assumptions (announced defaults)
<!-- Intent is UNCLEAR: research resolves ambiguity, defaults are adopted (not asked), and each is surfaced in the plan's human TL;DR for veto. -->
<!-- assumption | adopted default | rationale | reversible? -->
- User wants the target to be all-plugin, not merely plugin-friendly | Adopted: everything non-kernel is a plugin, including built-ins | User explicitly asked for “big bang everything is a plugin rewrite” and Pi validates this philosophy | yes
- Execution style | Adopted: gated/timeboxed waves, not one-shot rewrite | User clarified “do it all in gates and time right not all in one shot”; Rust dependency/persistence safety requires gates | yes
- Plugin ABI | Adopted: process JSONL bridge first, no native Rust dylib ABI | Rust lacks stable plugin ABI; bridge boundary protects JFC from version skew and crashes | yes
- Provider extensibility | Adopted: descriptors/bridge for external providers while internal providers may keep sealed implementation traits | JFC provider trait is sealed today; descriptors preserve internal type safety | yes
- TUI plugin layer | Adopted: separate later TUI plugin SDK, not mixed into base SDK | Pi/opencode both separate runtime/core extension from UI extension concerns | yes

## Findings (cited - path:lines)
- opencode: plugin SDK and runtime are separate. `packages/plugin/src/index.ts` defines `PluginInput`, `Plugin`, and hook contracts; `packages/opencode/src/plugin/index.ts` owns load/init/trigger/list/dispose semantics.
- opencode: `packages/core/src/plugin.ts` has the cleaner v2 shape: typed hook specs, `define`, scoped `add/remove/trigger/triggerFor`, and ordered mutable outputs.
- opencode: `packages/opencode/src/plugin/loader.ts` separates spec normalization, target resolution, entrypoint detection, compatibility, and import.
- opencode: TUI plugins are a separate API in `packages/plugin/src/tui.ts` with route/keymap/dialog/slot/state/theme/client/lifecycle APIs.
- Pi: root README says Pi is a minimal terminal coding harness where subagents, plan mode, permission popups, MCP, todos, and background bash are intentionally not built in; users add them with extensions/skills/packages.
- Pi: `packages/coding-agent/src/core/extensions/loader.ts` extension API registers tools, commands, shortcuts, flags, renderers, providers, and runtime actions.
- Pi: `packages/coding-agent/src/core/extensions/runner.ts` binds core actions, context actions, UI context, command context, provider registration, and error handling after extension load.
- Pi: `packages/coding-agent/src/core/agent-session.ts` builds runtime from base tools plus extension results and refreshes the tool registry from active built-in and extension tools.
- JFC: `crates/jfc-engine/src/lib.rs` says the engine is frontend-neutral but currently exports many modules and acts as an aggregation point.
- JFC: cargo metadata showed `jfc-engine` depends on almost all product crates and `jfc` depends on nearly the whole workspace.
- JFC: existing plugin-related seams are fragmented across `crates/jfc/src/cli/plugin.rs`, `crates/jfc-agents/src/registry.rs`, `crates/jfc-engine/src/workflows/registry.rs`, `crates/jfc-engine/src/hooks/mod.rs`, and `crates/jfc-mcp/src/registry.rs`.
- JFC: `jfc_provider::Provider` is sealed, so provider plugins need descriptor/bridge registration rather than external direct trait impls.

## Decisions (with rationale)
- Target architecture: JFC becomes a bare kernel plus plugin host. Every capability except lifecycle/state/event/cancellation/permission/config bootstrap is a plugin.
- Built-ins use the same registration path as external plugins. “Built-in” means shipped and trusted by default, not special-cased in engine internals.
- The implementation plan will be gated by dependency direction and behavior-preservation tests. It is not a one-shot rewrite.
- The SDK is stable DTOs and descriptors, not engine internals. `EngineState` and internal `EngineEvent` remain private or host-only until deliberately stabilized.
- External plugin v1 runs via a process JSONL bridge. Native Rust dynamic libraries remain out of scope; WASM/MCP plugin adapters are post-v1 follow-up work.
- Provider extension is descriptor-based. Internal provider traits can stay sealed until a separate native-plugin ABI decision exists.
- TUI extension is a separate later gate. The first spine is server/core/runtime; TUI plugin APIs come after the host can load and activate plugins reliably.

## Scope IN
- Create the plan for a full all-plugin JFC target architecture.
- Define timeboxed gates and stop/go criteria.
- Classify current crates/domains into kernel, host service, built-in plugin, frontend/API, or deferred bridge.
- Preserve existing behavior while migrating registration paths.
- Include verification commands, evidence paths, agent-driven QA surfaces, and commit gates.

## Scope OUT (Must NOT have)
- No unverified one-shot rewrite commit.
- No native Rust dylib plugin ABI in v1.
- No `jfc-plugin-sdk` dependency on `jfc-engine`, TUI, concrete providers, or config loader policy.
- No kernel dependency on plugin implementation crates.
- No exposing `EngineState` as the public SDK.
- No persisted config/session/task format changes without fixtures and migrations.
- No bypass of safe mode, permission gates, or tool policy for plugin tools.

## Open questions
- None blocking. User clarified the target is all-plugin and execution should be gated/timeboxed.

## Approval gate
status: planned
approval-source: user clarified “do it all in gates and time right not all in one shot right” after asking for big-bang all-plugin target.
