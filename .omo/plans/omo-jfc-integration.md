# OMO → JFC Architecture Integration

## TL;DR

> **Quick Summary**: Port the most impactful oh-my-opencode (OMO) architectural patterns into JFC's Rust TUI — focusing on Hashline content-anchored edits, permission automation, lifecycle hooks, background agent spawning, intent classification, Landlock process sandboxing, and advanced orchestration loops.
> 
> **Deliverables**:
> - Hashline edit system (content-hash-anchored line addressing for reliable edits)
> - Permission automation engine (TOML-driven allow/deny rules)
> - Lifecycle hook system (8 hook points with enum dispatch)
> - Background agent manager (parallel specialist spawning with session isolation)
> - Intent classification gate (heuristic-only, no LLM round-trip)
> - Landlock/seccomp process sandbox (for economy solver agents on Linux)
> - Argus-style code review agent profile
> - Ralph-style continuation loop
> - Tmux-integrated interactive tool
> 
> **Estimated Effort**: XL (5 phases, ~28 tasks)
> **Parallel Execution**: YES - 5 waves
> **Critical Path**: Phase 0 → Phase 1 (Hashline + Permissions) → Phase 2 (Hooks) → Phase 3 (Background Agents) → Phase 4 (Orchestration)

---

## Context

### Original Request
User asked to read OMO and JFC codebases, identify overlapping architecture, and create ONE comprehensive plan covering everything worth integrating.

### Research Findings

**From OMO (oh-my-opencode)**:
- 11 specialized agents with distinct system prompts and tool permissions
- 52 lifecycle hooks across 5 tiers (Session → Tool-Guard → Transform → Continuation → Skill)
- 26 built-in tools with hash-anchored edits achieving 6.7% → 68.3% edit success improvement
- Permission automation via config-driven rules (auto-approve patterns, deny-list, escalation)
- Background agent spawning with session isolation and result collection
- IntentGate classifies user intent before routing (research/implementation/investigation)
- Skill-embedded MCPs (Model Context Protocol servers bundled with agent skills)
- Argus security review, Ralph continuation loop, Prometheus planner as orchestration patterns

**From Codex-rs (OpenAI)**:
- Landlock LSM + seccomp for Linux process sandboxing
- Network proxy with allowlist for solver isolation
- exec_policy.rs for configurable execution restrictions (read-only, network-disabled, full-auto)

**JFC Already Has**:
- Standalone Rust TUI with render cache and virtual scroll
- Session persistence, task/todo system, memory system
- Swarm/team with permission_sync between leader/worker agents
- Multi-account OAuth (Anthropic, OpenAI, OpenWebUI)
- Cost tracking per model/session
- Economy sandbox (bounty/auction/validator with mechanistic verification)
- Inline tool parsing, LSP integration
- Slash commands, graph context engine

### Metis Review

**Identified Gaps (addressed in plan)**:
- Runtime model mismatch: OMO hooks are sync JS middleware → JFC needs async enum-dispatched hooks
- Config format decision: TOML static config (no scripting runtime) for all new features
- Edit tool assessment: Need to measure JFC's current edit success before committing to Hashline
- TUI output routing for background agents: Collapsed panel with expand-on-demand
- Testing strategy: Phase 0 establishes test infrastructure before any feature work
- Feature flag isolation: All features behind `#[cfg(feature = "X")]` with default-off

---

## Work Objectives

### Core Objective
Port OMO's most impactful architectural patterns into JFC as idiomatic Rust, prioritized by daily-use improvement first, new capabilities second, advanced orchestration third.

### Concrete Deliverables
- `crates/jfc-ui/src/hashline.rs` — content-hash-anchored edit resolution
- `crates/jfc-ui/src/permissions.rs` — TOML-driven permission automation
- `crates/jfc-ui/src/hooks.rs` — lifecycle hook system (8 hook points)
- `crates/jfc-ui/src/background.rs` — background agent manager
- `crates/jfc-ui/src/intent.rs` — heuristic intent classification
- `crates/jfc-ui/src/sandbox/landlock.rs` — Linux process sandbox
- Agent profile configs for Argus review and Ralph loop patterns

### Definition of Done
- [ ] `cargo test --all-features` passes
- [ ] `cargo clippy --all-features -- -D warnings` clean
- [ ] Each feature independently toggleable via feature flag
- [ ] No `unsafe` blocks written in JFC code (crate dependencies may use unsafe internally)
- [ ] No new binary crates (dev-dependencies and feature-gated optional deps allowed)
- [ ] Existing tool JSON schema unchanged for current variants (only additive Tmux variant added)

### Must Have
- Hashline edit resolution with fallback to fuzzy match
- Permission rules loaded from TOML config
- Hook points fire deterministically with <1ms p99 overhead
- Background agents crash-isolated from main TUI
- Intent classification in <5ms (heuristic only)
- All features behind feature flags (default off)

### Must NOT Have (Guardrails)
- ❌ NO scripting runtime (Lua, Rhai, WASM) — TOML + Rust enums only
- ❌ NO trait objects for hot-path hooks — enum dispatch or compile-time generics
- ❌ NO more than 3 new files per feature
- ❌ NO breaking changes to existing tool variant signatures (new additive variants like Tmux ARE allowed; existing Edit/Write/Read/Bash variant shapes must not change)
- ❌ NO port of all 52 OMO hooks (max 12 hook points)
- ❌ NO port of OMO agent personalities (only infrastructure, not system prompts)
- ❌ NO LLM round-trip for intent classification
- ❌ NO `unsafe` blocks in JFC code (dependencies like `landlock`/`seccompiler` crates handle their own unsafe internally)
- ❌ NO new binary crates (keep in existing crate structure; new dev-dependencies like `criterion`, `proptest`, and feature-gated optional deps like `landlock`, `seccompiler` ARE allowed)
- ❌ NO background agent nesting beyond depth 2

---

## Verification Strategy

> **ZERO HUMAN INTERVENTION** — ALL verification is agent-executed.

### Test Decision
- **Infrastructure exists**: YES (cargo test)
- **Automated tests**: TDD
- **Framework**: `cargo test -p jfc-ui --all-features`

### QA Policy
- **Hook latency**: `criterion` benchmark proving <1ms p99
- **Permission parsing**: Fuzz target (arbitrary TOML → no panics)
- **Background agents**: Stress test (spawn 20 agents, verify no deadlocks)
- **Hashline**: Property tests (100 edits to mutating file, measure success rate)
- **Sandbox**: Integration test verifying path escape prevention

---

## Execution Strategy

### Parallel Execution Waves

```
Phase 0 (Foundation — test infrastructure + feature flags, 3 tasks):
├── Task 1: Feature flag scaffolding + Cargo.toml features
├── Task 2: Test infrastructure (property test harness, benchmark setup)
└── Task 3: Configuration system (TOML loader with hot-reload pattern)

Phase 1 (Daily-Use Improvements — Hashline + Permissions, 7 parallel tasks):
├── Task 4: Hashline content-hash computation + line ID resolution
├── Task 5: Hashline fuzzy-match fallback + cache invalidation
├── Task 6: Hashline Edit tool integration (wrap existing Edit)
├── Task 7: Permission rule parser (TOML → RuleSet)
├── Task 8: Permission matcher (tool name + path glob matching)
├── Task 9: Permission integration with tool dispatch
└── Task 10: Permission escalation ceiling (deny-list overrides auto-approve)

Phase 2 (Extensibility — Hooks + Intent, 6 parallel tasks):
├── Task 11: Hook enum definition (8 hook points)
├── Task 12: Hook registry + deterministic ordering
├── Task 13: Hook integration with tool dispatch pipeline
├── Task 14: Intent classification heuristic engine
├── Task 15: Intent → tool availability mapping
└── Task 16: Hook + Intent wiring (intent informs hook behavior)

Phase 3 (New Capabilities — Background Agents + Sandbox, 6 parallel tasks):
├── Task 17: Background agent manager (spawn, track, collect)
├── Task 18: Background agent session isolation
├── Task 19: Background agent TUI output routing (collapsed panel)
├── Task 20: Landlock sandbox policy builder
├── Task 21: Seccomp filter for economy solver processes
└── Task 22: Sandbox integration with economy bounty spawning

Phase 4 (Advanced Orchestration — Review + Loop + Tmux, 6 tasks):
├── Task 23: Argus code review agent profile + tool selection
├── Task 24: Ralph continuation loop (detect incomplete → retry)
├── Task 25: Tmux interactive tool (send-keys, capture-pane)
├── Task 26: Comment checking hook (warn on AI-slop patterns)
├── Task 27: Session handoff protocol (context summary for new session)
└── Task 28: End-to-end integration test (full orchestration cycle)

Wave FINAL (Review):
├── F1: Plan compliance audit (oracle)
├── F2: Code quality review (general)
├── F3: Real QA (general)
└── F4: Scope fidelity check (general)
```

### Dependency Matrix

| Task | Depends On | Blocks |
|------|-----------|--------|
| 1-3 | None | 4-28 |
| 4-6 | 1, 2 | 28 |
| 7-10 | 1, 3 | 13 |
| 11-13 | 1, 2 | 16, 23-26 |
| 14-16 | 1, 11 | 17 |
| 17-19 | 1, 2, 11 | 22, 24 |
| 20-22 | 1, 17 | 28 |
| 23-28 | 11, 17 | F1-F4 |

### Agent Dispatch Summary

- **Phase 0**: 3 tasks → `general`
- **Phase 1**: 7 tasks → `general` (T4-T6 Hashline), `general` (T7-T10 Permissions)
- **Phase 2**: 6 tasks → `general`
- **Phase 3**: 6 tasks → `general` (T17-T19), `general` (T20-T22 sandbox)
- **Phase 4**: 6 tasks → `general` (T23-T25), `general` (T26-T28)
- **FINAL**: 4 tasks → `oracle` (F1), `general` (F2-F4)

---

## TODOs

- [x] 1. Feature Flag Scaffolding

  **What to do**:
  - Add feature flags to `crates/jfc-ui/Cargo.toml`: `hashline`, `permission-automation`, `hooks`, `background-agents`, `intent-gate`, `landlock-sandbox`
  - All default to OFF (`default = []`)
  - Add `#[cfg(feature = "X")]` module declarations in relevant `mod.rs` files
  - Verify `cargo build` passes with no features and with `--all-features`

  **Must NOT do**:
  - Do not add new crates
  - Do not add runtime dependencies that aren't gated behind features

  **Recommended Agent Profile**:
  - **Category**: `general`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Task 2, 3)
  - **Parallel Group**: Wave 0 (Phase 0)
  - **Blocks**: All subsequent tasks
  - **Blocked By**: None

  **References**:
  - `crates/jfc-ui/Cargo.toml` — existing dependency structure
  - Pattern for conditional module inclusion: `#[cfg(feature = "X")] pub mod X;` in the relevant `mod.rs` file (no existing example; this is the first use of feature-gated modules in jfc-ui)

  **Acceptance Criteria**:
  - [ ] `cargo build -p jfc-ui` passes (no features)
  - [ ] `cargo build -p jfc-ui --all-features` passes
  - [ ] Each feature flag is independently toggleable

  **QA Scenarios**:
  ```
  Scenario: Feature flags compile independently
    Tool: Bash
    Steps:
      1. cargo build -p jfc-ui --features hashline
      2. cargo build -p jfc-ui --features permission-automation
      3. cargo build -p jfc-ui --features hooks
      4. cargo build -p jfc-ui --features background-agents
      5. cargo build -p jfc-ui --all-features
    Expected: All 5 build commands pass with zero errors
    Evidence: .sisyphus/evidence/task-1-feature-flags.txt
  ```

  **Commit**: YES (groups with 2, 3)
  - Message: `feat(core): add feature flag scaffolding, test infra, config system`

- [x] 2. Test Infrastructure

  **What to do**:
  - Add `criterion` as a dev-dependency for benchmarks
  - Add `proptest` as a dev-dependency for property testing
  - Create `benches/hooks.rs` benchmark skeleton (empty, compiles)
  - Create `tests/integration/mod.rs` skeleton for cross-feature tests
  - Add `[[bench]]` entries to Cargo.toml

  **Must NOT do**:
  - Do not write actual tests yet (that's per-feature)
  - Do not add `loom` yet (only if concurrency issues emerge)

  **Recommended Agent Profile**:
  - **Category**: `general`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Task 1, 3)
  - **Parallel Group**: Wave 0
  - **Blocks**: Tasks 4-28 (all features need test infra)
  - **Blocked By**: None

  **References**:
  - `crates/jfc-ui/Cargo.toml` — add dev-dependencies section
  - Criterion docs: benchmark harness setup

  **Acceptance Criteria**:
  - [ ] `cargo bench -p jfc-ui --all-features` compiles (even if no benchmarks run yet)
  - [ ] `cargo test -p jfc-ui --all-features` passes

  **QA Scenarios**:
  ```
  Scenario: Benchmark infrastructure compiles
    Tool: Bash
    Steps:
      1. cargo bench -p jfc-ui --all-features --no-run
    Expected: Compiles without errors
    Evidence: .sisyphus/evidence/task-2-bench-infra.txt
  ```

  **Commit**: YES (groups with 1, 3)

- [x] 3. Configuration System (TOML Loader)

  **What to do**:
  - Extend the existing `crates/jfc-ui/src/config.rs` module (NOT create a new file — it already exists and is used by `main.rs` and `input.rs`)
  - Add new struct `FeatureConfig` containing feature-specific subsections: `permissions: PermissionsConfig`, `hooks: HooksConfig`, `intent: IntentConfig`, `background: BackgroundConfig`
  - Load feature config from `.jfc/features.toml` (separate from existing `config.rs` which handles UI/keybinding config)
  - Hot-reload pattern: re-read config at session start and before each stream call. JFC already reloads `CLAUDE.md` in a similar pattern — look at the existing reload logic in the stream module and follow the same approach.
  - Sections: `[permissions]`, `[hooks]`, `[intent]`, `[background]`
  - Parse errors produce clear tracing warnings, never panic

  **Must NOT do**:
  - Do not implement the actual permission/hook logic here (just the loader)
  - Do not add a file watcher (hot-reload via re-read is sufficient)

  **Recommended Agent Profile**:
  - **Category**: `general`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Task 1, 2)
  - **Parallel Group**: Wave 0
  - **Blocks**: Tasks 7-10 (permissions need config), Tasks 11-13 (hooks need config)
  - **Blocked By**: None

  **References**:
  - `crates/jfc-ui/src/config.rs` — EXISTING file, extend with `FeatureConfig` struct
  - `crates/jfc-ui/src/main.rs` — imports from config (verify existing usage not broken)
  - `toml` crate — existing dependency in workspace

  **Acceptance Criteria**:
  - [ ] FeatureConfig loads from `.jfc/features.toml` when present
  - [ ] Missing features.toml file → empty defaults (no error)
  - [ ] Malformed TOML → tracing::warn, returns defaults (no panic)
  - [ ] Existing config.rs functionality unchanged (no regression)

  **QA Scenarios**:
  ```
  Scenario: Feature config loads and handles missing file gracefully
    Tool: Bash
    Steps:
      1. Remove .jfc/features.toml if exists
      2. Call FeatureConfig::load() → returns defaults
      3. Write invalid TOML to .jfc/features.toml
      4. Call FeatureConfig::load() → returns defaults + logged warning
      5. cargo test -p jfc-ui --features permission-automation -- test_feature_config
    Expected: All assertions pass, no panic, existing config unaffected
    Evidence: .sisyphus/evidence/task-3-config.txt
  ```

  **Commit**: YES (groups with 1, 2)

- [x] 4. Hashline Content-Hash Computation + Line ID Resolution

  **What to do**:
  - Create `crates/jfc-ui/src/hashline.rs` (behind `hashline` feature)
  - Implement `LineId` = first 8 chars of SHA-256 of trimmed line content
  - `FileIndex` struct: maps `LineId → Vec<usize>` (line numbers, handles duplicates)
  - `resolve_line(file_content: &str, line_id: &str, hint_line: usize) -> Option<usize>`
  - Resolution priority: (1) exact line number if hash matches, (2) nearest line with matching hash, (3) None
  - Cache: `HashMap<PathBuf, (SystemTime, FileIndex)>` invalidated on mtime change

  **Must NOT do**:
  - Do not integrate with Edit tool yet (Task 6)
  - Do not implement fuzzy matching (Task 5)
  - Do not add undo/redo/history

  **Recommended Agent Profile**:
  - **Category**: `general`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 5, 7-10)
  - **Parallel Group**: Phase 1
  - **Blocks**: Task 5 (fuzzy fallback), Task 6 (Edit integration)
  - **Blocked By**: Task 1 (feature flags)

  **References**:
  - OMO Hashline concept: content-hash-anchored line addressing
  - `crates/jfc-ui/Cargo.toml:48` — `sha2 = "0.10"` already a direct dependency

  **Acceptance Criteria**:
  - [ ] `LineId` computation is deterministic (same content → same hash)
  - [ ] Resolution finds correct line after insertions/deletions above target
  - [ ] Cache invalidation works on file modification

  **QA Scenarios**:
  ```
  Scenario: Line resolution survives insertions above target
    Tool: Bash
    Steps:
      1. Create file with 10 lines, compute LineId for line 5
      2. Insert 3 lines above line 5 (now at line 8)
      3. resolve_line(new_content, line_id, hint=5) → returns 8
      4. cargo test -p jfc-ui --features hashline -- test_resolve_after_insertion
    Expected: Correctly resolves to new position
    Evidence: .sisyphus/evidence/task-4-hashline-resolve.txt
  ```

  **Commit**: YES (groups with 5, 6)
  - Message: `feat(edit): hashline content-anchored edit resolution`

- [x] 5. Hashline Fuzzy-Match Fallback + Cache Invalidation

  **What to do**:
  - Add fuzzy match: if exact hash not found, find line with minimum Levenshtein distance (threshold: 0.8 similarity)
  - Return `Resolution { line: usize, confidence: f32, method: Exact|Fuzzy|Failed }`
  - Add `verify_before_apply`: re-hash line at resolved position, confirm match before edit
  - Property test: apply 100 random edits to file, measure resolution success rate

  **Must NOT do**:
  - Do not add external fuzzy-matching crates (use simple Levenshtein inline)
  - Do not implement multi-edit transactions

  **Recommended Agent Profile**:
  - **Category**: `general`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 7-10)
  - **Parallel Group**: Phase 1
  - **Blocks**: Task 6 (Edit integration)
  - **Blocked By**: Task 4

  **References**:
  - `crates/jfc-ui/src/hashline.rs` — extends Task 4's implementation
  - Levenshtein distance algorithm (inline implementation, ~20 LOC)

  **Acceptance Criteria**:
  - [ ] Fuzzy match finds line with minor edits (whitespace, typo)
  - [ ] Confidence score accurately reflects match quality
  - [ ] Property test shows >80% resolution success on mutating files

  **QA Scenarios**:
  ```
  Scenario: Fuzzy match handles minor edits
    Tool: Bash
    Steps:
      1. Original line: "fn process_data(input: &str) -> Result<(), Error>"
      2. Modified line: "fn process_data(input: &str) -> Result<String, Error>"
      3. Resolve with original hash → finds modified line via fuzzy match
      4. cargo test -p jfc-ui --features hashline -- test_fuzzy_resolution
    Expected: Resolution succeeds with confidence > 0.8
    Evidence: .sisyphus/evidence/task-5-fuzzy.txt
  ```

  **Commit**: YES (groups with 4, 6)

- [x] 6. Hashline Edit Tool Integration

  **What to do**:
  - Wrap existing Edit handling: when Hashline is enabled, before applying an edit via exact `old_string` match:
    1. If exact match is unique → use it (current behavior, no change)
    2. If exact match is AMBIGUOUS (multiple occurrences) → use Hashline position hint from internal cache to disambiguate
    3. If exact match FAILS (content drifted) → use Hashline fuzzy resolution to find the intended target
  - **Metadata source (non-breaking)**: When Read/Glob tools display file content, the Hashline module silently builds/caches a `FileIndex` (mapping line hashes to positions). The `old_string` from the Edit call is hashed and looked up in this cache. The "hint line" is inferred from the LAST Read output that contained the old_string (the line number in the Read response). This requires NO schema change — all metadata is internal.
  - If Hashline resolves successfully (confidence > 0.9), use resolved position
  - If Hashline fails, fall back to current exact-string-match behavior (no regression)
  - Log resolution method and confidence for observability
  - Add `hashline_enabled: bool` to StreamOptions (from config)

  **Must NOT do**:
  - Do not change the Edit tool's JSON schema (no new fields in tool_use)
  - Do not require the model to pass Hashline metadata (it's invisible to the model)
  - Do not remove existing exact-match behavior
  - Do not make Hashline mandatory (always fallback available)

  **Recommended Agent Profile**:
  - **Category**: `general`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: NO (depends on 4, 5)
  - **Parallel Group**: Phase 1 (after 4+5)
  - **Blocks**: Task 28
  - **Blocked By**: Tasks 4, 5

  **References**:
  - `crates/jfc-ui/src/tools.rs` — existing Edit tool dispatch (match on ToolInput variants, particularly Edit/Write)
  - `crates/jfc-ui/src/hashline.rs` — from Tasks 4, 5

  **Acceptance Criteria**:
  - [ ] Edit with Hashline addressing succeeds where line-number-only would fail
  - [ ] Fallback to exact match works when Hashline disabled or fails
  - [ ] No regression in existing edit behavior

  **QA Scenarios**:
  ```
  Scenario: Hashline resolves when duplicate lines exist
    Tool: Bash
    Steps:
      1. Create file with 3 identical lines: "let x = 42;" at lines 3, 7, 11
      2. Compute Hashline for line 7 (includes position hint)
      3. Insert 2 lines above (target moves to line 9)
      4. Standard old_string match is ambiguous (3 matches → fails)
      5. Hashline resolves to line 9 (nearest to hint_line=7 with matching hash)
      6. cargo test -p jfc-ui --features hashline -- test_edit_disambiguation
    Expected: Hashline disambiguates duplicate content, plain old_string cannot
    Evidence: .sisyphus/evidence/task-6-edit-integration.txt

  Scenario: Hashline resolves fuzzy after minor target content change
    Tool: Bash
    Steps:
      1. Create file with target: "fn calculate_total(items: &[Item]) -> u64 {"
      2. Compute Hashline for that target
      3. Change target to: "fn calculate_total(items: &[Item]) -> u128 {"
      4. Standard old_string match fails (content changed)
      5. Hashline fuzzy resolution finds it (0.95 similarity)
      6. cargo test -p jfc-ui --features hashline -- test_edit_fuzzy_content_change
    Expected: Hashline resolves via fuzzy, exact-match fails
    Evidence: .sisyphus/evidence/task-6-edit-fuzzy.txt
  ```

  **Commit**: YES (groups with 4, 5)

- [x] 7. Permission Rule Parser (TOML → RuleSet)

  **What to do**:
  - Create `crates/jfc-ui/src/permissions.rs` (behind `permission-automation` feature)
  - Define `PermissionRule`: `{ action: Allow|Deny, tool: GlobPattern, path: Option<GlobPattern>, reason: String }`
  - Define `RuleSet`: ordered Vec of rules (first match wins)
  - Parse from `[permissions]` section of `.jfc/features.toml`
  - Support glob patterns: `Bash:*`, `Edit:src/**/*.rs`, `*:*` (wildcard)

  **Must NOT do**:
  - Do not implement the matching logic (Task 8)
  - Do not integrate with tool dispatch (Task 9)

  **Recommended Agent Profile**:
  - **Category**: `general`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 4-6, 8-10)
  - **Parallel Group**: Phase 1
  - **Blocks**: Task 8 (matcher)
  - **Blocked By**: Task 3 (config system)

  **References**:
  - OMO `src/features/permission-automation/rule-matcher.ts` — glob-based rules
  - `glob` crate pattern syntax

  **Acceptance Criteria**:
  - [ ] Parses valid TOML permission rules
  - [ ] Rejects malformed rules with clear error message
  - [ ] Glob patterns compile correctly

  **QA Scenarios**:
  ```
  Scenario: Permission rules parse from TOML
    Tool: Bash
    Steps:
      1. Write TOML with: allow Bash:*, deny Edit:../*, allow Edit:src/**
      2. Parse into RuleSet
      3. Assert 3 rules in correct order
      4. cargo test -p jfc-ui --features permission-automation -- test_rule_parse
    Expected: Rules parsed, globs compiled, order preserved
    Evidence: .sisyphus/evidence/task-7-rule-parse.txt
  ```

  **Commit**: YES (groups with 8, 9, 10)
  - Message: `feat(permissions): TOML-driven permission automation`

- [x] 8. Permission Matcher (Tool + Path Glob Matching)

  **What to do**:
  - Implement `RuleSet::evaluate(tool_name: &str, path: Option<&str>) -> PermissionDecision`
  - `PermissionDecision`: `{ action: Allow|Deny|Ask, rule: Option<&PermissionRule> }`
  - First matching rule wins; if no rules match → `Ask` (fail-open to current behavior)
  - Deny rules always override allow rules at same specificity

  **Must NOT do**:
  - Do not integrate with actual tool dispatch (Task 9)
  - Do not add role-based access control

  **Recommended Agent Profile**:
  - **Category**: `general`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 4-6)
  - **Parallel Group**: Phase 1
  - **Blocks**: Task 9
  - **Blocked By**: Task 7

  **References**:
  - `crates/jfc-ui/src/permissions.rs` — from Task 7
  - OMO rule-matcher.ts — precedence logic

  **Acceptance Criteria**:
  - [ ] Deny rule blocks matching tool
  - [ ] Allow rule permits matching tool
  - [ ] No matching rule → Ask (current default behavior)
  - [ ] Deny overrides Allow at same specificity

  **QA Scenarios**:
  ```
  Scenario: Deny overrides allow for same path
    Tool: Bash
    Steps:
      1. Rules: [allow Edit:src/**, deny Edit:src/secrets/**]
      2. Evaluate Edit with path="src/secrets/key.rs"
      3. Assert → Deny
      4. Evaluate Edit with path="src/lib.rs"
      5. Assert → Allow
      6. cargo test -p jfc-ui --features permission-automation -- test_deny_override
    Expected: Deny wins for secrets, allow wins for src
    Evidence: .sisyphus/evidence/task-8-matcher.txt
  ```

  **Commit**: YES (groups with 7, 9, 10)

- [x] 9. Permission Integration with Tool Dispatch

  **What to do**:
  - In `dispatch_tools_batched` (or equivalent): before executing each tool, call `RuleSet::evaluate`
  - If Allow → execute without prompting user
  - If Deny → return error result without executing
  - If Ask → fall through to existing permission prompt behavior
  - Log every permission decision at INFO level

  **Must NOT do**:
  - Do not modify the existing user-prompt flow for Ask decisions
  - Do not add caching (rules re-evaluated per invocation for hot-reload)

  **Recommended Agent Profile**:
  - **Category**: `general`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: NO (depends on 7, 8)
  - **Parallel Group**: Phase 1 (after 7+8)
  - **Blocks**: Task 10
  - **Blocked By**: Tasks 7, 8

  **References**:
  - `crates/jfc-ui/src/tools.rs` — tool dispatch entry point (execute_tool function and ToolInput enum match)
  - `crates/jfc-ui/src/permissions.rs` — from Tasks 7, 8
  - `crates/jfc-ui/src/app.rs` — existing `auto_approves()` and `tool_needs_approval()` methods that handle current permission logic

  **Acceptance Criteria**:
  - [ ] Denied tools never execute
  - [ ] Allowed tools skip user prompt
  - [ ] Ask tools behave exactly as before (no regression)

  **QA Scenarios**:
  ```
  Scenario: Denied tool returns error without executing
    Tool: Bash
    Steps:
      1. Configure deny rule for Bash:rm*
      2. Dispatch Bash tool with command "rm -rf /"
      3. Assert tool was NOT executed
      4. Assert error result contains "denied by permission rule"
      5. cargo test -p jfc-ui --features permission-automation -- test_deny_blocks
    Expected: Tool blocked, clear error message
    Evidence: .sisyphus/evidence/task-9-dispatch.txt
  ```

  **Commit**: YES (groups with 7, 8, 10)

- [x] 10. Permission Escalation Ceiling

  **What to do**:
  - Add `[permissions.ceiling]` config section: tools that are ALWAYS denied regardless of other rules
  - Default ceiling: `["Bash:rm -rf *", "Bash:dd *", "Bash:mkfs *"]` (destructive commands)
  - Ceiling rules checked FIRST, before any allow/deny rules
  - Even swarm leader auto-approve cannot override ceiling

  **Must NOT do**:
  - Do not make ceiling configurable to "off" (always active as safety net)
  - Do not add complex regex (glob is sufficient)

  **Recommended Agent Profile**:
  - **Category**: `general`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: NO (depends on 9)
  - **Parallel Group**: Phase 1 (after 9)
  - **Blocks**: None
  - **Blocked By**: Task 9

  **References**:
  - `crates/jfc-ui/src/permissions.rs` — extends evaluation
  - Metis finding: "escalation ceiling even in auto-mode"

  **Acceptance Criteria**:
  - [ ] Ceiling rules cannot be overridden by allow rules
  - [ ] Ceiling active even in auto-approve mode
  - [ ] Default destructive commands are blocked

  **QA Scenarios**:
  ```
  Scenario: Ceiling blocks even when allow-all configured
    Tool: Bash
    Steps:
      1. Configure: allow *:* (allow everything)
      2. Ceiling: deny Bash:rm -rf*
      3. Evaluate Bash with "rm -rf /"
      4. Assert → Deny (ceiling wins)
      5. cargo test -p jfc-ui --features permission-automation -- test_ceiling
    Expected: Ceiling overrides allow-all
    Evidence: .sisyphus/evidence/task-10-ceiling.txt
  ```

  **Commit**: YES (groups with 7, 8, 9)

- [x] 11. Hook Enum Definition (8 Hook Points)

  **What to do**:
  - Create `crates/jfc-ui/src/hooks.rs` (behind `hooks` feature)
  - Define `HookPoint` enum: `BeforeToolDispatch`, `AfterToolDispatch`, `BeforeStream`, `AfterStream`, `OnError`, `OnToolApproval`, `BeforeCommit`, `OnSessionStart`
  - Define `HookAction` enum: `Continue`, `Skip`, `Replace(ToolInput)`, `Abort(String)`
  - Define `HookHandler` enum (NOT trait): each variant is a concrete handler (e.g. `PermissionCheck`, `IntentEnricher`, `CommentChecker`, `CustomFn(fn(&HookContext) -> HookAction)`)
  - Dispatch via `match` on `HookHandler` variant — zero dynamic dispatch, no `Box<dyn>`
  - HookContext contains: tool name, tool input, session state, config

  **Must NOT do**:
  - Do not use `dyn Hook` or `Box<dyn HookHandler>` — enum dispatch only
  - Do not add more than 8 hook points
  - Do not add hook ordering priorities (FIFO is fine)

  **Recommended Agent Profile**:
  - **Category**: `general`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 12-16)
  - **Parallel Group**: Phase 2
  - **Blocks**: Tasks 12, 13, 16
  - **Blocked By**: Task 1

  **References**:
  - OMO hook tiers: Session → Tool-Guard → Transform → Continuation → Skill
  - JFC simplified to 8 concrete points (no middleware chains)

  **Acceptance Criteria**:
  - [ ] All 8 hook points defined as enum variants
  - [ ] HookAction enum covers Continue/Skip/Replace/Abort
  - [ ] Types compile under feature flag

  **QA Scenarios**:
  ```
  Scenario: Hook types compile and are exhaustive
    Tool: Bash
    Steps:
      1. cargo build -p jfc-ui --features hooks
      2. cargo test -p jfc-ui --features hooks -- test_hook_enum_variants
    Expected: 8 variants, match is exhaustive
    Evidence: .sisyphus/evidence/task-11-hook-types.txt
  ```

  **Commit**: YES (groups with 12-16)
  - Message: `feat(hooks): lifecycle hook system + intent classification`

- [x] 12. Hook Registry + Deterministic Ordering

  **What to do**:
  - `HookRegistry` struct: `Vec<(HookPoint, HookHandler)>` (built at startup, not hot-modified)
  - Registration order = execution order (FIFO, deterministic)
  - `fn fire(&self, point: HookPoint, ctx: &mut HookContext) -> HookAction` — iterates Vec, matches each HookHandler via enum dispatch
  - Short-circuit: first `Skip` or `Abort` stops further hooks at that point
  - Benchmark: fire 100 no-op hooks in <100μs

  **Recommended Agent Profile**:
  - **Category**: `general`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 14-16)
  - **Parallel Group**: Phase 2
  - **Blocks**: Task 13
  - **Blocked By**: Task 11

  **References**:
  - `crates/jfc-ui/src/hooks.rs` — extends Task 11

  **Acceptance Criteria**:
  - [ ] Hooks fire in registration order
  - [ ] Short-circuit on Skip/Abort works
  - [ ] Benchmark: 100 no-op hooks < 100μs

  **QA Scenarios**:
  ```
  Scenario: Hooks fire in order and short-circuit
    Tool: Bash
    Steps:
      1. Register hooks: [log, allow, deny]
      2. Fire BeforeToolDispatch
      3. Assert deny stops execution (log and allow ran, deny aborted)
      4. cargo test -p jfc-ui --features hooks -- test_hook_ordering
    Expected: First two hooks fire, third aborts
    Evidence: .sisyphus/evidence/task-12-hook-registry.txt
  ```

  **Commit**: YES (groups with 11, 13-16)

- [x] 13. Hook Integration with Tool Dispatch Pipeline

  **What to do**:
  - In tool dispatch: fire `BeforeToolDispatch` before execution
  - If hook returns `Skip` → skip tool, return empty result
  - If hook returns `Replace(new_input)` → execute new_input instead
  - If hook returns `Abort(msg)` → return error with message
  - Fire `AfterToolDispatch` after execution (informational, cannot modify result)

  **Recommended Agent Profile**:
  - **Category**: `general`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: NO (depends on 11, 12)
  - **Parallel Group**: Phase 2 (after 11+12)
  - **Blocks**: Tasks 23-26
  - **Blocked By**: Tasks 11, 12

  **References**:
  - `crates/jfc-ui/src/tools.rs` — tool dispatch entry
  - `crates/jfc-ui/src/hooks.rs` — from Tasks 11, 12

  **Acceptance Criteria**:
  - [ ] BeforeToolDispatch can block tool execution
  - [ ] AfterToolDispatch fires after every tool
  - [ ] Hook overhead < 1ms p99 (benchmark)

  **QA Scenarios**:
  ```
  Scenario: Hook blocks dangerous tool
    Tool: Bash
    Steps:
      1. Register BeforeToolDispatch hook that denies Bash commands with "rm"
      2. Dispatch Bash tool with "rm -rf /"
      3. Assert tool NOT executed, error message returned
      4. cargo test -p jfc-ui --features hooks -- test_hook_blocks_tool
    Expected: Tool blocked by hook
    Evidence: .sisyphus/evidence/task-13-hook-dispatch.txt
  ```

  **Commit**: YES (groups with 11, 12, 14-16)

- [x] 14. Intent Classification Heuristic Engine

  **What to do**:
  - Create `crates/jfc-ui/src/intent.rs` (behind `intent-gate` feature)
  - Define `Intent` enum: `Research`, `Implementation`, `Investigation`, `Fix`, `Evaluation`, `Chat`
  - Classify from message text using keyword/pattern heuristics (no LLM call)
  - Patterns: "find", "search", "where" → Research; "create", "add", "implement" → Implementation; etc.
  - Return `(Intent, f32)` confidence; threshold 0.6 for classification

  **Must NOT do**:
  - Do not call LLM for classification (must be <5ms)
  - Do not add NLP libraries (simple keyword matching)

  **Recommended Agent Profile**:
  - **Category**: `general`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 11-13, 15-16)
  - **Parallel Group**: Phase 2
  - **Blocks**: Task 15
  - **Blocked By**: Task 1

  **References**:
  - OMO IntentGate — heuristic classification
  - Simple keyword weighting approach

  **Acceptance Criteria**:
  - [ ] Classification completes in <5ms
  - [ ] "find the function that handles auth" → Research
  - [ ] "add a login page" → Implementation
  - [ ] Ambiguous text → Chat (default)

  **QA Scenarios**:
  ```
  Scenario: Intent classified correctly
    Tool: Bash
    Steps:
      1. Classify "search for all usages of foo" → Research
      2. Classify "implement dark mode toggle" → Implementation
      3. Classify "fix the bug in login" → Fix
      4. Classify "hello how are you" → Chat
      5. cargo test -p jfc-ui --features intent-gate -- test_intent_classify
    Expected: All 4 correct, each < 1ms
    Evidence: .sisyphus/evidence/task-14-intent.txt
  ```

  **Commit**: YES (groups with 11-13, 15-16)

- [x] 15. Intent → Tool Availability Mapping

  **What to do**:
  - Map Intent to suggested tool subset (advisory, not enforcing):
    - Research → prefer: Grep, Read, Glob; discourage: Edit, Write
    - Implementation → prefer: Edit, Write, Bash; all available
    - Fix → prefer: Edit, Bash, LSP; all available
    - Investigation → prefer: Read, Grep, LSP; discourage: Write
  - Add `suggested_tools(intent: Intent) -> Vec<ToolKind>` for system prompt enrichment
  - Intent shown in TUI status bar when classified

  **Recommended Agent Profile**:
  - **Category**: `general`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 11-13)
  - **Parallel Group**: Phase 2
  - **Blocks**: Task 16
  - **Blocked By**: Task 14

  **References**:
  - `crates/jfc-ui/src/intent.rs` — extends Task 14
  - OMO tool availability per agent profile

  **Acceptance Criteria**:
  - [ ] Mapping returns correct tools per intent
  - [ ] Advisory only (never blocks tool use)

  **QA Scenarios**:
  ```
  Scenario: Research intent suggests read-only tools
    Tool: Bash
    Steps:
      1. suggested_tools(Research) → contains Grep, Read, Glob
      2. suggested_tools(Research) → does NOT contain Edit, Write
      3. cargo test -p jfc-ui --features intent-gate -- test_intent_tools
    Expected: Correct tool suggestions
    Evidence: .sisyphus/evidence/task-15-intent-tools.txt
  ```

  **Commit**: YES (groups with 11-14, 16)

- [x] 16. Hook + Intent Wiring

  **What to do**:
  - Register built-in hook: `OnSessionStart` → classify intent, store in session state
  - Register built-in hook: `BeforeStream` → inject intent-appropriate guidance into system prompt
  - Intent stored in `HookContext` for other hooks to read
  - Config-driven: `[intent] enabled = true` in `.jfc/features.toml`

  **Recommended Agent Profile**:
  - **Category**: `general`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: NO (depends on 11-15)
  - **Parallel Group**: Phase 2 (after 14+15)
  - **Blocks**: None in Phase 2
  - **Blocked By**: Tasks 11, 14, 15

  **References**:
  - `crates/jfc-ui/src/hooks.rs` and `intent.rs`

  **Acceptance Criteria**:
  - [ ] Intent classification runs on session start
  - [ ] System prompt enriched with intent guidance
  - [ ] Can be disabled via config

  **QA Scenarios**:
  ```
  Scenario: Intent enriches system prompt
    Tool: Bash
    Steps:
      1. User message: "find where auth is handled"
      2. Intent classified as Research
      3. System prompt includes "focus on read-only exploration"
      4. cargo test -p jfc-ui --all-features -- test_intent_prompt
    Expected: Prompt enrichment matches intent
    Evidence: .sisyphus/evidence/task-16-intent-wiring.txt
  ```

  **Commit**: YES (groups with 11-15)

- [x] 17. Background Agent Manager (Spawn, Track, Collect)

  **What to do**:
  - Create `crates/jfc-ui/src/background.rs` (behind `background-agents` feature)
  - `BackgroundManager` struct: spawns tokio tasks, tracks by ID, collects results
  - `spawn(config: AgentConfig) -> AgentId` — creates isolated agent session
  - `status(id: AgentId) -> AgentStatus` (Running, Completed, Failed)
  - `collect(id: AgentId) -> Option<AgentResult>` — retrieves result
  - Max concurrent: configurable (default 5), reject spawn if at limit

  **Must NOT do**:
  - Do not implement agent nesting beyond depth 2
  - Do not share mutable state between agents (message passing only)
  - Do not add agent-to-agent direct communication

  **Recommended Agent Profile**:
  - **Category**: `general`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 18-22)
  - **Parallel Group**: Phase 3
  - **Blocks**: Tasks 18, 19, 22, 24
  - **Blocked By**: Tasks 1, 2, 11

  **References**:
  - OMO `src/features/background-agent/manager.ts` — spawn/track/collect pattern
  - JFC existing swarm: `crates/jfc-ui/src/swarm/mod.rs`, `swarm/runner.rs`, `swarm/permission_sync.rs`, `swarm/types.rs` — team management and permission forwarding

  **Acceptance Criteria**:
  - [ ] Can spawn background agent
  - [ ] Agent runs isolated from main TUI
  - [ ] Results collectable after completion
  - [ ] Max concurrent limit enforced

  **QA Scenarios**:
  ```
  Scenario: Background agent spawns and completes
    Tool: Bash
    Steps:
      1. Spawn background agent with simple task
      2. Wait for completion
      3. Collect result
      4. cargo test -p jfc-ui --features background-agents -- test_spawn_collect
    Expected: Agent completes, result collected
    Evidence: .sisyphus/evidence/task-17-bg-spawn.txt

  Scenario: Max concurrent limit enforced
    Tool: Bash
    Steps:
      1. Set max_concurrent=2
      2. Spawn 3 agents
      3. Assert 3rd spawn returns Err(AtCapacity)
      4. cargo test -p jfc-ui --features background-agents -- test_max_concurrent
    Expected: Third spawn rejected
    Evidence: .sisyphus/evidence/task-17-bg-limit.txt
  ```

  **Commit**: YES (groups with 18-22)
  - Message: `feat(agents): background agent manager + landlock sandbox`

- [x] 18. Background Agent Session Isolation

  **What to do**:
  - Each background agent gets: own message history, own token counter, own tool permissions
  - NO shared filesystem state (each operates in own git worktree if writing files)
  - Result is: `AgentResult { messages: Vec<Message>, tokens_used: usize, artifacts: Vec<PathBuf> }`
  - Crash isolation: if agent panics, main TUI unaffected (catch_unwind + task boundary)

  **Recommended Agent Profile**:
  - **Category**: `general`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 19-22)
  - **Parallel Group**: Phase 3
  - **Blocks**: Task 22
  - **Blocked By**: Task 17

  **References**:
  - OMO spawner.ts — session isolation
  - JFC existing worktree: `crates/jfc-ui/src/worktrees.rs`
  - JFC existing session: `crates/jfc-ui/src/session.rs`

  **Acceptance Criteria**:
  - [ ] Agent crash does not crash main TUI
  - [ ] Token counting is per-agent (not shared)
  - [ ] Message history is isolated

  **QA Scenarios**:
  ```
  Scenario: Agent crash does not affect main
    Tool: Bash
    Steps:
      1. Spawn agent that panics immediately
      2. Assert main BackgroundManager still functional
      3. Status shows Failed for crashed agent
      4. cargo test -p jfc-ui --features background-agents -- test_crash_isolation
    Expected: Main unaffected, agent marked Failed
    Evidence: .sisyphus/evidence/task-18-isolation.txt
  ```

  **Commit**: YES (groups with 17, 19-22)

- [x] 19. Background Agent TUI Output Routing

  **What to do**:
  - Add collapsed panel in TUI showing active background agents
  - Each agent shows: ID, status (spinner/done/failed), elapsed time
  - Expand agent to see its output (last N messages)
  - Notification when agent completes (brief toast in status bar)

  **Recommended Agent Profile**:
  - **Category**: `visual-engineering`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 20-22)
  - **Parallel Group**: Phase 3
  - **Blocks**: None
  - **Blocked By**: Task 17

  **References**:
  - JFC existing TUI rendering: `crates/jfc-ui/src/render.rs`, `crates/jfc-ui/src/render_cache.rs`, `crates/jfc-ui/src/toast.rs`
  - OMO background agent UI patterns

  **Acceptance Criteria**:
  - [ ] Panel shows active agents with status
  - [ ] Expanding agent shows messages
  - [ ] Toast notification on completion

  **QA Scenarios**:
  ```
  Scenario: Background panel renders correctly
    Tool: interactive_bash (tmux)
    Steps:
      1. Start jfc with background-agents feature enabled
      2. Spawn a background agent
      3. Verify panel appears with agent status
      4. Wait for completion, verify toast appears
    Expected: Panel visible, status updates, toast fires
    Evidence: .sisyphus/evidence/task-19-tui-panel.png
  ```

  **Commit**: YES (groups with 17, 18, 20-22)

- [x] 20. Landlock Sandbox Policy Builder

  **What to do**:
  - Create `crates/jfc-ui/src/sandbox/landlock.rs` (behind `landlock-sandbox` feature, Linux only)
  - `SandboxPolicy` builder: allow read/write to specific directories, deny everything else
  - Apply to child process via `prctl` + Landlock ruleset
  - Graceful degradation: if kernel doesn't support Landlock, log warning and proceed unsandboxed
  - Test: spawn sandboxed process, verify it CANNOT read `/etc/passwd`

  **Must NOT do**:
  - Do not write `unsafe` blocks in JFC code (the `landlock` crate handles unsafe internally)
  - Do not implement for non-Linux (feature-gate to `#[cfg(target_os = "linux")]`)

  **Recommended Agent Profile**:
  - **Category**: `general`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 17-19, 21-22)
  - **Parallel Group**: Phase 3
  - **Blocks**: Task 22
  - **Blocked By**: Task 1

  **References**:
  - Codex-rs `core/src/landlock.rs` — Linux LSM sandbox policy
  - `landlock` crate: https://docs.rs/landlock/

  **Acceptance Criteria**:
  - [ ] Sandboxed process cannot access paths outside allowlist
  - [ ] Non-Landlock kernels gracefully degrade
  - [ ] No `unsafe` code (use crate bindings)

  **QA Scenarios**:
  ```
  Scenario: Sandboxed process cannot escape
    Tool: Bash
    Steps:
      1. Build sandbox allowing only /tmp/test-dir
      2. Spawn child process attempting to read /etc/passwd
      3. Assert child fails with permission denied
      4. cargo test -p jfc-ui --features landlock-sandbox -- test_landlock_deny
    Expected: Access denied for paths outside allowlist
    Evidence: .sisyphus/evidence/task-20-landlock.txt
  ```

  **Commit**: YES (groups with 17-19, 21-22)

- [x] 21. Seccomp Filter for Economy Solver Processes

  **What to do**:
  - Add seccomp filter that restricts: no network syscalls (socket, connect, bind), no process spawning beyond allowlist
  - Apply to economy solver child processes (bounty solvers)
  - Allowlist: file I/O within worktree, process execution for build commands only
  - Uses `seccompiler` or `libseccomp` crate bindings

  **Must NOT do**:
  - Do not apply seccomp to the main JFC process
  - Do not implement for non-Linux

  **Recommended Agent Profile**:
  - **Category**: `general`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 17-20, 22)
  - **Parallel Group**: Phase 3
  - **Blocks**: Task 22
  - **Blocked By**: Task 1

  **References**:
  - Codex-rs exec_policy.rs — execution restriction levels
  - `seccompiler` crate

  **Acceptance Criteria**:
  - [ ] Filtered process cannot make network connections
  - [ ] Filtered process CAN execute build commands (zig, cargo, npm)
  - [ ] Filter only applied to child processes, not main

  **QA Scenarios**:
  ```
  Scenario: Seccomp blocks network access
    Tool: Bash
    Steps:
      1. Spawn child with seccomp filter
      2. Child attempts to connect to 1.1.1.1:80
      3. Assert connection fails (EPERM or similar)
      4. cargo test -p jfc-ui --features landlock-sandbox -- test_seccomp_no_network
    Expected: Network syscall blocked
    Evidence: .sisyphus/evidence/task-21-seccomp.txt
  ```

  **Commit**: YES (groups with 17-20, 22)

- [x] 22. Sandbox Integration with Economy Bounty Spawning

  **What to do**:
  - When spawning economy solver agents, apply Landlock + seccomp policy
  - Solver worktree is the ONLY writable path
  - Build tools (zig, cargo, npm, go, python) are in exec allowlist
  - Network blocked except for LLM API endpoint (if solver makes its own calls)
  - Log sandbox application and any degradation

  **Recommended Agent Profile**:
  - **Category**: `general`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: NO (depends on 17, 20, 21)
  - **Parallel Group**: Phase 3 (after 20+21)
  - **Blocks**: Task 28
  - **Blocked By**: Tasks 17, 20, 21

  **References**:
  - `crates/jfc-ui/src/tools.rs` — `verify_bounty_solution` function (currently applies Landlock-like path restriction via `resolve_solution_file_path`)
  - `crates/jfc-ui/src/sandbox/landlock.rs` — from Task 20
  - `crates/jfc-ui/src/worktrees.rs` — worktree creation/management for economy solvers

  **Acceptance Criteria**:
  - [ ] Economy solvers run sandboxed
  - [ ] Solvers can build within worktree
  - [ ] Solvers cannot access files outside worktree
  - [ ] Graceful degradation on non-Linux

  **QA Scenarios**:
  ```
  Scenario: Economy solver sandboxed
    Tool: Bash
    Steps:
      1. Post bounty with sandbox enabled
      2. Solver attempts path traversal (../../../etc/passwd)
      3. Assert access denied
      4. Solver can write to worktree normally
      5. cargo test -p jfc-ui --all-features -- test_economy_sandbox
    Expected: Traversal blocked, normal writes succeed
    Evidence: .sisyphus/evidence/task-22-economy-sandbox.txt
  ```

  **Commit**: YES (groups with 17-21)

- [x] 23. Argus Code Review Agent Profile

  **What to do**:
  - Define agent profile config: system prompt, tool selection, review methodology
  - Profile stored as TOML in `.jfc/agents/argus.toml`
  - Tools available: Read, Grep, Glob, LSP (no Edit, no Write, no Bash)
  - Review methodology: P0-P3 priority, structured output format
  - Invokable via slash command `/review` or hook on `BeforeCommit`

  **Recommended Agent Profile**:
  - **Category**: `general`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 24-28)
  - **Parallel Group**: Phase 4
  - **Blocks**: None
  - **Blocked By**: Tasks 11, 17

  **References**:
  - OMO Argus security review agent
  - JFC existing slash commands

  **Acceptance Criteria**:
  - [ ] Agent profile loads from TOML
  - [ ] Only read-only tools available to review agent
  - [ ] Structured P0-P3 output format

  **QA Scenarios**:
  ```
  Scenario: Argus profile restricts tools
    Tool: Bash
    Steps:
      1. Load argus.toml profile
      2. Assert tool_allowlist contains Read, Grep, Glob, LSP
      3. Assert tool_allowlist does NOT contain Edit, Write, Bash
      4. cargo test -p jfc-ui --all-features -- test_argus_profile
    Expected: Read-only tools only
    Evidence: .sisyphus/evidence/task-23-argus.txt
  ```

  **Commit**: YES (groups with 24-28)
  - Message: `feat(orchestration): argus review, ralph loop, tmux, handoff`

- [x] 24. Ralph Continuation Loop

  **What to do**:
  - Implement loop pattern: after agent completes, check if work is done
  - Detection: look for TODO items still pending, compilation errors, test failures
  - If incomplete: re-invoke agent with remaining context (max 3 iterations)
  - Configurable in `.jfc/features.toml`: `[loop] max_iterations = 3, check_compile = true, check_tests = true`
  - Log each iteration with reason for continuation

  **Recommended Agent Profile**:
  - **Category**: `general`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 23, 25-28)
  - **Parallel Group**: Phase 4
  - **Blocks**: None
  - **Blocked By**: Tasks 11, 17

  **References**:
  - OMO Ralph loop — detect incomplete → retry
  - JFC existing todo system

  **Acceptance Criteria**:
  - [ ] Loop detects compilation failure → retries
  - [ ] Loop respects max_iterations limit
  - [ ] Each iteration logged with reason

  **QA Scenarios**:
  ```
  Scenario: Loop retries on compilation failure
    Tool: Bash
    Steps:
      1. Agent produces code that doesn't compile
      2. Loop detects cargo build failure
      3. Re-invokes agent with error context
      4. Agent fixes, loop detects success, exits
      5. cargo test -p jfc-ui --all-features -- test_ralph_loop
    Expected: Loop retries once, succeeds on second attempt
    Evidence: .sisyphus/evidence/task-24-ralph.txt
  ```

  **Commit**: YES (groups with 23, 25-28)

- [x] 25. Tmux Interactive Tool

  **What to do**:
  - Add `ToolInput::Tmux` variant: `{ session: String, command: TmuxCommand }`
  - `TmuxCommand` enum: `SendKeys { keys: String }`, `CapturPane`, `NewSession { name: String }`, `Kill { session: String }`
  - Execute via `tokio::process::Command("tmux", [subcommand, args...])`
  - Return captured output as tool result
  - Safety: refuse to kill sessions not created by JFC

  **Recommended Agent Profile**:
  - **Category**: `general`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 23, 24, 26-28)
  - **Parallel Group**: Phase 4
  - **Blocks**: None
  - **Blocked By**: Task 1

  **References**:
  - OMO tmux integration — send-keys, capture-pane
  - JFC existing `tokio::process::Command` usage in tools.rs

  **Acceptance Criteria**:
  - [ ] Can create tmux session
  - [ ] Can send keys and capture output
  - [ ] Cannot kill sessions not created by JFC

  **QA Scenarios**:
  ```
  Scenario: Tmux tool creates session and captures output
    Tool: Bash
    Steps:
      1. Create session "jfc-test"
      2. Send keys "echo hello"
      3. Capture pane → contains "hello"
      4. Kill session "jfc-test"
      5. cargo test -p jfc-ui --all-features -- test_tmux_tool
    Expected: Session lifecycle works, output captured
    Evidence: .sisyphus/evidence/task-25-tmux.txt
  ```

  **Commit**: YES (groups with 23, 24, 26-28)

- [x] 26. Comment Checking Hook (AI-Slop Detection)

  **What to do**:
  - Register `AfterToolDispatch` hook for Edit/Write tools
  - Scan written content for AI-slop patterns:
    - Excessive `// This function...` comments
    - `/* eslint-disable */` or `#[allow(unused)]` spam
    - "TODO: implement" without specifics
    - Over-abstraction markers (generic names: data, result, item, temp, handler)
  - If detected: log warning, add note to session (advisory, not blocking)
  - Pattern list configurable in `[hooks.comment_check]` section of `.jfc/features.toml`

  **Recommended Agent Profile**:
  - **Category**: `general`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 23-25, 27-28)
  - **Parallel Group**: Phase 4
  - **Blocks**: None
  - **Blocked By**: Tasks 11, 13

  **References**:
  - OMO comment checking hook
  - JFC plan "Must NOT Have" anti-slop guardrails

  **Acceptance Criteria**:
  - [ ] Detects common AI-slop patterns
  - [ ] Advisory only (never blocks edit)
  - [ ] Pattern list configurable

  **QA Scenarios**:
  ```
  Scenario: Slop detection warns on generic comments
    Tool: Bash
    Steps:
      1. Write code with "// This function processes the data"
      2. Hook fires, detects slop pattern
      3. Warning logged (not blocking)
      4. cargo test -p jfc-ui --all-features -- test_slop_detection
    Expected: Warning emitted, edit not blocked
    Evidence: .sisyphus/evidence/task-26-slop.txt
  ```

  **Commit**: YES (groups with 23-25, 27-28)

- [x] 27. Session Handoff Protocol

  **What to do**:
  - Implement `/handoff` command: generates context summary for new session
  - Summary includes: files modified, decisions made, todos remaining, key context
  - Output as markdown saved to `.jfc/handoff/{timestamp}.md`
  - New session can load handoff via `/resume {timestamp}`
  - Context compression: only include last N messages + all tool results

  **Recommended Agent Profile**:
  - **Category**: `general`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 23-26, 28)
  - **Parallel Group**: Phase 4
  - **Blocks**: None
  - **Blocked By**: Task 1

  **References**:
  - OMO `/handoff` command
  - JFC existing session persistence

  **Acceptance Criteria**:
  - [ ] Handoff generates readable markdown summary
  - [ ] Resume loads context into new session
  - [ ] Context compressed to fit model window

  **QA Scenarios**:
  ```
  Scenario: Handoff generates and resumes
    Tool: Bash
    Steps:
      1. Run /handoff in session with history
      2. Verify .jfc/handoff/*.md created
      3. New session loads via /resume
      4. Verify context available
      5. cargo test -p jfc-ui --all-features -- test_handoff
    Expected: Summary created, resume loads it
    Evidence: .sisyphus/evidence/task-27-handoff.txt
  ```

  **Commit**: YES (groups with 23-26, 28)

- [x] 28. End-to-End Integration Test

  **What to do**:
  - Full orchestration cycle with all features enabled:
    1. Session starts → intent classified
    2. User requests code change → Hashline edit resolves
    3. Permission check allows edit
    4. Hook fires before tool, logs
    5. Background agent spawned for parallel research
    6. Comment check runs after edit
    7. Review agent provides feedback
  - Single integration test exercising the full pipeline

  **Recommended Agent Profile**:
  - **Category**: `general`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: NO (depends on all previous)
  - **Parallel Group**: Phase 4 (last task)
  - **Blocks**: F1-F4
  - **Blocked By**: All previous tasks

  **References**:
  - All feature modules

  **Acceptance Criteria**:
  - [ ] Full pipeline executes without errors
  - [ ] Each feature fires in correct order
  - [ ] Token tracking accurate across all agents

  **QA Scenarios**:
  ```
  Scenario: Full orchestration pipeline
    Tool: Bash
    Steps:
      1. Enable all features
      2. Send implementation request
      3. Verify: intent=Implementation, permission=Allow, hook fired, edit resolved via Hashline
      4. Verify: background agent spawned, comment check ran
      5. cargo test -p jfc-ui --all-features -- test_e2e_orchestration
    Expected: All features fire in correct sequence
    Evidence: .sisyphus/evidence/task-28-e2e.txt
  ```

  **Commit**: YES (groups with 23-27)

---

## Final Verification Wave

- [x] F1. **Plan Compliance Audit** — `oracle`
  Read plan end-to-end. For each "Must Have": verify implementation exists. For each "Must NOT Have": search for forbidden patterns. Check all feature flags exist and default to off.
  Output: `Must Have [N/N] | Must NOT Have [N/N] | VERDICT`

  **QA Scenarios**:
  ```
  Scenario: Must-Have verification
    Tool: Bash
    Steps:
      1. rg "hashline" crates/jfc-ui/Cargo.toml → feature exists
      2. rg "permission-automation" crates/jfc-ui/Cargo.toml → feature exists
      3. rg "hooks" crates/jfc-ui/Cargo.toml → feature exists
      4. rg "background-agents" crates/jfc-ui/Cargo.toml → feature exists
      5. rg "intent-gate" crates/jfc-ui/Cargo.toml → feature exists
      6. rg "landlock-sandbox" crates/jfc-ui/Cargo.toml → feature exists
      7. rg 'default = \[' crates/jfc-ui/Cargo.toml → empty array []
      8. cargo build -p jfc-ui (no features) → compiles
      9. cargo build -p jfc-ui --all-features → compiles
    Expected: All 9 checks pass
    Evidence: .sisyphus/evidence/F1-compliance.txt

  Scenario: Must-NOT-Have verification
    Tool: Bash
    Steps:
      1. rg "unsafe" crates/jfc-ui/src --include '*.rs' -l → 0 files
      2. rg "lua|rhai|wasm" crates/jfc-ui/Cargo.toml → 0 matches
      3. rg "dyn Hook|Box<dyn Hook" crates/jfc-ui/src → 0 matches
      4. Count new .rs files per feature: none exceeds 3
    Expected: All forbidden patterns absent
    Evidence: .sisyphus/evidence/F1-must-not.txt
  ```

- [x] F2. **Code Quality Review** — `general`
  Run `cargo clippy --all-features -- -D warnings`. Check for `unsafe`, `unwrap()` in non-test code, trait objects in hot paths. Verify no new binary crates created. Verify ToolInput enum has only ADDITIVE changes (new variants like Tmux are allowed; existing variant signatures unchanged).
  Output: `Build [PASS/FAIL] | Clippy [PASS/FAIL] | Tests [N pass] | VERDICT`

  **QA Scenarios**:
  ```
  Scenario: Clippy and build pass
    Tool: Bash
    Steps:
      1. cargo clippy --all-features -- -D warnings → exit 0
      2. cargo test --all-features → all pass
      3. rg "unwrap()" crates/jfc-ui/src --include '*.rs' -l | filter out test files → 0 in non-test code
      4. ls crates/ → same crate list as before (no new binary crates)
    Expected: Clippy clean, tests pass, no unwrap in prod code
    Evidence: .sisyphus/evidence/F2-quality.txt
  ```

- [x] F3. **Real QA** — `general` (+ `playwright` skill if UI)
  Start from clean state. Execute EVERY QA scenario from EVERY task — follow exact steps, capture evidence. Test cross-task integration (features working together, not isolation). Test edge cases: empty state, invalid input, rapid actions. Save to `.sisyphus/evidence/final-qa/`.
  Output: `Scenarios [N/N pass] | Integration [N/N] | Edge Cases [N tested] | VERDICT`

  **QA Scenarios**:
  ```
  Scenario: Cross-feature integration (permissions + hooks)
    Tool: Bash
    Steps:
      1. Enable all features
      2. Configure: permission deny Edit:*.secret, hook logs BeforeToolDispatch
      3. Request Edit to "config.secret" → denied by permission before hook fires
      4. Request Edit to "src/lib.rs" → hook fires, permission allows, edit succeeds
      5. cargo test -p jfc-ui --all-features -- test_permission_hook_integration
    Expected: Permission evaluation precedes hook execution
    Evidence: .sisyphus/evidence/F3-integration.txt

  Scenario: Edge case — all features enabled with empty config
    Tool: Bash
    Steps:
      1. Delete .jfc/features.toml
      2. cargo test -p jfc-ui --all-features → all tests pass
      3. Start JFC → no crash, defaults active
    Expected: Graceful behavior with missing config
    Evidence: .sisyphus/evidence/F3-empty-config.txt
  ```

- [x] F4. **Scope Fidelity Check** — `general`
  Verify no scripting engine added. No more than 3 files per feature. No breaking ToolInput changes. Feature flags all present and default-off.
  Output: `Scope [CLEAN/N issues] | VERDICT`

  **QA Scenarios**:
  ```
  Scenario: File count per feature
    Tool: Bash
    Steps:
      1. Count files with "hashline" in path → ≤ 3
      2. Count files with "permission" in path → ≤ 3
      3. Count files with "hooks" in path → ≤ 3
      4. Count files with "background" in path → ≤ 3
      5. Count files with "intent" in path → ≤ 3
      6. Count files with "sandbox" or "landlock" in path → ≤ 3
    Expected: Each feature ≤ 3 new files
    Evidence: .sisyphus/evidence/F4-scope.txt

  Scenario: No scripting runtime
    Tool: Bash
    Steps:
      1. rg "lua|rhai|wasm|rlua|mlua" Cargo.lock → 0 matches
      2. rg "scripting|eval(" crates/jfc-ui/src → 0 matches
    Expected: No scripting dependencies
    Evidence: .sisyphus/evidence/F4-no-scripting.txt
  ```

---

## Commit Strategy

| After Tasks | Message | Pre-commit |
|-------------|---------|------------|
| 1-3 | `feat(core): add feature flag scaffolding, test infra, config system` | `cargo build --all-features` |
| 4-6 | `feat(edit): hashline content-anchored edit resolution` | `cargo test --all-features` |
| 7-10 | `feat(permissions): TOML-driven permission automation` | `cargo test --all-features` |
| 11-16 | `feat(hooks): lifecycle hook system + intent classification` | `cargo test --all-features` |
| 17-22 | `feat(agents): background agent manager + landlock sandbox` | `cargo test --all-features` |
| 23-28 | `feat(orchestration): argus review, ralph loop, tmux, handoff` | `cargo test --all-features` |

---

## Success Criteria

### Verification Commands
```bash
cargo test --all-features                    # All tests pass
cargo clippy --all-features -- -D warnings   # Clean
cargo build --workspace                      # Full workspace builds
cargo bench --all-features                   # Hook latency < 1ms p99
```

### Final Checklist
- [ ] Hashline improves edit success rate (measurable via test harness)
- [ ] Permission rules block/allow correctly per TOML config
- [ ] Hooks fire in deterministic order with negligible overhead
- [ ] Background agents run isolated, results collected
- [ ] Intent classification runs in <5ms
- [ ] Landlock sandbox prevents path escape for economy solvers
- [ ] All features independently togglable via feature flags
- [ ] No new binary crates, no unsafe blocks in JFC src, no scripting runtime

---

## Research References

| Source | Key Contribution | Location |
|--------|-----------------|----------|
| OMO background-agent | Background agent spawn/track/collect pattern | `~/WebstormProjects/forks/oh-my-opencode/src/features/background-agent/manager.ts` (external, read-only reference) |
| OMO model-resolution | Multi-provider model routing with fallback | `~/WebstormProjects/forks/oh-my-opencode/src/shared/model-resolution-pipeline.ts` (external, read-only reference) |
| OMO permission-automation | Glob-based permission rules | `~/WebstormProjects/forks/oh-my-opencode/src/features/permission-automation/rule-matcher.ts` (external, read-only reference) |
| Codex-rs landlock | Linux LSM sandbox policy builder | `research/openai-codex/codex-rs/core/src/landlock.rs` (in-workspace) |
| Codex-rs exec_policy | Execution restriction levels | `research/openai-codex/codex-rs/core/src/exec_policy.rs` (in-workspace) |
| Codex-rs client_common | OpenAI Responses API streaming | `research/openai-codex/codex-rs/core/src/client_common.rs` (in-workspace) |
| OMO/opencode intent classification | Heuristic intent classification | Concept from opencode's agent routing (no specific file — derived from how opencode's /start-work dispatches by intent type). Design is original to JFC. |
| OMO Hashline (from opencode docs) | Content-hash-anchored line addressing | `~/WebstormProjects/forks/opencode/packages/opencode/` (external, read-only reference) |

> **Note**: OMO paths are external reference material at `~/WebstormProjects/forks/oh-my-opencode/`. They are read-only design references, not build dependencies. The implementation in JFC is self-contained — no code is copied verbatim.
