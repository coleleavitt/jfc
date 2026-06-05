# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

---

## [Unreleased] — 2026-05-24

### Added

- **System prompt: graph-first code-navigation routing**: session analysis showed the model used Read:Graph at 13:1 and 0% graph on unfamiliar projects (ers-rs, unlace, ClickHouse) because the prompt's opening line and `## Using your tools` only listed `(Bash, Read, Write, Edit, Glob, Grep)` — graph tools weren't named. Added a `### Code navigation — reach for the graph FIRST` section with explicit routing rules (symbol→graph_search, callers→graph_callers, file map→graph_outline, string→graph_grep), plus a note that the graph auto-builds for ANY cwd. Opening line now mentions code-graph tools. Read description updated to recommend offset/limit from graph ranges instead of whole-file reads (4,065 Reads, 0 used offset/limit). Grep description already steered well (99% legit literal patterns)
- **Proactive failure recovery (Agentic Task Graph, arXiv:2605.11951)**: replaces the old destructive cascade (which killed the entire dependent subtree on any failure) with attempt-tracked, dependency-aware recovery. New `Task.attempt_count` field; `recover_from_failure()` classifies failures as transient (timeout/network/lock class) vs hard (compile/assertion/test) — transient + under budget (default 3) re-queues as Pending (bounded retry); over budget or non-transient → hard-fail + create a replan task with dependent subtree rerouted (dependents block on the replan, preserved not destroyed). Replan prompt includes factory health metrics. +6 tests
- **Factory telemetry (Morescient GAI, arXiv:2406.04710)**: `FactoryMetrics` struct (completed/failed/retried/replan counts, success rate, rework ratio, avg attempts) computed from TaskStore. New `/factory` slash command surfaces the production-line dashboard. Metrics embedded in replan task descriptions so the model sees rework rate when diagnosing failures. +3 tests
- **Task-state-drift reminder (auto-nudge to keep the task list in sync)**: the auto-update mechanisms only covered *delegated* work (`parent_task_id` transitions a linked todo) and timestamp bookkeeping — when the model worked *directly* on the plan it had to remember to call `TaskUpdate`/`TaskDone` itself, and often didn't, forcing the user to nudge "update the tasks". `task_drift_reminder()` now detects drift (mutating work happened while tasks are pending-but-none-in-progress, or a task sits in_progress across turns) and injects a `<system-reminder>` on the next turn. Per SWE-agent's Agent-Computer-Interface principle (arXiv:2405.15793), it *surfaces state back to the agent* rather than silently mutating task semantics the model owns. The autonomous self-continuation path already carries an equivalent task-aware nudge. +3 tests

### Fixed

- **Destructive-Bash classifier precedence + missing work-discarding patterns (P0 safety)**: `classify_tool_use` checked `is_explicitly_allowed` *before* the destructive denylist, so a broad `Bash` (or `Bash(git*)`) allow-rule could wave through a work-discarding command. The denylist now runs first for Bash. Also expanded `DESTRUCTIVE_BASH_PATTERNS` to cover the "lost state" class the session audit flagged but the list missed: `git reset --hard` (was only `…origin`), `git clean -fd`, `git checkout -- `/`git checkout .`, `git restore `, `git branch -d/-D`, `git stash drop/clear`, `git worktree remove --force`, `git reflog expire`, `git update-ref -d` (and fixed the dead `git clean -fdX` upper-case entry that could never match a lowercased command). Single `destructive_bash_match()` helper now shared by the UI label and the classifier so they can't drift. These complement the independent catastrophic backstop (`shell_safety.rs`) that already gated whole-system ops even in Bypass. +2 tests
- **Subagent `parent_task_id` completion now advances linked plans (P0 lifecycle)**: the manual `TaskDone` path called `advance_linked_plans`, but the subagent completion path (`handle_task_completed`) only flipped the linked todo to Completed — so a plan never advanced when its work was *delegated* instead of done inline. Both paths now share the `advance_linked_plans` hook (re-exported `pub(crate)` from `tools::dispatch`).
- **Factory stall — stuck-task reaper + same-burst race fix**: when the leader went fully idle with a task still `InProgress` (a turn ended without `TaskDone`, or a crash left the claim dangling), the queue stalled until the user nudged "continue". `maybe_continue_task_factory` now reaps factory-owned stuck tasks (`TaskStore::requeue_stuck`) before claiming, so the loop self-heals. Fixing this surfaced a latent race: the factory enqueues `Submit` to a *later* event-burst but never set `turn_started_at`, so two factory-triggering events (`Tick`+`TaskFailed`/`AllComplete`) in one burst could each claim+submit — two concurrent turns (and, with the reaper, a requeue of the just-claimed task). New `commit_factory_turn()` sets `turn_started_at` before the Submit so the function's first guard makes any same-burst re-entry a no-op. +3 tests (incl. the double-submit regression)

- **Diff-aware stub evaluator (root-cause fix for false-positive `TaskDone` rejections)**: `evaluate_work_quality` previously scanned *whole files* via `scan_file`, so any pre-existing `placeholder`/`no-op`/`silently drop` doc comment in a file the change merely touched would block task completion. New `scaffold_detector::scan_added_lines()` evaluates only the `+` lines of `git diff HEAD` (per-file, `--unified=0`), so the gate flags stubs the change *introduced*, not patterns that were already there. Untracked new files (no diff base) are still scanned whole. +4 tests

### Added

- **Per-task `effort` + `model` overrides on persisted tasks**: subagent specs already carried `effort`/`model` (Task.effort > AgentDef.effort > global), but the *persisted* `Task` couldn't, so factory auto-continuation always ran at the session default. Added `effort: Option<String>` + `model: Option<String>` to `Task`/`TaskPatch` (`jfc-core`), exposed them on the `TaskCreate`/`TaskUpdate` tool schemas, and threaded them into the factory continuation prompt so a hard task can request `max` effort / a stronger model and a trivial one a cheaper model. Serde-skipped when `None`; round-trips verified. +1 test
- **Self-continuation guard (the "factory" behavioral half)**: derived from analyzing 133 turns where the user had to type "continue" — ~41% ended with the assistant *asking permission for the next obvious step* ("Want me to …?", a trailing question, "shall I", "let me know"). New `assistant_text_stalls()` detects these conversational stalls (tail-window phrase + trailing-question match), and `stream_done`'s terminal EndTurn path now auto-drives the next step via `continue_agentic_loop` when (a) the model stalled or (b) queued tasks remain. Gated by `[continuation] auto_continue` / `JFC_AUTO_CONTINUE` (factory mode implies it), disabled in plan mode, and capped by `max_self_continuations` (default 25, reset on every real user submit) to prevent runaway loops. The injected nudge phrases the operating rule explicitly: finish the scope, only pause for genuine forks. +6 tests
- **Graph content index (persistent cache for all body-read paths)**: new `content_index.rs` — a `DashMap`-backed, mtime-validated line cache + revision-gated per-file symbol-span index. Eliminates the perf gaps across **every** graph read path: (1) `graph_grep`, `graph_node`, `graph_search include_code`, and `graph_explore` now share one mtime-validated line cache instead of each re-reading files from disk on every call; (2) enclosing-symbol lookup is a binary search over a cached, start-sorted span list instead of an O(M×N) scan of every graph node per match. Invalidated per-file on `file_changed`. Measured: `graph_grep` 54ms→10ms (5.5×) on warm repeat; `graph_node` on a second symbol in an already-warm file 1.28ms→451µs (2.8×). `graph_status` reports warm-file count. +3 tests
- **Graph engine ergonomics — closing the search→sed loop**: derived from analyzing ~35k tool calls across 264 jfc + 45 codex sessions (where the model ran 6,014 `sed -n` line-reads, 1,603 `nl` calls, and 2,145 literal greps the symbol index couldn't answer). Six fixes: (1) `graph_search include_code=true` inlines each hit's full source body, collapsing the `search → sed` round-trip into one call; (2) new **`graph_outline(file)`** tool returns every symbol + `:start-end` range, replacing `nl -ba`; (3) new **`graph_grep(pattern, glob?)`** does regex content search enriched with the enclosing symbol — serves the log/error/string-literal searches the graph couldn't; (4) search results now show full `:start-end` line ranges (not just start) so callers `Read offset` / `symbol_edit` precisely; (5) `lang_for` covers all 12 adapter languages in code fences; (6) function search/node results append a `graph_callers`/`graph_callees` hint to shift the grep-for-callsites reflex. +7 graph tests
- **Wired remaining stubs into runtime**: `/learn dream` runs the real Dreamer cycle (lease + 5 tasks over loaded memories); `/learn status` + `/learn user-profile show` use the live `UserMemoryPipeline` (observation counts, promotion check, profile rendering); `/learn historize` reports staged-transcript readiness (LLM extraction runs from the daemon scheduler). PlanDreamer `verify`/`improve`/`maintain_docs` now do deterministic maintenance (flag empty Active plans, normalize whitespace bodies, backfill `last_advanced` baselines) instead of `Ok(0)`. Graph `Stub{ResolveSymbols,InferTypes}Pass` renamed to `Demo*` with genuine read-only traversals. MCP `Transport::request()` returns an actionable unsupported-method error instead of a silent 30s timeout
- **Remote control** (`jfc-remote` crate + `jfc-ui` integration): drive a jfc session from another device over WebSocket. Wire protocol (`RemoteEnvelope`, 13 variants) with HMAC-SHA256 frame attestation, monotonic sequence replay rejection, and bearer-token pairing. `WsServer` binds `127.0.0.1:4242`; expose via Tailscale/SSH tunnel/cloudflared. Multi-client broadcast fan-out (N devices, same stream). Host mirrors streaming events, tool calls, results, toasts, plan approvals, and **permission requests with diff preview** (Edit/Write/MultiEdit/Bash). Status tracking: transition-only `Running`/`WaitingApproval`/`Idle` derived post-burst. 20s heartbeat keepalive. Client interactive approval: `y`/`n` responses mapped to `ApprovalResponse`/`PlanApprovalResponse`. CLI: `jfc rc connect` and `jfc rc status`; launch flag: `--remote-control`/`--rc`; config: `[remote_control]` section; slash: `/remote-control` (`/rc`) toggle. See `docs/remote-control.md`
- **Scaffold detector — third-pass vocabulary**: added doc-comment `/// Stub:` prefix (High), `not yet wired` (High), `intentionally stubbed` (Medium), `tech debt` (Low), and corroborated `first/initial pass` (Low) patterns, derived from a third audit pass over the session corpus + the live codebase. 30 detector tests
- **Weighted scaffold/stub detector** (`scaffold_detector.rs`): replaces the binary `STUB_PATTERNS` list with a regex-based, severity-weighted detector (Critical 100 → Info 15) and category tags (unimplemented/placeholder/scaffold/hedge/shim). Vocabulary + weights derived from an audit of the session corpus. Context-aware: code-vs-comment distinction, test-file downgrade, `shim` requires corroboration, and bare `let _ =` is no longer flagged. Task-completion gate now blocks only on a `Critical` finding or cumulative weight ≥ 160, and also scans untracked new files
- **Slop guard**: 11 new quality checks from academic literature (duplication ratio, dead-code injection, churn detection, coherence scoring, complexity gates, test quality heuristics)
- **Claude Code 2.1.150 parity**: port remaining features including bridge attestation, idle prefetch, web cache, inline tools for non-native providers
- Wire all remaining dead-code modules into runtime triggers (dreamer scheduler, plan dreamer, speculation, coaching, session recap, sprint budgets)
- **Eager tool dispatch**: tools now execute immediately as they arrive mid-stream instead of waiting for StreamDone, eliminating perceived queuing latency
- **Task effort field**: `effort` parameter on Task tool for per-subagent reasoning effort override (low/medium/high/xhigh/max)
- Auto-link Task delegation to sole in-progress task when model omits `parent_task_id`
- All graph tools (GraphSearch, GraphContext, GraphNode, GraphExplore, GraphCallers, GraphCallees, GraphImpact, GraphStatus, GraphFiles) now auto-approved in plan mode
- Event loop burst-drain cap (256 events/iteration) to prevent producer starvation

### Fixed

- `sed` shell-safety parser now correctly distinguishes script arguments from file paths (no more false-positive rejections on `sed 's/foo/bar/' filename.rs`)
- Char-boundary safe truncation in `SendUserMessage`, `session_recap`, and WebFetch tool result cap (prevents panics on multi-byte UTF-8)
- Eager dispatch counter tracking: turn only completes when `in_flight_eager_dispatches` reaches 0
- Task failure cascade now uses linked task id (the queued todo) instead of agent task id
- Workflow runner: fix indentation of `schema` field in TaskInput construction
- Remove agentic turn cap (was 200, now unlimited) — configurable via `JFC_MAX_AGENTIC_TURNS` if needed

### Changed

- README expanded to document all 21 crates (added `jfc-session`, `jfc-tools`, `jfc-remote`), missing subsystems, and graph-discovered features
- **License**: fix `Cargo.toml` workspace license from `MIT` to `AGPL-3.0` to match `LICENSE` file
- **Cargo.toml**: add `repository`, `homepage`, `keywords`, `categories` metadata
- **Crate docs**: add `//!` crate-level doc comments to jfc-economy, jfc-graph, jfc-markdown, jfc-memory, jfc-provider, jfc-providers, jfc-theme
- **Formatting**: `cargo fmt --all` sweep across 93 files to normalize workspace formatting
- **.gitignore**: stop tracking runtime state (`.jfc/tasks.json`, `.jfc/learn/`, cache/height files, per-crate `.jfc/` dirs)

### Added (project infrastructure)

- **GitHub Actions CI** (`.github/workflows/ci.yml`): `cargo fmt --check`, `cargo clippy`, `cargo test`, and public-build verification on push/PR
- **CONTRIBUTING.md**: build instructions, test commands, PR process, coding style, feature flags
- **SECURITY.md**: vulnerability reporting via GitHub advisories or email, scope table, disclosure policy
- **CHANGELOG.md**: comprehensive history from all 401 commits
- **.editorconfig**: UTF-8, LF, 4-space Rust indent, 2-space TOML/YAML
- **.gitattributes**: `diff=rust` for `.rs`, linguist-vendored for `research/`, binary rules for images

---

## [0.9.0] — 2026-05-23

### Added

- **Go IR lowering** + wire IR into GraphBuilder for cross-language analysis
- **Interprocedural points-to analysis** with `PointsToOracle`
- Generic params + callee type args emission from Rust/TS adapters
- `graph_node` + `graph_explore` native tools for targeted symbol inspection
- `graph_status` / `graph_files` tools for graph health introspection
- Foundations for alias analysis, monomorphization tracking, and IR lowering (t269/t271/t273)

### Fixed

- Auto-wake idle leader + drain meta-prompts mid-stream
- Reject `Function` as `Contains`-edge source in graph
- 3 stale rust adapter call-edge tests

### Performance

- Skip `Paragraph::line_count` for short lines + persist highlight line counts
- Persist tool-height cache to eliminate 1s `--continue` gap
- Fast header-scan for `--continue` session lookup

---

## [0.8.0] — 2026-05-22

### Added

- **jfc-learn**: E2E 3-session learning test, ASG-SI verifier promotion gating, Historian wiring
- **jfc-learn**: AutoSearchHints pre-turn recall injection
- **jfc-plan**: plan↔task reverse linkage + PlanRecall request-pipeline gate
- PlanDreamer + jfc-learn Dreamer scheduled from daemon
- Expose `graph_context`/`search`/`callers`/`callees`/`impact` as MCP tools
- **jfc-graph**: cross-file call resolver, codegraph-grade markdown context output, schema/worktree/data_dir/overlay modules, GraphSession API
- **jfc-graph**: full codegraph feature parity — Kotlin + Swift language adapters
- **jfc-audit** and **jfc-learn** crates added to workspace
- Plan store + learn/recall tools
- Slash command dispatch collapsed through macro registry
- Wire plan + learn tools through dispatch — zero dead-code warnings

### Fixed

- Memory/threading bugs from profiling audit
- Unstick stale `tool_title_width` tests + NodeData test fixture

### Changed

- rustfmt sweep across graph adapters, cfg, dataflow, complexity modules
- Remove network EKG sparkline + dead tick handler

---

## [0.7.0] — 2026-05-21

### Added

- **Workflow system**: complete multi-agent orchestration scripts with `agent()`, `parallel()`, `pipeline()`, `phase()` primitives — CC 146 parity
- **PHP language adapter** for jfc-graph
- Teammates panel with agent navigation and background completion tracking
- Enhancement plan for codegraph feature parity roadmap

### Fixed

- Stabilize context gauge, immediate keypress echo, and reliable bash output streaming
- Correct mention popup chunk index, unicode-aware prompt width, needs_draw flags
- Prevent AllComplete emission before dispatched tool finishes in approval queue
- Security and correctness hardening across daemon, tools, providers
- Plan-mode CodeGraph MCP approval, keyboard enhancement timeout, action-intent detection

### Changed

- Replace hand-rolled MCP protocol with the `rmcp` SDK
- Harden event delivery and fix approval queue batching
- Remove narration-retry mechanism in favor of prompt-level discipline
- Extract jfc-agents, jfc-tools crates from jfc-ui monolith
- Wire jfc-core ExecutionResult + DiffView into jfc-ui
- Deduplicate notebook tool + shrink `built_in_agents()`

---

## [0.6.0] — 2026-05-20

### Added

- **Gemini provider**: proper `thought_signature` round-trip, model picker source labels, dynamic model listing, direct API key support
- **Antigravity (Google) OAuth provider**: Claude-via-Antigravity streaming + model auto-dispatch, Gemini streaming via Code Assist API
- **Inline tool XML interception** from LiteLLM/Bedrock for non-native providers
- Agent fan and pinned-tasks panels with rounded block borders
- Atomic mailbox reads, interruptible tool exec, prose-less success, billing propagation for swarm

### Fixed

- Harden agent market (agentic solvers, real adjudication, stable IDs, budget gate, charter enforcement)
- Harden audit-identified bugs (locks, leaks, watchdog, compaction)
- Gate subscription-locked betas per-account in Anthropic OAuth
- Finish botched runner split — wire coordinator, kill duplication

### Changed

- **Major refactoring wave**: split god-functions and extract crates
  - `render.rs` (5,615 lines) → `render/{frame,sidebar,visual,messages,agents,input_box,overlays,tests}.rs`
  - `input.rs` (7,102 lines) → `input/{key_dispatch,submit,slash_commands,tests}.rs`
  - `handle_key` (1,489 lines) → 198 lines
  - `execute_tool` (1,326 lines) → 767 lines
  - `permissions.rs` (1,391 lines) → 192 lines
  - `tools/mod.rs` (2,335 lines) → `tools/{dispatch,registry,safe_tools,tests}.rs`
  - `types/tool.rs` (2,529 lines) → `types/{tool_display,tool_call,tool_output}.rs`
  - `session/serialization.rs` (2,284 lines) → `session/{serialization,serialize,deserialize,serialization_tests}.rs`
  - `swarm/runner.rs` (2,022 lines) → `runner/executor/coordinator + runner_tests`
  - `agents.rs` (1,485 lines) → `agents/{state,lifecycle,registry}.rs`
  - `message_view/outputs.rs` (1,838 lines) → `message_view/{outputs,formatters,truncation}.rs`
  - `slash_commands.rs` (2,493 lines) → `slash_commands/{core,ext,ext2}.rs`
- Extract **jfc-markdown** crate from `markdown.rs` (2,611 lines)
- Extract **jfc-theme** crate from `theme.rs` (1,162 lines)
- Macro-ify ToolInput's exhaustive methods (drift-proof table)
- Macro-driven slash-command registry

---

## [0.5.0] — 2026-05-19

### Added

- **jfc-daemon** crate extracted (cron, pid, state, logs, registry, reconcile, runtime, worker spawn)
- **jfc-providers** crate extracted + move cost/content to jfc-provider
- **jfc-mcp** crate extracted with zero warnings
- Priority message queue + agent transcript + mid-loop drain + interrupt-on-submit
- Dispatch safe tools mid-stream — eliminates batch barrier
- CC 2.1.144 parity batch: task UI, plan mode, LSP, OWUI hooks, effort config
- Narration-only EndTurn guardrail with tool_choice retry
- Compaction: observation masking, file restoration, speculative compact, custom instructions
- Temporal awareness: full time gap markers + evaluator stub patterns
- Sidebar: token breakdown, pinned files, session age, temporal awareness
- Dim queued user messages + `[queued]` tag for visual distinction
- `/dream`, `/powerup`, `/voice`, `/deep-link`, idle-return, session header, 15s timeout

### Fixed

- Reset `cancel_token` on every new turn (fixes spurious "Interrupted by user")
- 8 critical+medium bugs from streaming/tool/input audit
- Per-file mutex prevents Edit/Write/MultiEdit race on same path
- Merge consecutive Text parts to fix 156-fragment bug
- Remove rejected Anthropic beta headers (mcp-servers, context-hint, ccr-byoc)
- Persist permission mode on `/mode` + fix image chip UX
- Remove false-positive budget warning from char-based estimate

### Changed

- Move AgentDef + PermissionMode + Effort to jfc-core
- Remove blanket `#![allow(dead_code)]`
- Split event_loop.rs into per-handler modules

---

## [0.4.0] — 2026-05-15 — 2026-05-18

### Added

- **Sprint system**: project-level task persistence + sprint boundaries + evaluator gate + per-turn budget injection
- **Anthropic pause_turn resume** + multi-message turn guard
- Redacted thinking + `previous_message_id` + new beta headers
- Byte-faithful `server_tool` round-trip + warn on missing stop_reason
- Crash-safe writes via temp + fsync + rename
- Auto-commit, `context_hint`, cache diagnosis
- **Intent classification**: auto-detect doc requests + plan-mode posture + `--permission-mode` flag
- **Goal loop**: `/goal` as a real stop-condition with evaluator + iteration loop
- **Slop guard**: implement full AI slop detection system
- `/plan`, `/roadmap`, `/parity`, `/philosophy`, `/usage` with strict format contracts

### Fixed

- Char-boundary safe string truncation across 4 panic sites
- Route mixed-mode `pause_turn` through resume builder after local tools complete
- Serialize graph-cache test to stop parallel flake
- Link delegated agents to queued tasks + reload TaskStore
- Stop dispatching terminal/denied tool calls

### Changed

- Remove hallucination guard, improve prompt caching + request tracing
- Dedupe substream setup + coalesce on save
- Extract runtime contracts and module boundaries
- Refactor event_loop into directory module + handler submodules

---

## [0.3.0] — 2026-05-13 — 2026-05-14

### Added

- **OpenWebUI full auth lifecycle**: JWT + OIDC + Duo MFA + auto-refresh + CLI (`jfc auth litellm`)
- **Single-ESC instant kill** + synthetic `tool_result` injection for clean cancellation
- **Enhanced planning**: TaskValidate, risk/kind/hierarchy, plan cache, unlimited agents
- **Adaptive re-plan**: scratchpad, consolidation, plan verification (literature-gap implementation)
- **Hallucination guard v2**: category mapping, three-state verdict, log-only mode
- **Image support**: prompt-local `[Image #N]` model + resize + clipboard fallback
- Task/subagent view now matches main chat view quality
- Unified task/main view under single `RenderCtx` renderer

### Fixed

- Major memory bugs found via dhat profiling (eliminated)
- Task-factory stall after detached completion + concurrency cap
- Serialize TaskStatus to provider + inject completion reminder
- Remove process-global attachment queue — per-message ownership
- Stop dropping tools + guard against hallucinated 'done' claims
- Teammate `abort_tx` leak + task store migration
- Use `floor_char_boundary` for all user-facing string truncations
- Unify token accounting + stop queued prompts from polluting the prompt
- Lifecycle bug sweep across event/swarm/daemon/compaction paths
- Suppress spurious config-updated toasts from sibling files

### Performance

- Cap TaskStatus, dedupe memories, overhead estimate, microcompact, budget logging
- mtime-gate daemon-state polls + cap terminal agents
- Cache agent config in OnceLock to avoid per-spawn disk I/O

---

## [0.2.0] — 2026-05-09 — 2026-05-12

### Added

- **LiteLLM proxy provider** with dynamic model fetching
- **Codex OAuth foundation** with browser login, device flow, status, logout
- **Claude Code 2.1.139 feature parity** batch (command and keybinding parity)
- **jfc-graph overhaul**: CSR (compressed sparse row), push/pull BFS, Tarjan SCC, DSL optimizer, aggregation, incremental cache, typed metadata, multi-language adapters, HyperLogLog, stratification
- **WebSearch**: arXiv + Semantic Scholar backends, `papers:` prefix for parallel dedup search
- Coverage pass, possible-types propagation, DSL piping
- Graph capabilities wiring and partial field metadata
- Theme picker, streaming render cache, draw-coalescing fix
- Inline color swatch rendering for hex and rgb literals
- v137 features + critical `kill(-1)` fix
- TaskGet tool
- Adaptive tick rate, kinetic scroll, cached theme styles
- Colorize git commit/push and diagnostic prefix output in message view
- OpenAI reads key from `~/.config/jfc/credentials.toml` fallback

### Fixed

- Stream watchdog to prevent stuck 30fps animation loop
- Spinner stuck after ExitPlanMode
- Tighten model picker filter to exclude non-chat OpenAI models
- Batch-drain all queued prompts in one turn (v137 parity)
- Remove browser header + update version fallback in OAuth
- Render color swatches in inline code (backtick) spans
- Hydrate detached agent progress

### Performance

- Replace pulsing cursor glow with static tint
- Reduce tokio worker threads from 24 to 4
- Warm tool-height cache after session load
- Eliminate redundant `build_render_items` + `getcwd` syscalls
- Cache `message_view_total_lines` to eliminate per-frame O(n×m) scan

### Changed

- Expand jfc-graph engine and jfc-ui integration
- Split app state into modules
- Decompose message_view into modular subfiles
- Expand daemon, SDK APIs, and swarm infrastructure

---

## [0.1.0] — 2026-05-07 — 2026-05-08

### Added

- **MCP support**: full Model Context Protocol with stdio/SSE transports, tool registry, dynamic dispatch (v132 parity)
- **Daemon**: fleet daemon with session management, cron, wakeups, socket API, CLI commands
- **GitHub integration**: deep integration via `gh` CLI for v2.1.132 parity
- **Advisor**: parallel `/advisor` mode with snapshot context + budget
- **Memory**: two-phase LLM-driven memory recall
- **Speculation**: pre-run tools in fs overlay for zero-latency commit
- **Slate**: dynamic per-turn model routing with `QueryClass` classifier
- **Fleet view**: ratatui fleet dashboard
- **Swarm**: team memory, fork, mirrors, teleport, turn classifier
- 8 v2.1.132 model-callable tools (EnterWorktree, ExitWorktree, CronCreate, etc.)
- Bedrock, Vertex, Console OAuth scaffolding
- Streaming bash output with real-time line-by-line progress
- Retry utility with exponential backoff + jitter
- Per-session log files (replacing daily rolling)
- System prompt expanded to match Claude Code's structure
- Slash command dispatcher + reasoning effort control
- Git session context module
- tmux session integration for agent worktrees
- Lifecycle hook system expanded to 31 points
- Categories, per-agent permissions, prompt customization, reasoning effort, top_p, variant, MCP, experimental flags
- v132 batch session work: PDF, plan-mode, system-reminder, output-style, retry, slash-cmds
- Bash output highlighting for jq, sed, awk, curl, cargo
- `StableGraph`, bridges, feedback-arc-set, dijkstra, dot-export, k-shortest-paths in graph

### Fixed

- Graph: spawn build on 64MB stack thread instead of tokio worker
- Graph: add size guards to O(V²) algorithms to prevent stack overflow
- Graph: prevent infinite recursion on symlink cycles in `walk_dir`
- Graph: replace hand-rolled `walk_dir` with `ignore` crate
- Render stall between stream phases + skip `research/` in graph walker
- Raise max agent turns from 20/25 to 200

### Performance

- All session saves made non-blocking (4+ sites)
- Non-blocking session save on submit + approval

---

## [0.0.2] — 2026-05-06

### Added

- **jfc-economy crate**: bounty marketplace with real LLM-driven solver/validator agents, worktree dispatch, adversarial adjudication, trust scoring, token ledger
- **jfc-graph crate**: code graph with LSP client, `preconditions` DSL operator, path-condition extraction, disk-persist large tool results, auto-queue cascade, `/cascade`, `graph_query` + `symbol_edit` tools
- **Orchestration**: argus review, ralph loop, tmux, comment check, handoff
- **Background agent manager** + Landlock sandbox
- **Lifecycle hook system** + intent classification
- **Hashline content-anchored edits** + TOML permission automation
- Feature flag scaffolding, test infra, config system
- Per-subagent live tool + token counters in fan UI
- Swarm auto-compaction ported from Claude Code v131
- Full petgraph algorithm suite leveraged in graph

### Fixed

- Require mechanistic bounty verification
- Wire up post→run dispatch, lift per-bounty budget cap
- Stop spinner/counter after stream interrupt (ESC×2)
- Cap subagent + teammate request bytes to prevent context-window blowups
- Cell-aware wrap, table reflow, scroll math
- Tool block height undercount bugs
- Correct scroll math and refactor bottom-area UI

### Testing

- +155 tests for input.rs key handler (5% → 79% region coverage)
- +103 tests for app/compact/context/scheduler/session (≥75% region coverage)
- +125 tests for providers/* and stream.rs (≥70% region coverage)
- +94 tests covering all swarm/* modules (0% → 92.88%)
- +167 tests for render + message_view pure helpers
- +197 tests across tools/notifications/theme/types
- Full orchestration pipeline integration test (E2E)

### Changed

- Broad cleanup across economy, graph, and ui crates
- Shared streaming HTTP client, async SwarmProvider, OAuth cleanup

---

## [0.0.1] — 2026-05-04 — 2026-05-05

### Added

- **Swarm multi-agent system**: memory persistence, notifications, major tool/rendering expansions
- **Themes**: directory-based skills, teammate lifecycle
- **Skills**: system-prompt injection, slash invocation (`/skill-name`), Skill tool model invocation, AgentDef.skills consumer
- **Session management**: cwd-scoped `/continue`, `/rename`, display title fallback, per-message usage for token gauge restore
- **Permission modes**: Default, Plan, AcceptEdits, Auto, Bypass (v126 parity)
- **v126-style token tracking**: include tool input JSON in spinner estimate
- **Model picker**: pricing/context columns from models.dev
- **Agent frontmatter**: effort, maxTurns, memory, mcpServers, hooks
- **LSP client** orchestration
- **Image attachments**: data layer + arboard image-data feature
- **Cost meter** pricing + sidebar total
- **MCP sidebar** rendering
- **Worktree slash commands** + session syntax/blink/history fixes
- Click-to-expand tool blocks
- Diff syntax highlighting inside +/- lines
- Config.toml per-agent models
- Configurable prompt animation modes
- two-face: ~250 syntect grammars + 32 themes
- Toasts, @mention autocomplete, diagnostics panel, cumulative usage
- Task tool, LargeText collapsing, background tasks, MCP/LSP support
- Subagent task view uses MessageView (markdown + xml-strip + collapse)
- Compaction: visible spinner, retry, monotonic counter, live output tokens

### Fixed

- Context token counter reset on session continue
- Tasks lost on session continue (use `TaskStore::open` instead of `in_memory`)
- Compaction retry loop, session compat, cost log spam, render cache
- Compaction suppress after permanent failure + thinking fallback
- Edit tool line counts
- Don't reset token counter mid-turn during agentic loop
- Resolve assistant message prefill 400 errors on Opus 4.6+ and Bedrock/LiteLLM
- Case-insensitive tool name normalization for cross-provider proxies
- Bedrock prefill — strip trailing empty assistant

### Changed

- Upgrade ratatui 0.30, crossterm 0.29, fix lru vulnerability
- Model-aware `max_output_tokens` + system-prompt sanitization

---

## [0.0.0] — 2026-05-03

### Added

- **Initial release**: jfc-ui binary with GPUI-based rendering
- Port from GPUI to **ratatui TUI** (same day)
- v126 baseline: agents, skills, tasks, CLAUDE.md hierarchy, tool-name normalization
- Typed tool rendering harness
- wgpui damage_buffer/opaque region support (pre-ratatui)

---

[Unreleased]: https://github.com/coleleavitt/jfc/compare/a99e10d...HEAD
[0.9.0]: https://github.com/coleleavitt/jfc/compare/7586868...a99e10d
[0.8.0]: https://github.com/coleleavitt/jfc/compare/8541a2e...7586868
[0.7.0]: https://github.com/coleleavitt/jfc/compare/3ec14d2...8541a2e
[0.6.0]: https://github.com/coleleavitt/jfc/compare/8d3d2fe...3ec14d2
[0.5.0]: https://github.com/coleleavitt/jfc/compare/7d61396...8d3d2fe
[0.4.0]: https://github.com/coleleavitt/jfc/compare/b8b2a9b...7d61396
[0.3.0]: https://github.com/coleleavitt/jfc/compare/5b341c1...b8b2a9b
[0.2.0]: https://github.com/coleleavitt/jfc/compare/3b3c326...5b341c1
[0.1.0]: https://github.com/coleleavitt/jfc/compare/5dd9037...3b3c326
[0.0.2]: https://github.com/coleleavitt/jfc/compare/0261ae5...5dd9037
[0.0.1]: https://github.com/coleleavitt/jfc/compare/80ca947...0261ae5
[0.0.0]: https://github.com/coleleavitt/jfc/compare/035b0ab...80ca947
