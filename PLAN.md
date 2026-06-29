# Migrate jfc-knowledge from rusqlite to sqlx (fully async)

## TL;DR

Replace the synchronous `rusqlite` backend of `jfc-knowledge` with async
`sqlx` (`SqlitePool`), make every `KnowledgeStore` method `async`, and cascade
`async` outward through all 238 call sites in the 7 dependent crates
(`jfc-session`, `jfc-engine`, `jfc-learn`, `jfc-memory`, `jfc-agents`,
`jfc-config`, `jfc-daemon`). Use **runtime `sqlx::query`/`query_as` only — no
compile-time macros** (avoids the `DATABASE_URL`/FTS5 macro-verification
friction), port the existing hand-rolled ordered migrator to async (preserve
versioning logic exactly), and keep the on-disk SQLite file format byte-compatible
(same engine, WAL, FTS5). The `sqlx` source of truth is the local checkout at
`~/RustProjects/forks/sqlx` (v0.9.0).

## Context

- `jfc-knowledge` holds an owned `rusqlite::Connection` behind `KnowledgeStore`.
  API surface in use: 64 `.execute`, 33 `.prepare`, 27 `.query_map`,
  14 `.query_row`, 5 `.transaction`, 3 `.pragma_update`, 3 `.execute_batch`,
  1 `.last_insert_rowid`, 1 `.busy_timeout`.
- Schema = 10 ordered DDL migrations gated on a `schema_version` table, applied
  in a transaction (`schema.rs`). Includes **FTS5 virtual tables + triggers**
  (`knowledge_fts`, `session_messages_fts`, `definitions_fts`).
- 238 call sites across 7 crates. Heaviest: `jfc-engine` (130),
  `jfc-session` (41), `jfc-memory` (18), `jfc-config` (15), `jfc-daemon` (14),
  `jfc-agents` (11), `jfc-learn` (9).
- Genuinely-sync call contexts that need explicit bridging:
  `jfc-session/src/task_store.rs` (`persist_unlocked`, `load_inner_from_db`,
  `reload_if_changed`), `task_history.rs`, `inbox.rs`, `search.rs`, `catalog.rs`.
  These are reached from sync `&self` methods and the `TaskStore` mutex path.
- Binary entrypoint (`crates/jfc/src/main.rs`) is already `#[tokio::main]`, so a
  tokio runtime is always present at the top.

## Work Objectives

- `jfc-knowledge` depends on `sqlx` (sqlite, runtime-tokio, no macros), not `rusqlite`.
- `KnowledgeStore` wraps a `SqlitePool`; every public method is `async`.
- Schema migrator ported to async, same version semantics, FTS5 + triggers intact.
- All 7 dependent crates updated so their call sites `.await` the new API.
- Genuinely-sync callers bridged via one documented helper
  (`block_on_knowledge`) using `tokio::task::block_in_place` +
  `Handle::current().block_on`, or refactored to async where the caller is async.
- `cargo build` and `cargo test` pass workspace-wide; `rg rusqlite` returns
  nothing under `crates/`.

## Verification Strategy

- Per-phase: `cargo build -p <crate>` after each crate is converted.
- `cargo test -p jfc-knowledge` after Phase 2 (store + schema parity).
- `cargo test -p jfc-session` after the task-store bridge (covers the earlier
  resurrection regression tests too).
- Workspace `cargo build` then `cargo test` in the Final Verification Wave.
- `cargo clippy --workspace` since this touches shared runtime abstractions.
- Grep gate: `rg -l rusqlite crates/` must be empty.

## Execution Strategy

Bottom-up, compiling at each boundary so breakage is localized:
1. Foundation (deps, error, pool, schema) — keep the crate compiling in isolation.
2. Convert the query layer + `KnowledgeStore` methods to async.
3. Cascade async through dependents, leaf crates first (`jfc-learn`,
   `jfc-memory`, `jfc-config`, `jfc-agents`, `jfc-daemon`), then `jfc-session`,
   then `jfc-engine` (largest).
4. Remove rusqlite, full verification wave.

Bridging rule: prefer making the caller `async` and `.await`ing. Only use the
`block_on_knowledge` sync bridge where the call site is structurally sync (a
`Drop`, a `Mutex`-guarded `persist_unlocked`, or a non-async trait method) and
cannot be made async without a second viral cascade.

## TODOs

- [ ] 1. Add `sqlx` (features: `runtime-tokio`, `sqlite`, `chrono`) and remove
  `rusqlite` in `crates/jfc-knowledge/Cargo.toml`, pointing at workspace dep;
  add `sqlx` to `[workspace.dependencies]` referencing the local fork path.
- [ ] 2. Rewrite `error.rs`: replace `rusqlite::Error` `#[from]` with
  `sqlx::Error`; keep `Migration`/`Io`/`InvalidRecord` variants.
- [ ] 3. Port `schema.rs` to async: `apply_pragmas`/`migrate` take `&SqlitePool`
  (or `&mut SqliteConnection`), run the same ordered DDL + `schema_version`
  gating in a transaction; preserve FTS5 tables/triggers verbatim. Configure
  pragmas (WAL, synchronous=NORMAL, foreign_keys=ON, busy_timeout) via
  `SqliteConnectOptions`.
- [ ] 4. Convert `KnowledgeStore` in `lib.rs` to hold `SqlitePool`; make
  `open`/`open_default`/`open_in_memory` async constructors; convert every
  method body from `self.conn` rusqlite calls to `sqlx::query*().await`.
- [ ] 5. Convert the remaining query modules (`query.rs`, `memory.rs`,
  `definitions.rs`, `record.rs`, `import.rs`, `session_mine.rs`, `project.rs`,
  `redact.rs`) and `agent_events/*` to async sqlx; map `query_map`→`fetch_all`
  + `try_get`, `query_row`→`fetch_one/optional`, `last_insert_rowid`→
  `last_insert_rowid()` on the `SqliteQueryResult`.
- [ ] 6. Update `jfc-knowledge` internal tests to `#[tokio::test]` and `.await`;
  `cargo test -p jfc-knowledge` green.
- [ ] 7. Add the `block_on_knowledge` sync bridge helper (in `jfc-session`, or a
  shared spot) using `block_in_place` + `Handle::current().block_on`; document
  the invariant that it must run inside the tokio runtime.
- [ ] 8. Cascade async through `jfc-learn` (9 sites): make callers async/await or
  bridge; `cargo build -p jfc-learn`.
- [ ] 9. Cascade through `jfc-memory` (18 sites); `cargo build -p jfc-memory`.
- [ ] 10. Cascade through `jfc-config` (15 sites); `cargo build -p jfc-config`.
- [ ] 11. Cascade through `jfc-agents` (11 sites); `cargo build -p jfc-agents`.
- [ ] 12. Cascade through `jfc-daemon` (14 sites); `cargo build -p jfc-daemon`.
- [ ] 13. Cascade through `jfc-session` (41 sites): bridge the `TaskStore`
  persist/reload/load paths and `task_history`/`inbox`/`search`/`catalog`;
  `cargo build -p jfc-session` and `cargo test -p jfc-session`.
- [ ] 14. Cascade through `jfc-engine` (130 sites): convert async call paths to
  `.await`; bridge structurally-sync ones; `cargo build -p jfc-engine`.
- [ ] 15. Remove the `rusqlite` dep entirely; `rg -l rusqlite crates/` empty.

## Final Verification Wave

- [ ] F1. `cargo build` (workspace) passes.
- [ ] F2. `cargo test` (workspace) passes, including the task-store resurrection
  regression tests and the knowledge schema parity tests.
- [ ] F3. `cargo clippy --workspace` clean (no new warnings on touched crates).
- [ ] F4. `rg -l rusqlite crates/` returns nothing; `~/.local/share/jfc/knowledge.db`
  opens and migrates cleanly via the sqlx path (smoke: open_default + a recall).

## Success Criteria

- `jfc-knowledge` uses `sqlx` exclusively; no `rusqlite` anywhere under `crates/`.
- Every `KnowledgeStore` method is async; the dependent crates await them, with
  only the documented `block_on_knowledge` bridge at structurally-sync sites.
- Schema/version semantics and FTS5 search behavior are preserved (existing DB
  files keep working).
- `cargo build` + `cargo test` + `cargo clippy --workspace` all green.

---

# Follow-on roadmap: bring Magic Context's hippocampus into JFC

## TL;DR

JFC already has the raw ingredients: `jfc-session`, `jfc-memory`,
`jfc-knowledge`, `jfc-learn`, `jfc-daemon`, `jfc-graph`, prompt-cache
diagnostics, task history, and a rich TUI. What is still missing, compared to
Magic Context, is the **cache-stable operating model** around those pieces:
deterministic m[0]/m[1]-style context layout, historian compartments with decay
rendering, validated hidden-agent outputs, explicit context-reduction tools,
semantic recall health, and user-visible diagnostics for every background
mutation.

This follow-on roadmap keeps the SQLx migration above as the storage foundation.
After that migration lands, implement Magic Context parity inside JFC as a native
Rust subsystem rather than vendoring the TypeScript plugin.

## Source anchors

- Magic Context source checkout: `~/WebstormProjects/forks/magic-context/`.
- Research copies already in this repo: `.research/magic-context/ARCHITECTURE.md`
  and `.research/magic-context/STRUCTURE.md`.
- JFC destination crates: `jfc-session`, `jfc-memory`, `jfc-knowledge`,
  `jfc-learn`, `jfc-engine`, `jfc-daemon`, `jfc`, `jfc-plugin-host`,
  `jfc-plugin-sdk`.
- JFC recent log-derived bugs that prove the need for this roadmap: repeated
  legacy memory import churn, repeated Anthropic thinking-signature recovery,
  and expected stream cancellations logged as hard errors.

## Non-goals

- Do **not** port Magic Context line-for-line.
- Do **not** put another all-powerful context manager beside JFC's compaction
  path; replace the path with a single JFC-owned context subsystem.
- Do **not** expose raw `EngineState` to plugins or memory workers.
- Do **not** make hidden LLM tasks mutate session state without validated output
  contracts and durable diagnostics.

## Architecture reset: Pi/opencode primitives first

The target is not "JFC plus Magic Context." The target is a smaller runtime with
Pi/opencode-style primitives, then Magic Context capability implemented as a
first-party pack on those primitives.

### opencode primitives to copy

- **Effect/service graph first.** opencode wraps infrastructure as `Context.Service`
  / `Layer` services and boots default layers. JFC should move runtime services
  behind traits + typed service registries before adding more product behavior.
- **Plugin contract before feature internals.** opencode's plugin V2 shape is a
  small `{ id, effect(context) }` unit; hooks are typed registrations over a
  known spec. JFC should make first-party behavior register through the same SDK
  as external behavior.
- **Schema-backed tool registry.** Tools should be descriptors + handlers, not
  permanent `ToolKind`/`ToolInput` enum growth.
- **Auth/provider/tool mutations as hooks.** Provider auth, request mutation,
  tool definition transformation, permission, and message transforms are hooks
  over typed events, not ad-hoc calls into a central engine object.

Source evidence to keep nearby while implementing: `packages/opencode/src/effect/run-service.ts`,
`packages/opencode/src/effect/config-service.ts`, `packages/opencode/src/provider/provider.ts`,
`packages/opencode/src/tool/registry.ts`, `packages/opencode/src/plugin/index.ts`,
`packages/opencode/src/plugin/loader.ts`, `packages/plugin/src/v2/effect/plugin.ts`, and
`packages/core/src/effect/layer-node.ts` in `~/WebstormProjects/forks/opencode`.

### Pi primitives to copy

- **Runtime factory per cwd/session.** Pi's `CreateAgentSessionRuntimeFactory`
  recreates cwd-bound services, resolves session options, and returns a runtime
  wrapper with diagnostics. JFC needs the same boundary: a runtime is not a global
  `EngineState`; it is a session plus services plus diagnostics.
- **Extension runner with maps.** Pi extensions register tools, commands,
  shortcuts, flags, message renderers, and providers into extension-owned maps;
  `bindCore()` later attaches curated runtime actions. JFC should mirror that
  shape in `jfc-plugin-host` rather than exposing app internals.
- **Append-only session entries.** Pi's JSONL session storage is a tree of typed
  entries (`message`, model change, thinking-level change, compact boundary,
  branch summary, custom entries, labels). JFC session persistence should move
  toward typed append entries instead of one mutable transcript blob plus side
  stores.
- **Curated runtime actions.** Extensions can request `sendMessage`,
  `appendEntry`, `setSessionName`, active-tool changes, provider registration,
  model/thinking changes, and event-bus access. JFC should expose these as safe
  DTOs, never raw state.

Source evidence to keep nearby while implementing: `packages/coding-agent/src/core/agent-session-runtime.ts`,
`packages/coding-agent/src/core/agent-session-services.ts`, `packages/coding-agent/src/core/agent-session.ts`,
`packages/coding-agent/src/core/extensions/{loader,runner,types}.ts`,
`packages/coding-agent/src/core/session-manager.ts`, `packages/agent/src/harness/session/jsonl-storage.ts`,
and `packages/coding-agent/src/core/tools/index.ts` in `~/WebstormProjects/forks/pi`.

### Rust architecture constraints

- Domain modules over flat file piles: parent `mod.rs`/`lib.rs` curates public
  surface, child modules own roles (`runtime/session`, `runtime/services`,
  `context/layout`, `context/reduce`, `context/health`).
- Split crates only when dependency or reuse boundaries require it; otherwise
  split modules first. This follows Rust compiler/Zulip guidance: fewer crates
  when a module boundary is enough, typed IDs at boundaries, explicit phases,
  and executable validators over prose-only rules.
- Every state transition should have a typed phase/dialect name. Avoid booleans
  like "compacting" where a type can distinguish `LiveTail`, `StableBaseline`,
  `Materializing`, `Deferred`, `Healing`, and `Archived`.
- Folder names should read like Pi/opencode packages (`core`, `agent`,
  `plugin`, `tui`, `providers`, `packs`) rather than every crate repeating
  `jfc-*`. The package name can remain publishable Rust (`jfc-kernel`) while the
  repo path should communicate ownership (`crates/kernel`).

## Current JFC structure audit

JFC is capability-rich but structurally sloppy in the exact way Pi/opencode avoid:

- **`jfc-engine` is a mega-composition point.** It owns stream orchestration,
  tools, runtime operations, compaction, memory hooks, prompt construction,
  plugin descriptors, workflows, goals, shell safety, provider policy, and UI
  side effects. This makes every new feature look like another module under
  `crates/jfc-engine/src/` instead of a registered service.
- **`jfc` still owns too much app/runtime coupling.** The TUI reaches into
  `App`/`EngineState` for rendering, status rows, input handlers, plugin widgets,
  and runtime actions. Pi keeps UI affordances behind extension/runtime contexts.
- **Tool dispatch is still partly closed-world.** Descriptor routes exist, but
  `ToolKind`/`ToolInput` and `tools/dispatch.rs` remain structural bottlenecks.
- **Session state is not entry-log shaped.** JFC has sessions, task stores,
  background results, compaction archives, and goal sidecars, but not one typed
  append-entry substrate like Pi's `SessionStorage`.
- **Context/memory work is split by accident, not by domain.** `jfc-memory`,
  `jfc-knowledge`, `jfc-learn`, `jfc-session`, compaction archives, and runtime
  prompt construction each own part of the hippocampus story. The target is one
  `context` domain with submodules for layout, memory, history, reduce, search,
  health, and maintenance.
- **The `jfc-*` crate sprawl is itself architectural debt.** Pi/opencode do not
  make every subsystem a top-level package with the product prefix. They have a
  small set of package roots and meaningful internal module trees. JFC should
  collapse/consolidate crates unless a public API, dependency boundary, or
  compile-time isolation justifies the split.

## Bare-kernel target

JFC should be brought back to a small kernel. The kernel owns only:

- an append-only session log and active leaf pointer;
- an event bus and turn lifecycle state machine;
- service handles for providers, tools, sessions, context, policy, plugins, and UI;
- permission/safety gates that every extension path must pass through;
- effect emission to frontends (`FrontendDirective`, status snapshots, toasts,
  render invalidations), not frontend implementation;
- cancellation, scoped stream IDs, and in-flight task/stream coordination.

Everything else becomes a first-party plugin pack, domain crate, or service:
tools, slash commands, GitHub, research, council, economy, memory/dreamer,
voice, web, design, daemon jobs, workflow optimizer, LSP, provider bridges,
status widgets, runtime actions, and UI panels.

## Target repository shape

Use this as the destination map. Names are intentionally short and ownership
oriented; avoid a new graveyard of `jfc-*` folders.

```text
crates/
  kernel/              # bare runnable kernel: event bus, lifecycle, RuntimeServices traits
  protocol/            # stable DTOs: messages, tools, sessions, provider content, IDs
  runtime/             # AgentRuntime factory, service graph, typed effects, diagnostics
  session/             # append-entry log, projections, search/catalog/task/inbox
  plugin/              # SDK + host + extension runner + descriptor registry
  tui/                 # ratatui shell only: paint view models, translate terminal input
  cli/                 # argument parsing + command output adapters only
  providers/           # built-in provider pack implementations
  tools/               # built-in tool pack implementations
  context/             # cache-stable layout, memory/history/reduce/search/health
  policy/              # permissions, shell safety, safe mode, provenance, trust gates
  orchestration/       # agents, tasks, swarm, council, workflows, advisor, goals
  daemon/              # cron, wakeups, workers, detached processes
  ui-model/            # status rows, sidebar rows, transcript/tool view models
```

Likely collapses/merges from current crates:

- `jfc-core` → `protocol`.
- `jfc-engine` → split into `kernel`, `runtime`, `context`, `policy`,
  `orchestration`, `tools`, `providers`, `session`; then delete the original
  catch-all crate.
- `jfc` → split into `tui` and `cli`, with `tui` depending on `ui-model` and
  `runtime`, not on every product domain.
- `jfc-plugin-sdk` + `jfc-plugin-host` → `plugin` with public SDK and host
  submodules.
- `jfc-memory` + relevant `jfc-knowledge` API + relevant `jfc-learn` hot-path
  contributors → `context::{memory,recall,health}` plus `orchestration`/`daemon`
  jobs for maintenance.
- `jfc-provider` + `jfc-providers` → `providers::{protocol,builtin,registry}`.
- `jfc-tools` + engine tool implementations → `tools::{registry,builtin,...}`.

Do not physically rename everything in one commit. Create the destination
modules first, move one service boundary at a time, and leave compatibility
re-exports only temporarily with deletion tasks attached.

Named policy: **Engine Root Module Freeze**. Existing root-level
`crates/jfc-engine/src/*.rs` files are grandfathered as teardown debt, but new
product-domain root files are blocked by the architecture guard unless an
explicit allowlist exception is recorded with evidence. New behavior should go
under a domain module or destination crate first.

## Destination ownership map

| Current JFC area | Teardown problem | Target owner |
| --- | --- | --- |
| `jfc-engine::tools/**`, `stream/tool_dispatch.rs` | Concrete tools and schema dispatch are embedded in the engine. | `jfc-tools` + `jfc-plugin-host` descriptor registry + a tiny kernel `ToolRuntime` trait. |
| `jfc-engine::commands/**`, `command_spec.rs` | Slash/product commands pull nearly every domain into engine. | New `jfc-command` crate; domain commands delegate to session/daemon/memory/etc. |
| `jfc-engine::stream/request/**` | Prompt construction mixes memory, docs, plugin context, runtime state, provider budget. | New `jfc-context` with `ContextAssembler`, contributors, layout, health, and cache policy. |
| `jfc-engine::stream/messages/**` | Provider wire lowering belongs to provider/request adapter layer. | `jfc-provider` generic message shaping + provider-specific lowering in `jfc-providers`. |
| `jfc-engine::session/**`, naming/recap/serialization helpers | Session persistence is not runtime kernel. | `jfc-session`; move serialization/deserialization/repair and typed entry-log here. |
| `jfc-engine::compact/**`, `compact_archive.rs`, `context_accounting/**` | Compaction and archive policy are context/session concerns. | `jfc-context` + `jfc-compress` + archive rows in `jfc-knowledge`/`jfc-session`. |
| `jfc-engine::agents/**`, `swarm/**`, council/workflow/advisor modules | Agent orchestration is product behavior, not kernel. | New `jfc-orchestration` plus `jfc-agent`/`jfc-agents`; engine only emits/receives orchestration events. |
| `jfc-engine::daemon/**`, `daemon_services.rs`, `dreamer_scheduler.rs` | Daemon and dreamer lifecycle belong to daemon/learn services. | `jfc-daemon` and `jfc-learn` with a kernel service trait. |
| `jfc-engine::github/**`, `research.rs`, `web_search.rs`, `ccr.rs`, bridge/remote helpers | External integrations inflate the engine. | `jfc-github`/`jfc-web`/`jfc-bridge`/`jfc-remote`, surfaced as plugins/tools. |
| `jfc-engine::app::EngineState` | Shared mutable bag couples every feature. | `AgentRuntime { session, services, diagnostics }`; keep `EngineState` only as transitional compatibility. |
| `jfc-engine::permissions`, `shell_safety`, `auto_classifier`, `auto_mode` | Safety policy and classifier prompts are not engine internals. | New `jfc-policy` or `jfc-tools::safety`; kernel asks policy service. |
| `crates/jfc/src/app/plugin_*`, `input/runtime_action_*`, plugin CLI/smoke | The terminal crate owns plugin runtime state and actions. | `jfc-plugin-host::ui_runtime` / new `jfc-plugin-ui`; TUI receives view-model DTOs. |
| `crates/jfc/src/render/status*`, widgets/panels/sidebar | Rendering code computes domain state directly from `App`/`EngineState`. | `jfc-ui-model` or plugin UI slot view models; Ratatui only paints rows. |
| `crates/jfc/src/runtime/event_loop` non-terminal work | TUI event loop still handles config reload, LSP, background polling, runtime bridges. | Kernel runtime tick/services; terminal loop only translates terminal input/output. |
| `jfc-memory`/`jfc-knowledge`/`jfc-learn`/prompt assembly split | Hippocampus story is distributed by accident. | `jfc-context` as owner; submodules use `jfc-knowledge` storage, `jfc-memory` model, `jfc-learn` jobs. |

## Proposed crate/module target graph

```text
crates/
  kernel/              # event bus, turn state machine, RuntimeServices traits
  runtime/             # AgentRuntime factory, service graph/layer composition, diagnostics
  session/             # typed append-entry log, catalog/search/inbox/task-store derived state
  context/             # layout, contributors, memory/history/reduce/search/health
  policy/              # permissions, shell safety, auto approval, safe mode, trust/provenance
  command/             # slash/command palette command registry and DTOs
  tools/               # descriptor-backed tool registry, built-in tool pack loading
  orchestration/       # subagents, swarm, council, workflows, advisors, goals
  ui-model/            # status rows, panels, widgets, transcript view models; no ratatui dependency
  tui/                 # ratatui shell only
  cli/                 # binary argument parsing and CLI output adapters only
```

This is intentionally more radical than the present crate set. Some names may be
modules during migration, but the ownership map should still be followed: code
that belongs to `context` should not remain under `engine` simply because moving
it is inconvenient.

## Teardown waves

### Wave 0: first boundary cut

Start here before touching Magic Context parity or plugin packs. This is the
smallest cut that changes the direction of the architecture.

- [ ] 0.1 Move transcript save/load/autosave responsibility out of
  `jfc-engine/src/session/core.rs` and `jfc-engine/src/runtime/session_save.rs`
  behind `jfc-session` APIs.
- [ ] 0.2 Introduce typed append-entry DTOs in `jfc-session` while preserving
  compatibility with current saved transcripts.
- [ ] 0.3 Make `jfc-engine` depend on a `SessionStore` trait for save/load/search,
  not concrete serialization functions.
- [ ] 0.4 Split runtime actions into engine-safe actions and frontend directives:
  TUI input handlers submit action requests; the engine returns `FrontendDirective`
  values for UI-only effects.
- [ ] 0.5 Add dependency tests that prevent new session serialization or runtime
  action semantics from being added under `crates/jfc-engine/src` or `crates/jfc/src/input`.

### Wave A: stop the bleeding with service seams

- [ ] A1. Add `RuntimeServices` and `AgentRuntime` interfaces while leaving the
  old `EngineState` in place as a compatibility holder.
- [ ] A2. Replace direct `EngineState` reads in new code with service accessors.
- [ ] A3. Keep enforcing the Engine Root Module Freeze: no new root-level
  `crates/jfc-engine/src/*.rs` product files and no new concrete product domains
  in `EngineState`.
- [ ] A4. Add dependency tests that fail if `jfc-engine` gains new dependencies on
  web/design/economy/daemon/learn/knowledge concrete crates outside service traits.

### Wave B: empty the engine crate

- [ ] B1. Move `jfc-engine/src/session/**` into `jfc-session`; keep wire format
  and tests. Engine receives a `SessionStore` trait.
- [ ] B2. Move prompt assembly (`stream/request/**`, memory recall, runtime
  extension prompt context, project/git/env context, auto hints) into `jfc-context`.
- [ ] B3. Move compaction policy/archive/search into `jfc-context` + `jfc-compress`;
  engine only asks for `ContextUpdate` or `CompactionPlan`.
- [ ] B4. Move slash commands into `jfc-command`; engine gets only command events
  and command effects.
- [ ] B5. Move concrete tool dispatch to `jfc-tool-runtime`/`jfc-tools`; engine
  routes `ToolRequest` through a registry.
- [ ] B6. Move provider bridge/discovery into `jfc-provider`/`jfc-providers`;
  engine depends on a `ProviderRegistry` trait.

### Wave C: reduce the TUI crate to a shell

- [ ] C1. Move plugin UI state/refresh/smoke/doctor management from `crates/jfc`
  into `jfc-plugin-host` or `jfc-plugin-ui`.
- [ ] C2. Move runtime-action semantics out of input handlers; TUI sends
  `RuntimeActionRequest` and applies `FrontendDirective` responses.
- [ ] C3. Move status/sidebar/task-panel data construction into `jfc-ui-model`;
  ratatui modules only paint view models.
- [ ] C4. Move auth/daemon/bridge/remote/memory/audit CLI command logic into
  domain crates; `jfc` only parses CLI args and renders output.
- [ ] C5. Move voice runtime into `jfc-voice`; TUI displays voice state and sends
  voice commands.

### Wave D: create first-party packs

- [ ] D1. Register built-in filesystem/search/edit tools as first-party tool pack
  descriptors instead of enum variants.
- [ ] D2. Register provider backends as provider pack descriptors.
- [ ] D3. Register context/historian/dreamer as a first-party context pack.
- [ ] D4. Register orchestration features (task, swarm, council, workflow,
  advisor, goal) as orchestration packs.
- [ ] D5. Register UI/status panels/widgets through descriptor slots; no domain
  panel reads raw runtime state.

### Wave E: typed append-entry session substrate

- [ ] E1. Define `SessionEntry` variants: user message, assistant message,
  reasoning, tool use, tool result, model change, thinking change, compaction
  boundary, branch/fork summary, custom plugin entry, label, system/context event.
- [ ] E2. Make existing transcript/session JSON load into the entry model.
- [ ] E3. Make task store, background task status, compact archives, and goal
  sidecars derived side tables or typed custom entries.
- [ ] E4. Add migration/repair tooling and a session integrity checker.

## Hard success criteria for the architecture reset

- `jfc-engine` no longer depends directly on `jfc-web`, `jfc-design`,
  `jfc-economy`, `jfc-daemon`, `jfc-learn`, or `jfc-knowledge` concrete APIs.
- `EngineState` stops being the public integration API; extensions and frontends
  interact through services, DTOs, and directives.
- `crates/jfc` has no domain command implementations and no plugin runtime state;
  it is a terminal shell.
- New tools/providers/commands/UI slots can be added by descriptor registration
  without editing a central enum or `match` in the kernel.
- Session persistence is typed append-entry based and can support branch/fork,
  labels, compaction boundaries, and plugin custom entries without sidecar sprawl.
- Magic Context parity is implemented as a context pack on this substrate, not as
  another hard-wired engine subsystem.

## Phase MC-0: harden the storage and health substrate

- [ ] MC-0.1 Finish the SQLx migration above and keep `jfc-session` tests green;
  Magic Context parity depends on async shared SQLite access.
- [ ] MC-0.2 Add a `ContextHealth` record in `jfc-session` or `jfc-knowledge`
  that can store: context layout version, last context rewrite reason, embedding
  provider health, historian state, memory import/verify state, and cache-break
  diagnosis.
- [ ] MC-0.3 Add a TUI/status-panel row and `/context-health` or `/doctor`
  subcommand section that reports the health record without requiring log grep.
- [ ] MC-0.4 Add regression tests for idempotent legacy memory import and
  expected-cancel log classification; these are guardrails for later context
  rewriting work.

## Phase ARCH-0: carve out Pi/opencode-shaped runtime primitives

- [ ] ARCH-0.1 Define `RuntimeServices` as the JFC equivalent of Pi's
  cwd-bound service bundle: provider registry, tool registry, session store,
  context store, task store, config, policy, plugin host, diagnostics.
- [ ] ARCH-0.2 Define `AgentRuntime` as `session + services + diagnostics`, not
  `EngineState`. Keep `EngineState` as a compatibility shell until callers move.
- [ ] ARCH-0.3 Move session persistence toward typed append entries: user message,
  assistant message, tool use/result, model change, thinking change, compaction
  boundary, branch/fork summary, custom plugin entry, label.
- [ ] ARCH-0.4 Turn built-in tools/providers/commands/status rows into
  first-party descriptors registered through `jfc-plugin-host`.
- [ ] ARCH-0.5 Add a typed service graph or registry pattern that enforces
  dependency direction at compile time; no feature may grab `EngineState` because
  it is convenient.
- [ ] ARCH-0.6 Add a crate/module budget: no new root-level `jfc-engine/src/*.rs`
  file unless it is a domain owner; feature internals must live below a domain
  directory.

## Phase MC-1: cache-stable context layout

- [ ] MC-1.1 Design a Rust-native `ContextLayout` with stable prefix, volatile
  delta, and live tail. The Magic Context mental model is m[0] (stable baseline)
  + m[1] (volatile delta), but name the JFC types by domain, not by index.
- [ ] MC-1.2 Move project docs, user profile, durable memories, and decayed
  session history into explicit provider messages with deterministic ordering.
- [ ] MC-1.3 Add cache-break classification: model/tool/system changes,
  memory epoch changes, session-history materialization, explicit flush, and
  emergency overflow.
- [ ] MC-1.4 Ensure defer passes are byte-identical for the stable prefix. Add
  a test that renders the same session twice and byte-compares provider payloads.
- [ ] MC-1.5 Wire prompt-cache diagnostics so cache-read drops point at a known
  invalidator rather than only logging "cache read dropped unexpectedly".

## Phase MC-2: historian compartments and deterministic decay

- [ ] MC-2.1 Define `Compartment`, `CompartmentTier`, and `CompartmentEvent` in
  `jfc-session` or `jfc-learn`, with source raw-message range, importance,
  paraphrase tiers, and validation fingerprints.
- [ ] MC-2.2 Implement a bounded historian prompt in `jfc-learn` that reads raw
  history chunks and returns validated structured output. Use a typed schema, not
  free-form assistant text.
- [ ] MC-2.3 Add deterministic decay rendering: no LLM call on the hot path;
  tier selection depends on age, importance, pressure, and context window.
- [ ] MC-2.4 Add protected-tail boundary logic so the newest meaningful user
  context and open tool arcs are not compacted away.
- [ ] MC-2.5 Add `/expand <archive-or-range>` parity over raw archived history,
  tied into existing compact transcript archives.

## Phase MC-3: memory capture, verification, and curation

- [ ] MC-3.1 Adopt a stable JFC memory taxonomy equivalent to Magic Context's
  PROJECT_RULES / ARCHITECTURE / CONSTRAINTS / CONFIG_VALUES / NAMING, mapped
  onto existing `jfc-memory` levels/scopes.
- [ ] MC-3.2 Make historian output promote project facts only through a host-side
  validator; hidden agents may propose but never directly write durable memory.
- [ ] MC-3.3 Add dreamer jobs for map, verify, broad-verify, curate, classify,
  refresh primers, maintain docs, and smart-note evaluation.
- [ ] MC-3.4 Make every dreamer/historian child output use one validated-output
  retry path, and persist parse failures with the exact task/run id.
- [ ] MC-3.5 Add visible stale/verified/archived counts to memory status and the
  dashboard/TUI panel.

## Phase MC-4: recall and search parity

- [ ] MC-4.1 Unify recall over durable memories, raw session messages,
  compartments, git commits, and codegraph hits behind one `ContextSearch` API.
- [ ] MC-4.2 Add git commit indexing per project with caps, incremental updates,
  and embedding/FTS fallback.
- [ ] MC-4.3 Add embedding health: provider/model, loaded/disabled state, last
  failure, coverage, stale vector count, and repair action.
- [ ] MC-4.4 Add auto-search hints that surface compact recall cues without
  injecting full search results into every prompt.
- [ ] MC-4.5 Add tests for embedding provider failure modes: local runtime
  missing, remote timeout, empty body, malformed JSON, model substitution, and
  caller abort.

## Phase MC-5: context-reduction tools and replay-safe drops

- [ ] MC-5.1 Add a `ContextReduce` tool descriptor or built-in tool that queues
  drops instead of immediately mutating the live prompt.
- [ ] MC-5.2 Persist drop decisions in session state with deterministic replay:
  full drop, skeleton/truncated drop, edit-marker drop, and protected-tail skip.
- [ ] MC-5.3 Ensure dropped tool-use/tool-result pairs remain provider-valid for
  Anthropic/OpenAI-compatible adjacency rules.
- [ ] MC-5.4 Add an `ctx_expand`-equivalent tool to recover exact raw text for a
  compartment, dropped range, or archive id.
- [ ] MC-5.5 Add emergency pressure handling at 85%/95% context use, with
  deterministic oldest-first tool cleanup before LLM compaction.

## Phase MC-6: observability and operations

- [ ] MC-6.1 Add history-integrity counters: healed tool-only gaps, safety-net
  gap heals, rejected narrative gaps, shrink retries, partial-recomp repairs.
- [ ] MC-6.2 Add dashboard/TUI rows for historian progress, compartment coverage,
  embedding coverage, memory verification state, and cache-bust cause.
- [ ] MC-6.3 Add a diagnostics bundle command that exports session health,
  context layout state, recent cache events, and dreamer task failures.
- [ ] MC-6.4 Add schema-drift warnings for any DB/dashboard mismatch with a copyable
  repair command.

## Phase MC-7: plugin-first packaging

- [ ] MC-7.1 Package the entire context subsystem as a first-party plugin pack
  registered through `jfc-plugin-host`, not as special cases in `jfc-engine`.
- [ ] MC-7.2 Expose context tools, status rows, and prompt contributors through
  `jfc-plugin-sdk` descriptors.
- [ ] MC-7.3 Keep the kernel-owned safety policies: permission checks, safe mode,
  prompt-cache invariant checks, and provenance.

## Acceptance criteria for Magic Context parity

- Long-running sessions continue without user-visible compaction pauses.
- Stable context prefix renders byte-identically across defer passes.
- Hidden historian/dreamer outputs are schema-validated and failure-visible.
- Project memory is captured, verified, classified, curated, and searchable.
- `ctx_search`/equivalent search covers memories, sessions, git commits,
  compartments, and codegraph.
- TUI/dashboard/doctor show context health without reading logs.
- Workspace `cargo build`, focused context tests, and relevant session/memory tests
  pass before any parity milestone is marked complete.
