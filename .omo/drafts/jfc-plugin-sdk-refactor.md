# jfc-plugin-sdk-refactor draft

status: awaiting-approval
intent: unclear architecture-scale refactor planning, amended by user preference for all-plugin target architecture
pending_action: write `.omo/plans/jfc-plugin-sdk-refactor.md` after explicit approval
checkpoint_commit: `d05c187 chore: checkpoint dirty workspace before refactor`

## Routing

The request is open-ended: “mirror opencode architecture into Rust” and make JFC bare-bones/minimal with plugins/SDK. I treated this as an UNCLEAR architecture-scale planning task, so I derived best-practice defaults from the repositories rather than asking an interview tree.

Update from the user: the desired target is a big-bang separation-of-concerns model where JFC itself is extremely bare-bones and every meaningful capability is a plugin. I accept that as the target architecture. I do not accept a literal all-at-once implementation commit as the safe execution strategy; the executable plan should build an all-plugin target boundary first, then migrate capabilities through mechanically verified waves.

## Evidence ledger

- JFC dirty tree was checkpointed in `d05c187`; current branch is ahead of `origin/master` by one commit.
- opencode root is a Bun workspace with distinct app/runtime/server/tui/sdk/plugin packages. Root `package.json` depends on `@opencode-ai/plugin`, `@opencode-ai/script`, and `@opencode-ai/sdk`.
- opencode server plugin contract is public in `/home/cole/WebstormProjects/forks/opencode/packages/plugin/src/index.ts`: `PluginInput` exposes a generated client, project/worktree/directory, workspace registration, server URL, and Bun shell; `Plugin` returns `Hooks`.
- opencode hooks include config/event/tool/auth/provider/chat params/chat headers/permission/tool before-after/shell env/experimental transforms/compaction/tool definition.
- opencode runtime host in `packages/opencode/src/plugin/index.ts` loads internal then configured plugins, preserves deterministic hook order, exposes `trigger/list/init`, invokes config/event/dispose hooks, and isolates plugin failures.
- opencode v2 core plugin service in `packages/core/src/plugin.ts` defines typed hooks with `define`, `add/remove/trigger/triggerFor`, scoped lifetimes, keyed locks, and mutable output drafts.
- opencode boot service in `packages/core/src/plugin/boot.ts` registers internal plugins against a typed service graph and exposes `wait()`.
- opencode loader in `packages/opencode/src/plugin/loader.ts` separates spec normalization, target resolution/install, entrypoint detection, compatibility checks, and dynamic import.
- opencode config tracks plugin provenance as `{ spec, source, scope }`, dedupes by plugin identity, and auto-discovers local plugin files under `.opencode/plugin(s)`.
- opencode TUI plugin API in `packages/plugin/src/tui.ts` is separate from server hooks and exposes app/attention/keymap/mode/route/ui/kv/state/theme/client/event/slots/plugins/lifecycle.
- Pi root is at `/home/cole/WebstormProjects/forks/pi`. It is a small package stack: `@earendil-works/pi-ai` for unified LLM APIs, `@earendil-works/pi-agent-core` for agent runtime/tool/state, `@earendil-works/pi-coding-agent` for CLI/TUI/harness, and `@earendil-works/pi-tui` for terminal UI.
- Pi’s README explicitly says the core is minimal and features like sub-agents, plan mode, permission popups, MCP, to-dos, and background bash are intentionally not built in; users add them through extensions, skills, prompt templates, themes, and packages.
- Pi extension API in `packages/coding-agent/src/core/extensions/loader.ts` registers tools, commands, shortcuts, flags, message renderers, providers, model/thinking changes, active-tool changes, and event handlers into an extension object.
- Pi extension runner in `packages/coding-agent/src/core/extensions/runner.ts` binds core actions, context actions, provider registration, UI context, command context, and lifecycle/runtime state after extensions load.
- Pi agent session in `packages/coding-agent/src/core/agent-session.ts` builds the runtime from base tool definitions plus extension results, then refreshes the tool registry from built-ins and extension tools.
- Pi resource loader in `packages/coding-agent/src/core/resource-loader.ts` lets extensions contribute skill, prompt, and theme paths through resource discovery; those resources retain extension source info for UI/diagnostics.
- Pi packages bundle extensions, skills, prompts, and themes through package metadata or conventional directories; package install/update/config is a first-class CLI surface.
- JFC Cargo dependency graph shows `jfc-engine` currently depends on almost every feature crate, while the `jfc` binary depends on almost every crate. This makes `jfc-engine` a feature aggregator rather than a thin orchestrator.
- JFC already has fragmented extension pieces: `crates/jfc/src/cli/plugin.rs` installs/list/removes plugins, `crates/jfc-agents/src/registry.rs` discovers plugin roots for skills/agents, workflow registry discovers plugin workflows, `crates/jfc-engine/src/hooks/mod.rs` has many shell lifecycle hooks, and MCP exposes namespaced tool defs.
- `jfc_provider::Provider` is sealed and explicitly says external extension requires adding a module and registering it internally; this conflicts with plugin/SDK provider extensibility.

## Adopted defaults

| Decision | Default | Rationale | Reversible |
| --- | --- | --- | --- |
| Architecture target | All-plugin target: JFC kernel + SDK + host; every capability registers through plugin descriptors, including built-ins | Mirrors Pi’s minimal-kernel philosophy and opencode’s SDK/host split. JFC itself should become the lifecycle/event/state host, not the owner of providers/tools/agents/web/design/etc. | Yes |
| Plugin granularity | Treat every non-kernel capability as a plugin in the target design, including built-in plugins | Matches the user’s preferred separation of concerns and Pi’s extensibility model. The plan should classify all domains as kernel, host service, or plugin. | Yes |
| Execution mode | Big-bang target boundary, phased implementation waves | A literal single-shot rewrite would likely break Rust dependency direction, persisted sessions/config, providers, and TUI. The plan can be uncompromising about the target while still migrating through verified slices. | Yes |
| External plugin execution | Process/WASM/MCP-backed v1, not native Rust dylibs | Rust has no stable plugin ABI; out-of-process isolates crashes and version skew. | Yes |
| Provider extensibility | Keep internal sealed provider trait but expose external `ProviderDescriptor` + bridge executor | Preserves internal zero-cost/provider safety while allowing third-party providers through stable DTOs. | Yes |
| TUI plugins | Later, separate `jfc-tui-plugin-sdk` or gated module | opencode separates TUI plugin API from server/core hooks; JFC should not leak ratatui/UI into base SDK. | Yes |
| Engine role | `jfc-engine` becomes composition/runtime orchestrator only | Current engine is over-coupled; target is boot/wiring/event orchestration, not feature ownership. | Yes |
| Migration style | Descriptor adapters first, physical code movement second | Allows incremental verification and keeps behavior stable. | Yes |

## Component ledger

1. Core vocabulary layer: `jfc-core` as stable DTOs, ids, message/tool/session/event/error shapes.
2. Public extension contract: new `jfc-plugin-sdk` for manifests, typed hooks, descriptors, compatibility, provenance, bridge DTOs.
3. Runtime host: new `jfc-plugin-host` for discovery, provenance, activation, lifecycle, ordered hook triggering, and bridge adapters. This should be the only place that knows how plugins load.
4. Bare kernel: shrink `jfc-engine` to boot/runtime/session/event orchestration, state machine, permission envelope, cancellation, and host callbacks. No providers/tools/agents/web/design/economy logic in the kernel.
5. Built-in plugin pack: providers/tools/agents/skills/workflows/MCP/session/auth/memory/economy/daemon/web/design/voice/audit/compression all register through the same descriptor path external plugins use.
6. Frontends and APIs: TUI, server/SDK/remote use the kernel + plugin host through stable APIs. TUI extension API can follow Pi/opencode after the server/core plugin spine works.

## Initial migration waves

1. Boundary freeze and crate graph inversion: document kernel/host/SDK/plugin layers, then add dependency direction checks that make the all-plugin target enforceable.
2. SDK + manifest skeleton: create `jfc-plugin-sdk` with manifest, capability, typed hook, descriptor, compatibility, provenance, source-info, and bridge DTOs reusing `jfc-core`.
3. Host spine: create `jfc-plugin-host` as the only plugin loader/activator, initially wrapping existing plugin roots, shell hooks, workflow discovery, MCP tool metadata, and deterministic ordered hook semantics.
4. Built-in plugin pack migration: convert JFC’s existing domains into internal plugins registered through descriptors before any third-party dynamic plugin support is promised.
5. Kernel diet: remove product-domain dependencies from `jfc-engine` until it owns only event/session/state/cancellation/permission/plugin-host orchestration.
6. External bridge wave: add process/WASM/MCP plugin execution for third-party plugins, with provider/tool/resource registration through descriptors.
7. Frontend/TUI extension wave: add a separate TUI plugin API for widgets, overlays, keybindings, palettes, model picker/status panels, and render hooks.

## Must-not-have list

- No native third-party Rust dynamic plugin ABI in v1.
- No `jfc-plugin-sdk` dependency on `jfc-engine`, TUI, concrete providers, or config loader policy.
- No literal unverified one-commit rewrite of all domains at once. The target is all-plugin; the execution still needs wave gates.
- No stringly hook registry except at serialized process/MCP boundaries.
- No direct external implementation of internal `Provider` as the stable extension contract.
- No weakening existing provider/tool/session behavior just to fit the plugin shape.
- No kernel dependency on plugin implementations. Built-in plugins may depend on the kernel/SDK/host; the kernel must not depend back on built-in plugin crates.

## Verification strategy draft

- `cargo metadata`/architecture tests enforce dependency direction for `jfc-core`, `jfc-plugin-sdk`, `jfc-plugin-host`, `jfc-engine`, feature crates, and frontends.
- Golden tests cover plugin ordering, duplicate identity dedupe, skipped/failed plugin isolation, and sequential mutable hook output.
- Lifecycle tests cover activation, finalizers, failed activation cleanup, per-session/global scoping, and dispose exactly once.
- Compatibility tests cover missing entrypoint, unsupported SDK version, invalid manifest, disabled plugin, duplicate plugin, and unavailable bridge backend.
- Provider descriptor tests cover built-in provider path unchanged, external descriptor model/auth exposure, and bridge failure isolation.
- Every migration wave verifies with `cargo fmt --all --check`, targeted crate tests, `cargo build`, `cargo test`, and then `cargo clippy --workspace` when the wave is stable.

## Approval gate

If approved, write one decision-complete plan at `.omo/plans/jfc-plugin-sdk-refactor.md`. The plan will not implement code; it will produce executable waves with exact files, dependencies, acceptance criteria, QA surfaces, and commit strategy.
