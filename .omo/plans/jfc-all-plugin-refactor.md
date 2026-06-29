# jfc-all-plugin-refactor - Work Plan

## TL;DR (For humans)
**What you'll get:** JFC becomes a tiny kernel that hosts plugins; every real capability, including built-ins, registers through the same plugin contract. The work is sliced into timed gates so each gate leaves the app compiling and usable before the next domain moves.

**Why this approach:** Pi proves the “bare harness + extensions” model, while opencode proves the “stable plugin SDK + runtime host” model. JFC should combine both: all-plugin target, Rust-safe gated execution.

**What it will NOT do:** It will not attempt one giant unverified rewrite, expose internal state as the SDK, or use native Rust dynamic libraries as the first plugin ABI.

**Effort:** XL
**Risk:** High - this inverts the workspace dependency graph and touches providers, tools, sessions, config, and the TUI.
**Decisions I made for you:** all non-kernel capability becomes a plugin; built-ins use the same path as external plugins; execution is gate/timeboxed; external plugin v1 uses a process JSONL bridge, not native dylibs; TUI plugin APIs come after the core plugin host works.

Your next move: say `start work on .omo/plans/jfc-all-plugin-refactor.md` to execute Gate 0. Full execution detail follows below.

---

> TL;DR (machine): XL/high-risk all-plugin JFC rewrite plan, delivered through 8 timeboxed gates with dependency checks, behavior fixtures, and agent-driven CLI/TUI/API QA before each go/no-go.

## Scope
### Must have
- Target architecture: `jfc` is only a thin CLI/TUI/API host, `jfc-engine` is only the runtime kernel, `jfc-plugin-sdk` is the stable contract, `jfc-plugin-host` is the only loader/lifecycle/hook executor, and every product capability registers as a plugin.
- Add a stable SDK crate for plugin manifests, source/provenance metadata, capabilities, typed hook specs, provider/tool/resource/command/auth/session/TUI descriptors, compatibility errors, and bridge protocol DTOs.
- Add a plugin-host crate that owns discovery, provenance, activation order, lifecycle/finalizers, plugin status, error isolation, safe-mode enforcement, and the v1 process JSONL bridge adapter.
- Convert existing built-in capability families into built-in plugins through descriptors: providers, tools, agents/skills/workflows, MCP, web, memory/learn/compression, session/changeset, economy/audit/design/voice/daemon/remote.
- Invert dependencies so `jfc-engine` does not depend on product plugin crates. The kernel may depend on `jfc-core`, `jfc-plugin-sdk`, `jfc-plugin-host`, `jfc-provider` only until provider descriptors fully replace provider trait use.
- Keep all persisted config/session/task formats compatible unless a task includes explicit fixture-backed migrations.
- Provide timed gates with stop/go checks and commits after each successful gate.

### Crate disposition ledger
Every current workspace crate must be classified before Gate 1. Gate 0 fails if any workspace crate lacks a target role.

| Crate | Target role | Allowed dependencies after Gate 4 | Migration owner todo |
| --- | --- | --- | --- |
| `jfc-core` | kernel foundation | external deps only | 2 |
| `jfc-plugin-sdk` | SDK contract | `jfc-core` + serde/thiserror-style lightweight deps only | 4 |
| `jfc-plugin-host` | host service | `jfc-core`, `jfc-plugin-sdk`, `jfc-provider` only unless the evidence file justifies one bridge-neutral trait crate | 5-7 |
| `jfc-engine` | bare runtime kernel | `jfc-core`, `jfc-plugin-sdk`, `jfc-plugin-host`, `jfc-provider`, and explicitly approved persistence/bootstrap crates named in this table | 12 |
| `jfc` | CLI/TUI composition host | may depend on kernel, host, SDK, frontend support, and built-in plugin packs | 15-16 |
| `jfc-provider` | provider trait/bridge-neutral provider vocabulary | `jfc-core`; no engine/TUI/product plugin deps | 4,9 |
| `jfc-providers` | built-in provider plugin pack | SDK/host/core/provider plus provider support crates; no engine dep | 9 |
| `jfc-anthropic-sdk` | provider implementation support plugin/support crate | no engine/TUI dep | 9 |
| `jfc-auth` | auth bootstrap service and/or auth plugin pack; final classification decided in Gate 0 evidence | no engine/TUI dep unless evidence justifies composition-host use | 9,13,14 |
| `jfc-bridge` | auth/provider bridge capability plugin | SDK/host/core/provider; no engine dep | 9,14 |
| `jfc-agent` | agent primitive or built-in agent capability; final classification decided in Gate 0 evidence | if primitive: core/sdk only; if plugin: no engine dep | 10,12 |
| `jfc-agents` | built-in agent/skill/resource plugin pack | SDK/host/core/agent primitive; no engine dep | 10 |
| `jfc-mcp` | MCP/tool bridge capability plugin | SDK/host/core/provider; no engine dep | 11a,14 |
| `jfc-tools` | built-in tool implementation plugin pack | SDK/host/core; no engine dep | 8,11a |
| `jfc-config` | bootstrap/persistence service | may be approved bootstrap dependency for kernel/host; no product plugin deps | 3,13 |
| `jfc-session` | persistence/bootstrap service | may be approved bootstrap dependency for kernel/host; no product plugin deps | 3,13 |
| `jfc-changeset` | persistence/safety plugin or bootstrap service; final classification decided in Gate 0 evidence | no engine dep after Gate 4 unless approved bootstrap | 3,13 |
| `jfc-web` | knowledge/data built-in plugin | SDK/host/core/provider as needed; no engine dep | 11b |
| `jfc-memory` | knowledge/data built-in plugin | SDK/host/core/provider as needed; no engine dep | 11b |
| `jfc-learn` | knowledge/data built-in plugin | SDK/host/core; no engine dep | 11b |
| `jfc-compress` | knowledge/data built-in plugin | SDK/host/core; no engine dep | 11b |
| `jfc-graph` | knowledge/tool built-in plugin if present in workspace metadata; otherwise document absence in Gate 0 | SDK/host/core; no engine dep | 11b |
| `jfc-economy` | governance/background built-in plugin | SDK/host/core/agent primitive; no engine dep | 11c |
| `jfc-audit` | governance/background built-in plugin | SDK/host/core; no engine dep | 11c |
| `jfc-daemon` | background/runtime adapter plugin or composition-host support; final classification decided in Gate 0 evidence | no engine dep after Gate 4 unless approved bootstrap | 11c |
| `jfc-remote` | remote/API adapter plugin | SDK/host/core; no engine dep after Gate 4 unless approved transport boundary | 11c |
| `jfc-design` | UX/product built-in plugin | SDK/host/core; no kernel/TUI dep except through TUI plugin API after Gate 6 | 11d,15 |
| `jfc-voice` | UX/product built-in plugin | SDK/host/core; no engine dep | 11d |
| `jfc-markdown` | frontend/render support | frontend/TUI host only unless classified kernel-neutral utility in Gate 0 | 15 |
| `jfc-theme` | frontend/TUI support | frontend/TUI host only | 15 |

### Must NOT have (guardrails, anti-slop, scope boundaries)
- Must not do the full rewrite as one commit or skip gates after early success.
- Must not expose `EngineState`, internal `EngineEvent`, or current tool-dispatch internals as the public plugin SDK.
- Must not introduce native Rust dynamic library plugin ABI in v1.
- Must not let plugin tools bypass safe mode, permission policy, MCP/native tool policy, or sandbox decisions.
- Must not make `jfc-plugin-sdk` depend on `jfc-engine`, `jfc`, concrete providers, ratatui/crossterm, daemon, web, design, voice, or config-loader policy.
- Must not make the kernel depend on built-in plugin crates after the engine diet gate.
- Must not remove serde defaults/aliases or change on-disk session/task/config shapes without old/new fixtures.
- Must not weaken tests, delete failing tests, add `#[allow]`/`unwrap`/`panic` shortcuts, or use stringly hooks inside Rust-only APIs.

## Verification strategy
> Zero human intervention - all verification is agent-executed.
- Reference checkouts used by this plan:
  - opencode: `/home/cole/WebstormProjects/forks/opencode` (`codegraph init` already run there).
  - Pi: `/home/cole/WebstormProjects/forks/pi` (`codegraph init` already run there).
  - Any task citing opencode/Pi must use these absolute roots or restate the exact source fact in its evidence file.
- Test decision: tests-first for boundary/compatibility gates; tests-after only for mechanical dependency moves that cannot compile until the move lands. Framework: Cargo unit/integration tests, architecture dependency checks, fixture tests, and agent-driven tmux/CLI QA.
- Always run after each changed Rust gate: `cargo fmt --all --check`, `cargo check --workspace`, targeted `cargo test -p <changed-crate>`, then `cargo test --workspace` at gate boundary.
- Run `cargo clippy --workspace --all-targets` at Gate 0, Gate 3, Gate 5, and final verification.
- Run feature checks at Gate 4 and final: `cargo check -p jfc --no-default-features`, `cargo check -p jfc --no-default-features --features hooks,permission-automation,intent-gate`, and `cargo check -p jfc-design --features server-api`.
- Agent-driven CLI/TUI QA gate for CLI/TUI/API surfaces: `cargo run -p jfc -- --help`, `cargo run -p jfc -- plugin list`, `cargo run -p jfc -- doctor paths`, `cargo run -p jfc -- daemon status`, `cargo run -p jfc -- --print "ping" --output-format stream-json` when provider auth is configured, and tmux launch of `cargo run -p jfc` for TUI startup/help/model/plugin status.
- Evidence: `.omo/evidence/task-<N>-jfc-all-plugin-refactor.md` for task logs; `.omo/evidence/gate-<N>-jfc-all-plugin-refactor.md` for gate go/no-go summaries.
- “Agent-driven CLI/TUI QA” means agents execute commands and tmux scripts and record logs/screenshots. No acceptance criterion may require the user to manually click, visually inspect, or approve behavior.

## Execution strategy
### Workspace strategy
- Refactor in the existing JFC repository history. Do not create a clean-room replacement project and copy code over.
- Preferred isolation: create a Git worktree/branch from this repo, for example `GIT_MASTER=1 git worktree add -b refactor/all-plugin ../jfc-all-plugin-refactor HEAD`, then execute the plan there.
- Allowed spikes: short-lived disposable prototypes under `.omo/spikes/` or `/tmp` to prove a host/SDK seam. A spike is not the product; only reviewed, minimal code is ported back through the gated todos.
- Recovery rule: each gate lands as one or more commits, so failed gates can be reverted without losing earlier verified work.

### Parallel execution waves
> Target 5-8 todos per wave. Fewer than 3 (except the final) means you under-split.

- Gate 0, timebox 0.5-1 day: checkpoint, crate graph guardrails, baseline verification. Stop if baseline cannot be reproduced.
- Gate 1, timebox 1-1.5 days: SDK skeleton and typed contract, no runtime loading. Stop if SDK depends upward.
- Gate 2, timebox 1-2 days: plugin host skeleton and adapters over existing registries. Stop if order/lifecycle/safe-mode tests fail.
- Gate 3, timebox 2-3 days: built-in plugin registration path for providers, tools, agents/skills/workflows, MCP. Stop if existing CLI/TUI behavior changes.
- Gate 4, timebox 2-4 days: engine diet and dependency inversion. Stop if kernel still depends on product plugin crates or if session/tool/provider flows regress.
- Gate 5, timebox 2-3 days: process JSONL external plugin bridge and trust/install policy. Stop if a malformed plugin can bypass permissions or crash the host. WASM/MCP plugin adapters are post-v1 follow-up work unless a later approved plan adds them.
- Gate 6, timebox 1.5-2.5 days: TUI plugin API and UI slots, after core host is stable. Stop if render/accessibility snapshots regress.
- Gate 7, timebox 1 day: docs, examples, deprecation cleanup, final review. Stop if public docs describe unsupported plugin capabilities.

### Dependency matrix
| Todo | Depends on | Blocks | Can parallelize with |
| --- | --- | --- | --- |
| 1 | checkpoint commits `d05c187`, `3c1bb2f` | 2-16 | none |
| 2 | 1 | 3-16 | 3 |
| 3 | 1 | 4-16 | 2 |
| 4 | 2,3 | 5-16 | none |
| 5 | 4 | 6,7,8,10,11a-d | none |
| 6 | 5 | 8,10,11a-d | 7 |
| 7 | 5 | 8,10,11a-d | 6 |
| 8 | 5,6,7 | 9,10,11a-d,12 | none |
| 9 | 8 | 12-16 | none |
| 10 | 8 | 12-16 | 11a-d |
| 11a | 8 | 12-16 | 10,11b,11c,11d |
| 11b | 8 | 12-16 | 10,11a,11c,11d |
| 11c | 8 | 12-16 | 10,11a,11b,11d |
| 11d | 8 | 12-16 | 10,11a,11b,11c |
| 12 | 9,10,11a,11b,11c,11d | 13-16 | none |
| 13 | 12 | 15-16 | 14 |
| 14 | 12 | 15-16 | 13 |
| 15 | 13,14 | 16 | none |
| 16 | 15 | final verification | none |

## Todos
> Implementation + Test = ONE todo. Never separate.
<!-- APPEND TASK BATCHES BELOW THIS LINE WITH edit/apply_patch - never rewrite the headers above. -->
- [ ] 1. Gate 0: Record baseline and architecture invariants before moving code
  What to do / Must NOT do: Create `.omo/evidence/gate-0-jfc-all-plugin-refactor.md` with current commits, `cargo metadata` dependency graph, baseline command results, and the precise dependency rules below using “A may depend on B” semantics. Also complete the crate disposition ledger for every workspace member discovered by metadata. Do not edit Rust code in this task except optional test/metadata harness files that only inspect dependencies.
  Dependency rules: `jfc-core` may depend on no JFC workspace crate; `jfc-plugin-sdk` may depend on `jfc-core` only; `jfc-plugin-host` may depend on `jfc-core`, `jfc-plugin-sdk`, and explicitly approved bridge-neutral traits such as `jfc-provider`, but not `jfc-engine`, `jfc`, TUI crates, or product capability crates; `jfc-engine` may depend on `jfc-core`, `jfc-plugin-sdk`, `jfc-plugin-host`, `jfc-provider`, and the explicitly approved persistence/bootstrap crates named in the crate disposition ledger; built-in plugin crates may depend on SDK/host/core/provider as needed, but `jfc-engine` must not depend on them; `jfc` may depend on frontend/TUI crates, engine, host, SDK, and built-in plugin packs as the composition root.
  Parallelization: Gate 0 | Blocked by: checkpoint commits | Blocks: all implementation gates
  References (executor has NO interview context - be exhaustive): `Cargo.toml`; `crates/jfc-engine/Cargo.toml`; `crates/jfc/Cargo.toml`; `.omo/drafts/jfc-all-plugin-refactor.md`; `/home/cole/WebstormProjects/forks/opencode/packages/core/src/plugin.ts`; `/home/cole/WebstormProjects/forks/pi/packages/coding-agent/src/core/extensions/loader.ts`.
  Acceptance criteria (agent-executable): `cargo metadata --no-deps --format-version=1` output is captured; every workspace crate is present in the disposition ledger or explicitly marked absent/stale; `GIT_MASTER=1 git status --short --branch` shows only intended task files; evidence file names the checkpoint commits `d05c187` and `3c1bb2f`.
  QA scenarios (name the exact tool + invocation): happy: `bash cargo metadata --no-deps --format-version=1`; failure: intentionally inspect metadata for any cycle/upward dependency before continuing, Evidence `.omo/evidence/task-1-jfc-all-plugin-refactor.md`.
  Commit: Y | `docs(plugin): record all-plugin baseline`

- [ ] 2. Gate 0: Add enforceable workspace dependency-direction check
  What to do / Must NOT do: Add `crates/jfc-core/tests/workspace_dependency_rules.rs`, a Rust integration test that shells out to `cargo metadata --no-deps --format-version=1` and fails if `jfc-core` depends upward or, once the crates exist, if `jfc-plugin-sdk` depends on host/kernel/frontend/plugin implementation crates. Include a `#[ignore]` or warning-only assertion for the future Gate 4 rule that `jfc-engine` must not depend on product plugin crates, then flip it to hard-fail in Todo 12. Do not use an external service or a Python-only checker.
  Parallelization: Gate 0 | Blocked by: 1 | Blocks: 4,8,12
  References: root `Cargo.toml`; `crates/jfc-engine/Cargo.toml`; `crates/jfc-provider/Cargo.toml`; `crates/jfc-core/Cargo.toml`; `/home/cole/WebstormProjects/forks/pi/package.json` scripts `check:*`; `/home/cole/WebstormProjects/forks/opencode/package.json` `typecheck`/package split.
  Acceptance criteria: `cargo test -p jfc-core workspace_dependency_rules` passes; the exact command is documented in `.omo/evidence/gate-0-jfc-all-plugin-refactor.md`; the future Gate 4 rule is present but not yet hard-failing.
  QA scenarios: happy: `cargo test -p jfc-core workspace_dependency_rules`; failure: the test includes a fixture/assertion helper proving a forbidden edge would be rejected, Evidence `.omo/evidence/task-2-jfc-all-plugin-refactor.md`.
  Commit: Y | `test(arch): add plugin layer dependency guard`

- [ ] 3. Gate 0: Preserve persisted config/session/task compatibility fixtures
  What to do / Must NOT do: Add fixture coverage for existing config/session/task shapes before plugin migration. Include current config with plugin/safe-mode/hooks/MCP settings, minimal config, unknown fields, session JSON/JSONL, task store map and legacy array formats. Do not migrate shapes yet.
  Parallelization: Gate 0 | Blocked by: 1 | Blocks: 9
  References: `crates/jfc-config/src/lib.rs`; `crates/jfc-session/src/lib.rs`; `crates/jfc-session/src/task_store.rs`; `crates/jfc-engine/src/hooks/mod.rs`; `crates/jfc-mcp/src/lib.rs`.
  Acceptance criteria: targeted tests pass for `jfc-config` and `jfc-session`; fixtures are stored under crate test fixture directories, not runtime `.jfc/` state; no persisted schema changes are made.
  QA scenarios: happy: `cargo test -p jfc-config && cargo test -p jfc-session`; failure: fixture with legacy task array still loads or reports a precise migration error, Evidence `.omo/evidence/task-3-jfc-all-plugin-refactor.md`.
  Commit: Y | `test(persistence): lock config session task fixtures`

- [ ] 4. Gate 1: Create `jfc-plugin-sdk` as the stable all-plugin contract
  What to do / Must NOT do: Add `crates/jfc-plugin-sdk` to the workspace with modules for `manifest`, `capability`, `source`, `hook`, `descriptor`, `bridge`, `compat`, and `error`. Reuse `jfc-core` IDs/types where stable; introduce `PluginId`, `PluginVersion`, `PluginSource`, `PluginScope`, `HookName` enum, `ToolDescriptor`, `ProviderDescriptor`, `ResourceDescriptor`, `CommandDescriptor`, `AuthDescriptor`, and compatibility error DTOs. Base SDK may define UI-agnostic extension slots but must not depend on ratatui/crossterm or expose concrete TUI widget types. Do not depend on `jfc-engine`, `jfc`, or concrete plugin crates.
  Parallelization: Gate 1 | Blocked by: 2,3 | Blocks: 5-16
  References: `/home/cole/WebstormProjects/forks/opencode/packages/plugin/src/index.ts`, `/home/cole/WebstormProjects/forks/opencode/packages/core/src/plugin.ts`; `/home/cole/WebstormProjects/forks/pi/packages/coding-agent/src/core/extensions/types.ts`, `/home/cole/WebstormProjects/forks/pi/packages/ai/src/types.ts`; JFC `crates/jfc-core/src/lib.rs`, `crates/jfc-provider/src/lib.rs`.
  Acceptance criteria: `cargo test -p jfc-plugin-sdk` passes; dependency check proves SDK has only `jfc-core` plus serialization/error deps; docs in crate root define what is stable vs experimental.
  QA scenarios: happy: `cargo test -p jfc-plugin-sdk`; failure: compile test rejects a hook/descriptor with unknown string hook name in Rust API, Evidence `.omo/evidence/task-4-jfc-all-plugin-refactor.md`.
  Commit: Y | `feat(plugin-sdk): add stable plugin contract crate`

- [ ] 5. Gate 2: Create `jfc-plugin-host` registry, lifecycle, provenance, and ordered hook execution
  What to do / Must NOT do: Add `crates/jfc-plugin-host` with plugin registry, activation order, source/provenance model, enable/disable status, lifecycle finalizers, deterministic hook trigger, plugin error reporting, and status snapshot. Initially support internal in-process plugin registrations only. Do not load arbitrary external code yet.
  Parallelization: Gate 2 | Blocked by: 4 | Blocks: 6,7,8,10,11a-d
  References: `/home/cole/WebstormProjects/forks/opencode/packages/opencode/src/plugin/index.ts`, `/home/cole/WebstormProjects/forks/opencode/packages/core/src/plugin.ts`; `/home/cole/WebstormProjects/forks/pi/packages/coding-agent/src/core/extensions/runner.ts`; JFC `crates/jfc-engine/src/hooks/mod.rs`.
  Acceptance criteria: `cargo test -p jfc-plugin-host` covers ordered hook mutation, duplicate plugin ids, failed activation cleanup, finalizer exactly-once, enable/disable, and status snapshots.
  QA scenarios: happy: `cargo test -p jfc-plugin-host ordered_hook_mutation -- --exact` expects plugin A then B mutation order; failure: `cargo test -p jfc-plugin-host failed_activation_preserves_prior_plugin -- --exact` expects plugin A active and plugin B skipped with finalizer cleanup, Evidence `.omo/evidence/task-5-jfc-all-plugin-refactor.md`.
  Commit: Y | `feat(plugin-host): add lifecycle and ordered hook spine`

- [ ] 6. Gate 2: Move existing plugin discovery/provenance into the host without changing behavior
  What to do / Must NOT do: Adapt existing discovery paths from `crates/jfc/src/cli/plugin.rs`, `crates/jfc-agents/src/registry.rs`, and `crates/jfc-engine/src/workflows/registry.rs` into host-owned discovery/provenance helpers. Existing public commands and loaders may delegate to the host. Do not remove legacy functions until callers are migrated.
  Parallelization: Gate 2 | Blocked by: 5 | Blocks: 8,10,11a-d
  References: `crates/jfc/src/cli/plugin.rs`; `crates/jfc-agents/src/registry.rs`; `crates/jfc-engine/src/workflows/registry.rs`; `/home/cole/WebstormProjects/forks/pi/packages/coding-agent/README.md` package install/config docs; `/home/cole/WebstormProjects/forks/opencode/packages/opencode/src/config/plugin.ts`.
  Acceptance criteria: existing plugin list/install/remove tests still pass; new host tests cover global/project/plugin-root source info, namespace derivation, disabled plugin filtering, duplicate identity dedupe.
  QA scenarios: happy: `cargo run -p jfc -- plugin list`; failure: disabled plugin is not surfaced in skill/agent/workflow roots, Evidence `.omo/evidence/task-6-jfc-all-plugin-refactor.md`.
  Commit: Y | `refactor(plugin): centralize discovery provenance in host`

- [ ] 7. Gate 2: Convert shell hooks into typed plugin-host hooks while preserving config compatibility
  What to do / Must NOT do: Make `crates/jfc-engine/src/hooks/mod.rs` register as a built-in host plugin or host adapter. Existing `[hooks]` config and Claude-compatible shell hook JSON behavior must remain compatible. Do not remove hook config fields or aliases.
  Parallelization: Gate 2 | Blocked by: 5 | Blocks: 8,10,11a-d
  References: `crates/jfc-engine/src/hooks/mod.rs`; `crates/jfc-config/src/lib.rs`; `/home/cole/WebstormProjects/forks/opencode/packages/plugin/src/index.ts` `Hooks`; `/home/cole/WebstormProjects/forks/pi/packages/coding-agent/src/core/extensions/runner.ts` event handlers.
  Acceptance criteria: hook tests still pass; new host test proves typed hooks can wrap shell hooks; safe-mode/plugin-disable policy does not suppress configured shell hooks unless explicitly required by existing settings.
  QA scenarios: happy: `cargo test -p jfc-engine hooks`; failure: non-zero pre-tool shell hook still blocks with the same message, Evidence `.omo/evidence/task-7-jfc-all-plugin-refactor.md`.
  Commit: Y | `refactor(hooks): route shell hooks through plugin host`

- [ ] 8. Gate 3: Register built-in tools through plugin descriptors
  What to do / Must NOT do: Convert built-in tool definitions/executors into `ToolDescriptor` registrations consumed by the host, while keeping existing tool names, schemas, approval policy, progressive catalog behavior, undo tracking, and MCP namespace behavior. Do not change model-visible schemas except where tests explicitly approve.
  Parallelization: Gate 3 | Blocked by: 5,6,7 | Blocks: 9,10,11a-d,12
  References: `crates/jfc-engine/src/tools/dispatch.rs`; `crates/jfc-engine/src/tools/catalog.rs`; `crates/jfc-engine/src/tools/defs`; `crates/jfc-tools/src/lib.rs`; `crates/jfc-mcp/src/registry.rs`; `/home/cole/WebstormProjects/forks/pi/packages/coding-agent/src/core/tools/index.ts`.
  Acceptance criteria: tool schema snapshot tests pass; `cargo test -p jfc-engine tools`; plugin-host status lists built-in tool plugin; permission/safe-mode tests still gate Bash/Edit/Write.
  QA scenarios: happy: `cargo run -p jfc -- --help` and a dry tool catalog test show built-ins visible; failure: denied Bash remains denied through plugin descriptor path, Evidence `.omo/evidence/task-8-jfc-all-plugin-refactor.md`.
  Commit: Y | `refactor(tools): register built-ins as plugin descriptors`

- [ ] 9. Gate 3: Register providers through descriptors without exposing internal `Provider` as plugin ABI
  What to do / Must NOT do: Add provider descriptor registration for built-in providers in `jfc-providers` and adapt current provider selection/model list paths to consume host provider registry. Keep internal `jfc_provider::Provider` sealed until external bridge gate. Do not change provider request parity or model ids.
  Parallelization: Gate 3 | Blocked by: 8 | Blocks: 12-16
  References: `crates/jfc-provider/src/lib.rs`; `crates/jfc-providers/src/lib.rs`; `crates/jfc-engine/src/app/engine_state.rs`; `/home/cole/WebstormProjects/forks/opencode/packages/plugin/src/index.ts` provider hooks; `/home/cole/WebstormProjects/forks/pi/packages/ai/src/api-registry.ts`, `/home/cole/WebstormProjects/forks/pi/packages/ai/src/types.ts`.
  Acceptance criteria: `cargo test -p jfc-provider`, `cargo test -p jfc-providers`, and provider model selection tests pass; descriptor registry can list providers and models without requiring TUI state.
  QA scenarios: happy: `cargo test -p jfc-providers provider_descriptor_selects_builtin_model -- --exact` expects Anthropic/OpenAI-style provider selection through descriptor; failure: `cargo test -p jfc-providers provider_descriptor_bridge_failure_isolated -- --exact` expects isolated provider error and no host crash, Evidence `.omo/evidence/task-9-jfc-all-plugin-refactor.md`.
  Commit: Y | `refactor(providers): add descriptor registry for built-ins`

- [ ] 10. Gate 3: Register agents, skills, workflows, and resource paths as plugins
  What to do / Must NOT do: Convert `jfc-agents` skill/agent roots and workflow discovery to `ResourceDescriptor`/`CommandDescriptor` registration through host source info. Preserve namespacing, built-in override rules, user/project precedence, and enabledPlugins behavior. Do not change skill file format.
  Parallelization: Gate 3 | Blocked by: 8 | Blocks: 12-16
  References: `crates/jfc-agent`; `crates/jfc-agents/src/registry.rs`; `crates/jfc-engine/src/workflows/registry.rs`; `crates/jfc-agents/src/lifecycle.rs`; `/home/cole/WebstormProjects/forks/pi/packages/coding-agent/src/core/resource-loader.ts` and `/home/cole/WebstormProjects/forks/pi/packages/coding-agent/README.md` package model; `/home/cole/WebstormProjects/forks/opencode/packages/core/src/plugin/boot.ts` skill/system prompt plugin boot.
  Acceptance criteria: existing skill/agent/workflow tests pass; new tests show extension plugin can contribute skill, agent, workflow, and command descriptors with source info.
  QA scenarios: happy: `cargo test -p jfc-agents`; failure: disabled plugin namespace hides contributed skills/agents/workflows, Evidence `.omo/evidence/task-10-jfc-all-plugin-refactor.md`.
  Commit: Y | `refactor(resources): register agents skills workflows as plugins`

- [ ] 11a. Gate 3: Register MCP and tool-adjacent service capabilities as built-in plugins
  What to do / Must NOT do: Add built-in plugin descriptors for MCP and tool-adjacent service capabilities, including MCP namespaces/status and any `jfc-tools` integration points not covered by Todo 8. Do not move unrelated web/memory/economy/design code here.
  Parallelization: Gate 3 | Blocked by: 8 | Blocks: 12-16 | Can parallelize with: 10,11b,11c,11d
  References: `crates/jfc-mcp`, `crates/jfc-tools`, `crates/jfc-engine/src/tools`; `/home/cole/WebstormProjects/forks/opencode/packages/core/src/plugin/boot.ts`; `/home/cole/WebstormProjects/forks/pi/packages/coding-agent/src/core/tools/index.ts`.
  Acceptance criteria: MCP registry/status tests pass; disabling the MCP capability hides only MCP descriptors; `cargo test -p jfc-mcp` passes.
  QA scenarios: happy: `cargo test -p jfc-plugin-host builtin_mcp_capability_lists_descriptors -- --exact`; failure: `cargo test -p jfc-plugin-host disabled_mcp_capability_hides_only_mcp -- --exact`, Evidence `.omo/evidence/task-11a-jfc-all-plugin-refactor.md`.
  Commit: Y | `feat(plugins): register mcp capability plugin`

- [ ] 11b. Gate 3: Register knowledge and data capabilities as built-in plugins
  What to do / Must NOT do: Add built-in plugin descriptors for `jfc-web`, `jfc-memory`, `jfc-learn`, `jfc-compress`, and `jfc-graph` if present in workspace metadata. If `jfc-graph` is not a workspace crate on disk, record the stale reference in evidence rather than inventing a crate.
  Parallelization: Gate 3 | Blocked by: 8 | Blocks: 12-16 | Can parallelize with: 10,11a,11c,11d
  References: `crates/jfc-web`, `crates/jfc-memory`, `crates/jfc-learn`, `crates/jfc-compress`, `crates/jfc-graph`; `/home/cole/WebstormProjects/forks/pi/packages/coding-agent/src/core/resource-loader.ts`.
  Acceptance criteria: each existing capability appears in plugin status; `cargo test -p jfc-web`, `cargo test -p jfc-memory`, `cargo test -p jfc-learn`, and `cargo test -p jfc-compress` pass for crates that exist.
  QA scenarios: happy: `cargo test -p jfc-plugin-host builtin_knowledge_capabilities_list_existing_crates -- --exact`; failure: `cargo test -p jfc-plugin-host missing_optional_graph_capability_is_reported_not_panicked -- --exact`, Evidence `.omo/evidence/task-11b-jfc-all-plugin-refactor.md`.
  Commit: Y | `feat(plugins): register knowledge capability plugins`

- [ ] 11c. Gate 3: Register governance, audit, background, and remote capabilities as built-in plugins
  What to do / Must NOT do: Add built-in plugin descriptors for `jfc-economy`, `jfc-audit`, `jfc-daemon`, and `jfc-remote`. Keep daemon/remote runtime behavior unchanged; this task only exposes ownership through descriptors and status.
  Parallelization: Gate 3 | Blocked by: 8 | Blocks: 12-16 | Can parallelize with: 10,11a,11b,11d
  References: `crates/jfc-economy`, `crates/jfc-audit`, `crates/jfc-daemon`, `crates/jfc-remote`; JFC CLI daemon/remote command files; opencode server/runtime split.
  Acceptance criteria: plugin status lists each governance/background capability; `cargo test -p jfc-economy`, `cargo test -p jfc-audit`, `cargo test -p jfc-daemon`, and `cargo test -p jfc-remote` pass.
  QA scenarios: happy: `cargo test -p jfc-plugin-host builtin_governance_capabilities_list_existing_crates -- --exact`; failure: `cargo test -p jfc-plugin-host disabled_daemon_capability_does_not_break_status -- --exact`, Evidence `.omo/evidence/task-11c-jfc-all-plugin-refactor.md`.
  Commit: Y | `feat(plugins): register governance capability plugins`

- [ ] 11d. Gate 3: Register UX and product capabilities as built-in plugins
  What to do / Must NOT do: Add built-in plugin descriptors for `jfc-design`, `jfc-voice`, and any frontend-adjacent capability not migrated in TUI Gate 6. Descriptors must not pull ratatui/crossterm into `jfc-plugin-sdk` or kernel.
  Parallelization: Gate 3 | Blocked by: 8 | Blocks: 12-16 | Can parallelize with: 10,11a,11b,11c
  References: `crates/jfc-design`, `crates/jfc-voice`, `crates/jfc-markdown`, `crates/jfc-theme`; `/home/cole/WebstormProjects/forks/opencode/packages/plugin/src/tui.ts`; `/home/cole/WebstormProjects/forks/pi/packages/coding-agent/examples/extensions`.
  Acceptance criteria: capability descriptors exist for design/voice; `cargo test -p jfc-design` and `cargo test -p jfc-voice` pass; SDK/kernel dependency checks show no ratatui/crossterm leak.
  QA scenarios: happy: `cargo test -p jfc-plugin-host builtin_ux_capabilities_list_existing_crates -- --exact`; failure: dependency guard rejects `jfc-plugin-sdk -> ratatui`, Evidence `.omo/evidence/task-11d-jfc-all-plugin-refactor.md`.
  Commit: Y | `feat(plugins): register ux capability plugins`

- [ ] 12. Gate 4: Invert `jfc-engine` dependencies so kernel does not depend on plugin implementations
  What to do / Must NOT do: Move wiring from direct product-crate imports into plugin-host descriptors/adapters until `jfc-engine` no longer depends on product plugin crates. The kernel keeps only event/state/session orchestration, cancellation, permission envelope, and host callback calls. Update the dependency-direction check so all Gate 4 rules that were warnings in Gate 0 become hard failures; the task is incomplete unless a simulated `jfc-engine -> product plugin crate` edge fails the check. Do not rewrite TUI rendering here.
  Parallelization: Gate 4 | Blocked by: 9,10,11a,11b,11c,11d | Blocks: 13-16
  References: `crates/jfc-engine/Cargo.toml`; `crates/jfc-engine/src/lib.rs`; `crates/jfc-engine/src/runtime`; `crates/jfc-engine/src/app/engine_state.rs`; `cargo metadata` evidence from Gate 0.
  Acceptance criteria: dependency check enforces no kernel -> product plugin crate edges; `cargo check --workspace` and `cargo test -p jfc-engine` pass; engine public facade remains source-compatible where documented.
  QA scenarios: happy: metadata check shows kernel diet complete; failure: adding a product crate dependency to `jfc-engine` fails dependency check, Evidence `.omo/evidence/task-12-jfc-all-plugin-refactor.md`.
  Commit: Y | `refactor(engine): invert dependencies through plugin host`

- [ ] 13. Gate 4: Adapt config/session/task persistence to plugin-host source/provenance without format breakage
  What to do / Must NOT do: Route plugin config, enabled/disabled capability state, package roots, and source info through plugin-host types while preserving existing config/session/task files. Add migrations only when fixtures prove old files roundtrip. Do not delete aliases/defaults.
  Parallelization: Gate 4 | Blocked by: 12 | Blocks: 15-16 | Can parallelize with: 14
  References: `crates/jfc-config/src/lib.rs`; `crates/jfc-session/src/lib.rs`; `crates/jfc-session/src/task_store.rs`; `/home/cole/WebstormProjects/forks/opencode/packages/opencode/src/config/plugin.ts` plugin origin provenance; `/home/cole/WebstormProjects/forks/pi/packages/coding-agent/src/core/source-info.ts`, `/home/cole/WebstormProjects/forks/pi/packages/coding-agent/src/core/resource-loader.ts`.
  Acceptance criteria: fixture tests from Todo 3 pass; new provenance fields are derived or optional; `cargo test -p jfc-config && cargo test -p jfc-session` pass.
  QA scenarios: happy: old config/session/task fixtures load and save; failure: unknown plugin config field is preserved or ignored according to existing policy, Evidence `.omo/evidence/task-13-jfc-all-plugin-refactor.md`.
  Commit: Y | `refactor(config): preserve persisted shapes through plugin provenance`

- [ ] 14. Gate 5: Add external plugin bridge with trust, safe-mode, and process isolation
  What to do / Must NOT do: Implement external plugin execution over process JSONL only. Support manifest compatibility checks, handshake, capability registration, hook calls, tool execution, timeout/cancellation, stderr/error capture, and disable-on-failure policy. Do not support native dylib ABI. Do not implement WASM or MCP plugin adapters in this plan; define DTOs so they are not precluded by future plans.
  Parallelization: Gate 5 | Blocked by: 12 | Blocks: 15-16 | Can parallelize with: 13
  References: `crates/jfc-plugin-sdk`; `crates/jfc-plugin-host`; `crates/jfc/src/cli/plugin.rs`; `crates/jfc-mcp`; `/home/cole/WebstormProjects/forks/pi/packages/coding-agent/README.md` package install/trust docs; `/home/cole/WebstormProjects/forks/opencode/packages/opencode/src/plugin/loader.ts` compatibility stages.
  Acceptance criteria: host tests cover malformed manifest, incompatible SDK version, missing entrypoint, crashed plugin process, timeout, duplicate plugin id, disabled plugin, and permission-denied tool. Add `plugin_policy_matrix` tests: safe mode ON denies external install/update; safe mode ON denies external runtime activation unless already trusted and explicitly allowed by config; safe mode ON permits built-in descriptors but mutating tools still require existing permission policy; external Bash/Edit/Write-equivalent capability is denied unless existing permission/sandbox path approves it; disabled plugin cannot register descriptors, hooks, tools, or resources.
  QA scenarios: happy: `cargo test -p jfc-plugin-host process_plugin_registers_read_only_tool -- --exact`; failure: `cargo test -p jfc-plugin-host plugin_policy_matrix_denies_mutating_external_tool -- --exact`, Evidence `.omo/evidence/task-14-jfc-all-plugin-refactor.md`.
  Commit: Y | `feat(plugin-host): add external process bridge`

- [ ] 15. Gate 6: Add separate TUI plugin API and migrate UI extension points
  What to do / Must NOT do: Add a TUI-specific plugin API that can register keybindings, command palette entries, status/footer/sidebar/message renderers, model/tool/plugin status views, and overlays. Keep it separate from base `jfc-plugin-sdk` or behind a feature/module that does not pull ratatui into the core SDK. Do not destabilize the existing renderer.
  Parallelization: Gate 6 | Blocked by: 13,14 | Blocks: 16
  References: `crates/jfc/src/render`, `crates/jfc/src/input`, `crates/jfc/src/app/state.rs`; `/home/cole/WebstormProjects/forks/opencode/packages/plugin/src/tui.ts`, `/home/cole/WebstormProjects/forks/opencode/packages/tui/src/plugin/adapters.tsx`; `/home/cole/WebstormProjects/forks/pi/packages/coding-agent/examples/extensions`, `/home/cole/WebstormProjects/forks/pi/packages/coding-agent/README.md`.
  Acceptance criteria: render/accessibility snapshot tests pass; a fixture TUI plugin adds a status/footer item and a command without touching kernel; screen-reader mode still passes.
  QA scenarios: happy: tmux launch of `cargo run -p jfc` shows normal startup and plugin status; failure: tiny viewport/screen-reader/CJK render snapshots do not regress, Evidence `.omo/evidence/task-15-jfc-all-plugin-refactor.md`.
  Commit: Y | `feat(tui): add plugin UI extension surface`

- [ ] 16. Gate 7: Remove legacy special-case registries, update docs/examples, and freeze v1 plugin contract
  What to do / Must NOT do: Remove or deprecate old direct registries only after all callers use plugin-host APIs. Update README, AGENTS guidance if needed, plugin examples, migration docs, and `jfc --help`/plugin command docs. Do not claim native dylib plugin support or unsupported TUI hooks.
  Parallelization: Gate 7 | Blocked by: 15 | Blocks: final verification
  References: `README.md`; `AGENTS.md`; `crates/jfc/src/cli/plugin.rs`; `crates/jfc-engine/src/tools/registry.rs`; `crates/jfc-agents/src/registry.rs`; `.omo/drafts/jfc-all-plugin-refactor.md`.
  Acceptance criteria: docs describe kernel/plugin split accurately; legacy code paths either removed or explicitly deprecated; `cargo test --workspace` and `cargo clippy --workspace --all-targets` pass.
  QA scenarios: happy: install/list/config docs match actual CLI behavior; failure: unsupported plugin type returns clear compatibility error, Evidence `.omo/evidence/task-16-jfc-all-plugin-refactor.md`.
  Commit: Y | `docs(plugin): freeze all-plugin v1 contract`

## Final verification wave
> Runs in parallel after ALL todos. ALL verifier agents must APPROVE before the implementation may be reported as complete.
- [ ] F1. Plan compliance audit: verify every Must Have is implemented, every Must NOT is respected, every gate evidence file exists, and all commits map to one gate.
- [ ] F2. Code quality review: run `task(subagent_type="oracle", load_skills=["rust-style","refactor"], prompt=<changed files + plan + final crate graph>)`; expected output starts with `APPROVE` or `REJECT`; any `REJECT` item blocks completion until patched and re-reviewed.
- [ ] F3. Agent-driven CLI/TUI QA: tmux-drive `cargo run -p jfc -- --help`, `plugin list`, `doctor paths`, daemon status, TUI startup, and provider-auth-gated `--print` if credentials are present; record screenshots/logs under `.omo/evidence/final-agent-driven-qa-jfc-all-plugin-refactor.md`.
- [ ] F4. Scope fidelity: run `task(subagent_type="momus", load_skills=["rust-style","refactor"], prompt=<final diff + /home/cole/WebstormProjects/forks/opencode plugin facts + /home/cole/WebstormProjects/forks/pi extension facts>)`; expected output starts with `APPROVE` or `REJECT`; verify the final design is all-plugin while not copying irrelevant TypeScript/product sprawl.
- [ ] F5. Full command gate: `cargo fmt --all --check`, `cargo check --workspace`, `cargo test --workspace`, `cargo clippy --workspace --all-targets`, `cargo check -p jfc --no-default-features`, `cargo check -p jfc-design --features server-api`.

## Commit strategy
- One commit per todo unless the todo only updates evidence for a previous commit.
- Commit messages use `feat(plugin-sdk)`, `feat(plugin-host)`, `refactor(engine)`, `refactor(tools)`, `refactor(providers)`, `test(arch)`, or `docs(plugin)` scopes.
- Do not squash gate commits during execution; gate boundaries are recovery points.
- If a gate fails after three materially different attempts, revert only that gate’s own changes, keep earlier gate commits, consult Oracle, and record failure in `.omo/evidence/gate-<N>-jfc-all-plugin-refactor.md`.
- Do not push automatically.

## Success criteria
- JFC has a minimal kernel/plugin-host architecture: built-in capabilities register through plugin descriptors, and external plugins use a stable bridge contract.
- `jfc-engine` no longer depends on product plugin implementation crates after Gate 4.
- Existing CLI/TUI/headless/provider/tool/session behavior remains compatible unless an explicit, fixture-backed migration says otherwise.
- Safe mode and permission gates apply identically to built-in and external plugin tools.
- Public plugin SDK is documented, tested, and dependency-clean.
- Final verification wave passes or names pre-existing blockers with evidence.
