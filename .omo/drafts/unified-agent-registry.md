# unified-agent-registry — Planning Draft

status: awaiting-approval
intent-routing: UNCLEAR / architecture-scale
pending-action: write `.omo/plans/unified-agent-registry.md` after user approves the derived approach

## Request

User asked to read the two newest HTML docs in `docs/`, read the code, and figure out the new implementation. User then supplied PlantUML for the target unified architecture and message/type flows.

## Source evidence read directly

- `docs/jfc-current-architecture.html`: current-state diagnosis — fragmented agent identity/status/state, dual state authority between `BackgroundTask` and `TeamContext`, disconnected messaging, invisible economy/council agents.
- `docs/jfc-unified-architecture.html`: target — one `AgentRegistry`, unified `AgentState`, `MessageBus`, execution backends for in-process, team, daemon, council, economy.
- User PlantUML paste: stronger target — `EngineState` should no longer own `BackgroundTask` map or drifting `TeamContext`; `roster.rs` reads only from registry; dispatch is trivial routing; all agent spawns go through registry; `MessageBus` replaces mailbox/event/stub scatter.
- Current code evidence from direct reads / codegraph:
  - `crates/jfc-core/src/execution.rs`: `ExecutionStatus`/`TaskLifecycle` already provides Pending/Running/Idle/Completed/Failed/Cancelled with lifecycle helpers.
  - `crates/jfc-core/src/ids.rs`: `AgentId`, `TaskId`, `SessionId`, `ToolId` string-compatible newtypes already exist for compatibility.
  - `crates/jfc-engine/src/app/engine_state.rs`: `BackgroundTask` and `EngineState.background_tasks` remain the live UI/task state store; `team_context` remains separate.
  - `crates/jfc-engine/src/swarm/types.rs`: `TeammateIdentity`, `TeammateStatus`, `InProcessTeammateState`, `TeamContext`, `TeammateInfo` still exist; `TeammateInfo.abort_tx` is a runtime liveness handle.
  - `crates/jfc-engine/src/runtime/event_loop/handlers/team.rs`: team runner events currently update `BackgroundTask` and separately clear `TeammateInfo.abort_tx` on terminal events.
  - `crates/jfc/src/render/teammates_panel.rs` plus `crates/jfc/src/render/teammates_panel_tests.rs`: current worktree already has a regression fix/test so the team section prefers `BackgroundTask.status` over `abort_tx` for the confirmed UI bug.
  - `crates/jfc-daemon/src/state.rs`: `BackgroundAgentInfo` persists detached-agent state with `BackgroundAgentStatus`; daemon schema has only Running/Completed/Failed/Cancelled for background agents.
  - `crates/jfc-engine/src/runtime/background.rs`: daemon state is polled and mirrored into `EngineState.background_tasks`.
  - `crates/jfc-engine/src/tools/economy.rs`: `EconomySwarmProvider::send_message` is still an audit-only stub; economy invoker emits Task events so solvers/validators appear as `BackgroundTask` rows.
  - `crates/jfc-engine/src/council.rs` and `crates/jfc-engine/src/council_session/*`: council is provider-agnostic fanout + persistent RoundTable-style session state; seats are not registry-backed today.
  - `crates/jfc-engine/src/tools/dispatch_heavy.rs`: bounty/council-heavy orchestration still lives outside a registry, though dispatch is already partially split into thin/heavy handlers.

## Components ledger

1. Core unified agent domain types (`AgentId`, `AgentStatus`, `AgentRole`, `AgentState`, `AgentProgress`, `AgentResult`, `SpawnConfig`).
2. Agent registry API and in-memory/persistent implementation.
3. Message bus API over in-process channels and file-backed inbox compatibility.
4. EngineState migration from `background_tasks`/`team_context` reads to registry reads.
5. Spawn-path migration for one-shot tasks, teammates, detached daemon workers, council seats, and economy solver/validator agents.
6. UI roster/modal migration to registry snapshots only.
7. Persistence/session/daemon compatibility and migration.
8. Economy slimming / BountyCoordinator relocation after registry-backed solver/validator spawning works.
9. Tests and QA gates for lifecycle, UI drift, daemon restore, messaging, council, and economy.

## Adopted defaults / working decisions

- Treat target as a multi-wave architecture refactor, not a one-shot delete-and-rewrite.
- Create/use a singular lifecycle crate boundary for registry primitives, but do not repurpose existing `jfc-agents` because it currently owns agent/skill definition loading.
- Prefer reusing existing `jfc_core::ExecutionStatus` as the canonical lifecycle primitive instead of adding another duplicate status enum; expose it as `AgentStatus` from the new registry boundary if needed.
- Prefer preserving the existing string-compatible `jfc_core::AgentId` wire shape initially while generating UUID-valued IDs for new agents and mapping legacy task/name aliases. This avoids breaking old sessions/daemon/team files while still moving new state toward UUID identity.
- Do not implement PlantUML's `AgentState` role-specific Option bag literally. Use `AgentRole` enum variants with typed metadata (`Teammate`, `Solver`, `Validator`, `Council`, `Solo`) and keep only truly cross-role optional lifecycle/progress fields optional.
- Keep `TaskLifecycle`/`TaskStatusPart` as model/transcript compatibility until all UI/message surfaces are registry-native; add explicit conversions instead of deleting it early.
- Keep existing `CouncilSession` as deliberation/transcript state in early waves; registry owns council seat lifecycle/visibility, not debate semantics. Delete/merge only after parity tests prove no RoundTable behavior loss.
- Treat the current teammate-panel bug fix as an interim patch, not the final architecture. Final UI must render from registry snapshots, not a `BackgroundTask` row plus a `TeamContext` row.
- Dirty worktree is substantial; final plan must warn worker to preserve unrelated modified/untracked files and avoid broad resets.

## Risks / proof needed before final plan

- Background subagent/session persistence currently uses `TaskId`-keyed `BackgroundTask` and daemon `BackgroundAgentInfo`; UUID AgentId migration needs alias mapping and serde compatibility tests.
- The docs say `NO more BackgroundTask HashMap`, but current code has many render/session/message-path dependencies on `BackgroundTask`; plan must introduce registry-backed adapter before deletion.
- PlantUML says daemon state is pushed back to registry and no more daemon-state polling, but detached workers are separate processes; an IPC/file-backed registry channel must replace polling or a compatibility poll remains during migration.
- Economy still has pools/orchestrator in `jfc-economy`; fully slimming it is high blast radius and should happen only after registry-driven solver/validator invocations work.
- Existing untracked/modified files include partial refactors; final worker plan must differentiate existing intended changes from new work and start with a clean evidence snapshot, not assume main branch state.
- Verification note: `cargo check -p jfc-engine -p jfc` passes on the current worktree. Targeted regression tests that pass: `cargo test -p jfc team_section_uses_background_lifecycle_over_abort_handle_regression`, `cargo test -p jfc-engine teammate_task_id_format_normal`, and `cargo test -p jfc-daemon background_agents_for_restore_filters_by_parent_session_robust`.
- Verification blocker: `cargo test -p jfc-economy test_agent_id_unique` currently SIGSEGVs rustc while compiling `jfc-core`, with the backtrace inside `target/debug/deps/libtracing_attributes-*.so`. Retried with `RUST_MIN_STACK=16777216`, `RUSTC_WRAPPER=`, and `CARGO_INCREMENTAL=0`; still SIGSEGV. Linker overrides to `cc`/`/usr/bin/ld` failed for linker-flavor/libgcc reasons. Final plan should include an environment/toolchain preflight before using economy tests as a hard gate.

## Gate state

Awaiting user approval to generate the work plan. Approval authorizes writing the plan only, not implementation.

Brief to present:
- Treat the HTML docs plus PlantUML as the target architecture, but adapt them to current code by reusing `jfc_core::ExecutionStatus` and the existing typed ID infrastructure.
- Build a compatibility-first registry/read model before deleting `BackgroundTask`/`TeamContext` so old sessions, daemon agents, and the current UI continue to work during migration.
- Use role-variant metadata instead of an Option bag in `AgentState`.
- Route all spawn/write paths through registry/message bus in waves: one-shot tasks, teammates, daemon workers, SendMessage, council seats, economy solvers/validators.
- Finish with UI reading only registry snapshots and deletion/slimming of old duplicate state after parity tests.
