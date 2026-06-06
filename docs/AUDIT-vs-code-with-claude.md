# JFC Audit — measured against the "Code with Claude" talks

Read-only audit (no code changed). Scope: tools, system prompt, skills,
subagents, memory/dreaming, context management, verification loops, evals,
and the agent economy. Findings verified by reading source; `*/research/*`
(vendored rust-analyzer/tree-sitter) excluded from all counts.

**Headline:** JFC already implements the *architecture* the talks advocate —
progressive-disclosure skills, fresh-context subagents, a real "dreaming"
memory consolidator, verification agents, background routines, coverage→graph,
and competitive solver bounties. The gaps are not missing subsystems; they're
**(1) tool-surface bloat fighting the context budget, (2) no self-improving
verification skill, (3) no agent-quality eval harness, and (4) stale docs.**

---

## What the talks said to build → what JFC has

| Talk lesson | JFC status | Evidence |
|---|---|---|
| Skills: name+desc in context, body on demand; personal + project dirs | ✅ **Implemented well** | `jfc-agents/src/registry.rs:79-87` (`~/.claude/skills` + `<root>/.claude/skills`, also `.codex`/`.agents`); only name+desc rendered (`lifecycle.rs:19-47`, desc capped 200 chars); body injected only on `Skill` tool call (`tools/subagent.rs:9-63`) |
| Subagents used sparingly: parallelize + fresh-context review | ✅ **Implemented well** | builtin agents explore/plan/verification/orchestrator/general-purpose (`jfc-agents/src/builtin_prompts/*.txt`); separate context windows; parallel fan-out via daemon workers (just used 3 here) |
| Verification = adversarial, "try to break it" | ✅ **Prompt is excellent** | `builtin_prompts/verification.txt:1` "your job is not to confirm it works — it's to try to break it"; build-fail/test-fail = automatic FAIL |
| Memory: persistent, versioned, filesystem-like | ✅ **Exceeds the bar** | `jfc-memory/src/store.rs:30-45,135-188` — user/project/team `.md` + frontmatter (`normalized_hash`, TTL, `verification_status`, `superseded_by`); atomic writes `:761-796` |
| "Dreaming": async de-dup / fact-check / index / archive | ✅ **Implemented for real** | `jfc-learn/src/dreamer.rs` — `Consolidate`/`Verify`/`ArchiveStale`/`Improve`/`MaintainDocs`, lease + circuit breaker; scheduled hourly via daemon |
| Context mgmt: compact / clear / context viewer; recall not full-dump | ✅ **Implemented** | memory **recall** (LLM-selected) replaces full dump (`stream/request.rs:475-506`); plan recall `:519-549`; CLAUDE.md 5-layer hierarchy `context.rs:267-324` |
| Background loops / routines (`/loop`) | ✅ **Implemented** | daemon cron `jfc-daemon/src/cron.rs` (`@daily`, `@every 5m`), `CronCreate`/`ScheduleWakeup` tools |
| Coverage→graph `untested` operator | ✅ **Implemented** | `tools/defs/graph.rs:424` `run_coverage`, `dispatch_heavy.rs:415` runs `cargo llvm-cov` |
| Competitive solvers + budget enforcement | ✅ **Implemented** | `jfc-economy` bounties (`post_bounty`/`run_bounty`), charter spending caps |
| Prefer few human primitives over many bespoke tools | ⚠️ **Violated — see Gap 1** | 86 `ToolKind` variants / ~69 native defs (`jfc-core/src/tool_kind.rs`, `tools/defs/`) |
| Self-improving verification skill (re-documents on every blocker) | ❌ **Missing — Gap 2** | verification agent is ephemeral & read-only; no persisted skill it edits |
| Eval harness w/ graders + LLM judges (hill-climb agent quality) | ❌ **Missing — Gap 3** | no grader/judge/eval-case infra anywhere in `crates/*/src` |

---

## Gap 1 — Tool-surface bloat (the biggest, and it's self-inflicted)

**Fact:** 86 `ToolKind` variants (`jfc-core/src/tool_kind.rs`), ~69 native tool
defs concatenated unconditionally in `all_tool_defs()`
(`tools/defs/mod.rs:10-20`), **plus** MCP server tools appended on top
(`safe_tools.rs:268`) with only name-collision de-dup and **no cap**.

The talks' central efficiency argument: *every tool definition burns context
tokens on every turn*, and Claude Code auto-switches to **tool-search mode once
MCP tools exceed 10% of context**. JFC ships `tool_search`/`tool_suggest` tools
(the search mechanism exists!) but — confirmed — there is **no gating that
drops the 69 always-on defs from the prompt**. Every turn pays for all 69 (the
schema for this very session shows the full catalogue). Many are narrow enough
to be primitives or skill-gated:

- 15 `graph_*` tools — could collapse behind one `graph` tool with a `mode`
  arg, or move the rarer 6 (status/files/explore/node/outline/grep) into a skill.
- 7 `plan_*` + 7 `task_*` + several `learn_*` / `cron_*` / `scratchpad_*` /
  `team_*` — lifecycle verbs that are rarely all needed in one session.

**Recommendation (not done):** introduce a default tool tier (the ~12 core
primitives + task tools) and lazy-load the rest via the existing
`tool_search`/`Skill` machinery — i.e. actually *use* the progressive-disclosure
you already built for skills, on tools too. This is the single highest-leverage
context win.

## Gap 2 — Verification exists, but isn't a *self-improving* skill

The talks' specific recipe (Sid Bidasaria): package the verification loop as a
**skill that re-documents itself on every blocker**, so the next run/teammate
doesn't hit the same wall. JFC's verification agent
(`builtin_prompts/verification.txt`) is **read-only and ephemeral** — it's
explicitly forbidden from writing to the project (`:4-6`) and keeps no durable
artifact. So learnings evaporate at session end.

The pieces to fix this already exist: skills are writable
(`.claude/skills/`), and the Dreamer already does `MaintainDocs`. The missing
link is a verification *skill* the agent appends blockers to (auth setup, seed
state, dev-server quirks) — exactly the talk's pattern.

## Gap 3 — No eval harness for agent quality

Confirmed absent: no grader / LLM-judge / eval-case / golden-output infra in
any `crates/*/src` file. JFC has excellent *unit* test discipline (DO-178B docs,
per-function tests) and `run_coverage`, but **nothing measures whether the agent
produces good outputs** — the talks' core "hill-climb on evals, don't trust
vibes" loop. There's no way to A/B a system-prompt change, compare models on
JFC's own tasks, or catch a prompt regression. Given JFC *ships* its own system
prompt + 5 builtin agent prompts, this is the riskiest blind spot: prompt edits
land unmeasured.

The bounty/validator machinery in `jfc-economy` is the closest existing
primitive (adversarial validators scoring solver output) and could seed a
grader harness.

## Gap 4 — Stale onboarding docs (cheap, high-signal)

- **`AGENTS.md` is wrong:** says "Rust workspace: 4 crates (jfc-anthropic-sdk,
  jfc-economy, jfc-graph, jfc)". There are **23 crates**. The crate overview
  lists only those 4. An agent reading this onboards to a false map — directly
  contradicts the talks' "codebase/CLAUDE.md is the source of truth, keep it
  current" principle.
- **No root `CLAUDE.md`** — JFC has a full 5-layer CLAUDE.md loader
  (`context.rs:267`) but doesn't dogfood it at the repo root. The talk's advice
  ("turn your spec into a checked-in skill/CLAUDE.md") is unused on JFC itself.
- `docs/` mixes durable architecture with transient work logs
  (`FIXES_IMPLEMENTED.md`, `WORK_COMPLETION_REPORT.md`,
  `SESSION_ANALYSIS_SUMMARY.md`) — noise for an onboarding agent.
- **8 core dumps (~2.1 GB)** and `dhat-heap.json` sit in the repo root
  (`core.2033498` … `core.4161678`). Not a talk issue, but they pollute the
  working tree an agent globs over.

---

## Net assessment

JFC is, by the talks' own yardstick, an **unusually complete** agent harness —
it has built nearly every primitive Anthropic demoed (skills, dreaming,
verification agents, routines, coverage-graph, competitive solvers) and several
they didn't (code-graph-native navigation, plan recall, the economy layer).

The work it *needs* is consolidation, not construction:
1. **Gate the 69 always-on tools** behind the tool-search/skill machinery you
   already built. (Highest leverage; pure context savings.)
2. **Make verification a self-improving, checked-in skill**, not an ephemeral
   read-only agent.
3. **Stand up a minimal eval harness** (graders + LLM judge, seeded from the
   bounty-validator code) so prompt/model changes stop landing blind.
4. **Fix `AGENTS.md` (4→23 crates), add a root `CLAUDE.md`, and quarantine
   work-log docs + core dumps.**

Priority order by leverage-to-effort: **4 (trivial) → 1 (high) → 2 (medium) → 3 (largest build).**
