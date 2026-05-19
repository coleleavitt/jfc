# Agent Architecture Research: Long-Running Task Completion & Turn Management

## Date: 2026-05-19
## Context: jfc CLI coding agent — session analysis + industry research

---

## Part 1: The Core Problem (From jfc Session Logs)

### Observed Symptoms

Analysis of 185 sessions (~946MB) in `~/.config/jfc/sessions/` reveals these recurring patterns:

#### 1. Consecutive Assistant Messages ("TurnsAssistant" Bug)
**Evidence:** `ses_20260515_175208.json` — session shape:
```
1 user → 4 assistant → 1 user → 15 assistant → 1 user → 9 assistant → 1 user → 22 assistant
```

**Root cause:** Each agentic sub-stream (tool execution + response) creates a new `ChatMessage::assistant(...)` in `app.messages`. On save, these aren't coalesced. On resume (`--continue`), the provider message builder sees N consecutive assistant messages and must merge/strip them — but `ensure_user_last()` only appends a synthetic user at the END, not between the assistants.

**Impact:** Anthropic API rejects trailing assistant prefill on Opus 4.6+. Sessions with 10+ sub-streams per user turn get corrupted on resume.

**Status (as of commit 36ad8bd):** Partially fixed:
- `coalesce_on_save` merges consecutive assistants in persisted JSON
- `merge_consecutive_same_role()` handles runtime
- But edge cases remain with `server_tool_use` blocks and mixed pause_turn + local tools

#### 2. Incomplete Work / Stub Generation
**Evidence:** Sessions with 76-104 "fix" requests from the user (ses_20260513_035753, ses_20260516_012322), 226 unmerged branches in one project, repeated "do all of it please" followed by partial execution.

**Pattern:** User requests N tasks → agent creates task list → starts executing → hits context window / token limit / max_turns → declares "done" with stubs or partial skeletons → user has to re-request in next session.

**Key observation from ses_20260514_042620:** Agent spawned work across 226 branches, many with merge conflicts and incomplete implementations. No unified tracking of which are actually done vs stub.

#### 3. Agent Lifecycle Bugs
**From ses_20260513_035753 (104 fix requests):**
- Bug 1: Teammates marked "Done" because abort handle dropped immediately after spawn
- Bug 2: Task-store switch bug (unknown task id t35-t46)  
- Bug 3: Spinner state stuck in "thinking" after all agents spawned
- Bug 4: CompactionDone doesn't resume tool-result continuation

#### 4. Silent Failures / False Completion
**From ses_20260516_012322:**
- `parse_stop_reason(None) => EndTurn` silently treats unknown/missing stop reasons as "turn done"
- Same class of bug that hid `pause_turn` for months
- Sub-agents report "Done" when their abort handle is dropped (lifecycle cancellation ≠ completion)

#### 5. Repeated Session Restarts
- 4 sessions required `--continue` multiple times
- Users asking "check if all of them finished" repeatedly
- Pattern: fan-out N agents → lose track → ask repeatedly → some finished, some didn't

---

## Part 2: Industry Research Findings

### A. Anthropic's GAN-Inspired Three-Agent Harness
**Source:** Anthropic Engineering Blog (Mar 2026), Prithvi Rajasekaran
**Paper:** "Harness Design for Long-Running Application Development"

**Architecture:**
```
┌─────────┐     ┌───────────┐     ┌───────────┐
│ PLANNER │────▶│ GENERATOR │────▶│ EVALUATOR │
│         │     │           │◀────│           │
│ Decomposes    │ Builds in │     │ Tests via │
│ into sprints  │ sprints   │     │ Playwright│
└─────────┘     └───────────┘     └───────────┘
      ▲                                 │
      └─────────── feedback ────────────┘
```

**Key principles:**
1. **Sprint decomposition**: Break work into 5-15 min sprints with clear done-criteria
2. **Adversarial evaluator**: Separate "judge" agent that's easier to tune harsh than making builder self-critical
3. **Context resets between sprints**: Each sprint gets fresh context with only the plan + prior results (NOT the full conversation)
4. **GAN dynamic**: Generator improves because evaluator provides specific failure signals, not just pass/fail

**Opus 4.6+ simplification:**
- Dropped per-sprint context resets (model handles long context better)
- Dropped per-sprint evaluation (evaluator runs at end of session instead)
- Cut costs ~50%

**Relevance to jfc:** The sprint-decomposition pattern directly addresses the "78 tasks → stubs" problem. Instead of one giant agentic loop, decompose into bounded sprints where each sprint has a clear completion signal.

### B. MemoryOS: Three-Tier Hierarchical Memory
**Source:** Kang et al. (Tencent AI Lab + BUPT), arXiv:2506.06326

**Architecture:**
```
┌───────────────────────────────────────────┐
│           Response Generation              │
├───────────────────────────────────────────┤
│            Memory Retrieval                │
│  STM: all recent │ MTM: segment→page      │
│  LPM: persona+KB │ (2-stage retrieval)    │
├───────────────────────────────────────────┤
│            Memory Updating                 │
│  STM→MTM: FIFO  │ MTM→LPM: heat-based    │
├───────────────────────────────────────────┤
│            Memory Storage                  │
│  STM (queue=7)   │ MTM (segments/pages)   │
│  LPM (persona)   │                        │
└───────────────────────────────────────────┘
```

**Key metrics:**
- 49.11% F1 improvement over baselines on LoCoMo benchmark
- 4.9 avg LLM calls per response (vs 13 for A-Mem, 4.3 for MemGPT)
- 3,874 tokens per response (vs 16,977 for MemGPT)

**Heat-based eviction formula:**
```
Heat = α·N_visit + β·L_interaction + γ·R_recency
R_recency = exp(-Δt / μ)
```

**Relevance to jfc:** The STM/MTM/LPM hierarchy maps to:
- STM = current session messages (what we have now)
- MTM = session summaries grouped by topic/project (what we need)
- LPM = user preferences, project conventions, recurring patterns (`.jfc/memory/`)

### C. Claude Code's Task System (v2.1.142+)
**Source:** Claude Code docs, GitHub issues, community reports

**Architecture:**
- Replaced `TodoWrite` (flat markdown list) with structured `TaskCreate/TaskUpdate/TaskGet/TaskList`
- Tasks persist to `.tasks.json` in project root
- Tasks have: id, subject, description, status, blocked_by, verification_command
- Shared across sessions via file — any session reading the same `.tasks.json` sees the same state
- Sub-agents (Task tool) can read/update the task list

**Key limitation reported (GitHub #55754, #6159, #4935):**
- Agent stops mid-task, provides summary "as if complete"
- `max_turns` not enforced (subagents continue past limit)
- Stop hooks can cause infinite loops when background agents are pending
- No mechanism to distinguish "task done" from "agent ran out of context"

**Key insight (GitHub #39663):**
- "Claude responds to failures by writing rules about failures, not by actually fixing the problem. It's bureaucracy, not engineering."
- Plans referenced in MEMORY.md but never actually persisted to disk
- Session N+1 can't recover context from session N

### D. Beads: Coding Agent Memory System (Steve Yegge, Oct 2025)
**Concept:** Each "bead" is a self-contained unit of work with:
- Input state (what files looked like before)
- Actions taken
- Output state (what files look like after)
- Summary/rationale

**Key properties:**
- Beads are composable and replayable
- Session continuity comes from reading the bead chain, not the full conversation
- "Unprecedented continuity from session to session"

**Relevance:** Maps to jfc's git-worktree-per-task model — each worktree IS a bead.

### E. Claude Code Design Space Analysis (arXiv:2604.14228)
**Key findings on context management:**

1. **Five-stage context shaping pipeline:**
   - System prompt assembly (static + dynamic)
   - Message pruning (remove old tool outputs)
   - Conversation compaction (summarize old turns)
   - Retrieval injection (add relevant context)
   - Auto-compact (emergency when near limit)

2. **Recovery mechanisms:**
   - Max output tokens escalation (retry with higher limit, up to 3x)
   - Reactive compaction (summarize just enough to free space)
   - Prompt-too-long handling (collapse + retry before terminating)
   - Fallback model switching

3. **Stop conditions:**
   - No tool use (model produced only text)
   - Max turns reached
   - Context overflow
   - Hook intervention
   - Explicit abort

4. **The key gap:** No "I'm not done yet but hitting limits" signal. The model either continues (tool_use) or stops (text-only). There's no "pause and resume later" at the task level.

---

## Part 3: Architectural Recommendations for jfc

### Priority 1: Fix the Consecutive-Assistant / Turn Corruption

**What:** The "TurnsAssistant" bug where sessions accumulate N assistant messages per user turn.

**How:**
1. Coalesce on save is already implemented — verify it handles ALL edge cases:
   - server_tool_use blocks must not be merged across
   - pause_turn resume blocks must preserve their trailing-assistant signal
2. Add a `validate_session_shape()` that runs on `--continue` load and repairs any pre-coalesce sessions
3. Add `tracing::warn!` for the silent `EndTurn` defaults instead of silently declaring "done"

### Priority 2: Sprint-Based Task Decomposition (The Real Fix for Stubs)

**What:** Instead of one giant agentic loop that hits context limits and produces stubs, decompose into bounded sprints.

**Architecture:**
```
┌─────────────────────────────────────────────────────┐
│                   TASK FILE                           │
│  .jfc/tasks.json (persists across ALL sessions)      │
│  - Structured tasks with acceptance criteria          │
│  - Verification commands                             │
│  - Dependencies (blocked_by)                         │
│  - Status: pending/in_progress/completed/failed      │
└──────────────────────┬──────────────────────────────┘
                       │
         ┌─────────────┼─────────────┐
         │             │             │
    ┌────▼────┐  ┌────▼────┐  ┌────▼────┐
    │Sprint 1 │  │Sprint 2 │  │Sprint 3 │
    │(fresh   │  │(fresh   │  │(fresh   │
    │context) │  │context) │  │context) │
    └────┬────┘  └────┬────┘  └────┬────┘
         │             │             │
         ▼             ▼             ▼
    git commit    git commit    git commit
    task.done()   task.done()   task.done()
```

**Key design decisions:**
1. **Task file is the source of truth** — not the conversation. Every session reads it on start.
2. **Each sprint = 1 task** — bounded scope, clear acceptance criteria, verification command.
3. **Fresh context per sprint** — load task description + relevant file content, NOT the full history.
4. **Commit after each sprint** — even if it's a partial step, it's checkpointed in git.
5. **Failed sprint ≠ session end** — mark task failed with error, move to next independent task.

**Implementation in jfc:**
- Already have: `TaskCreate/TaskUpdate/TaskDone/TaskList` tools
- Already have: git worktrees for isolation
- Need: **task file persistence across sessions** (currently tasks live in memory)
- Need: **sprint boundary detection** (when to stop current task, save progress, move on)
- Need: **verification runner** (run the `verification_command` and mark pass/fail)
- Need: **context budget per sprint** (don't let one task consume the whole window)

### Priority 3: Session Memory / MTM Layer

**What:** When a session ends (by context limit, max_turns, or user quit), persist a structured summary that the NEXT session can use to continue.

**Implementation:**
```
.jfc/memory/
├── sessions/
│   ├── 2026-05-19_sprint1_summary.md   ← auto-generated
│   └── 2026-05-19_sprint2_summary.md
├── project/
│   └── architecture.md                  ← evolving project knowledge
└── feedback/
    └── user_preferences.md              ← learned from corrections
```

This already partially exists in jfc's memory system. The gap is:
- No auto-generation of session summaries on exit
- No structured "what was I working on" state that persists to disk
- No retrieval of relevant past session summaries on new session start

### Priority 4: Evaluator / Verification Agent

**What:** After each sprint completes, run a verification step. This is the "adversarial evaluator" from Anthropic's harness.

**Implementation:**
- Each task has `verification_command` (e.g., `cargo test -p jfc-graph`)
- After task.done(), automatically run verification
- If verification fails: mark task as failed, create a "fix" sub-task
- Prevents the "declares done but left stubs" pattern

### Priority 5: Graceful Degradation on Context Limits

**What:** When approaching token limits, don't produce stubs. Instead:
1. Commit whatever is actually complete
2. Mark current task as "in_progress" with a progress note
3. Create/update the task file with remaining work
4. Exit cleanly so next session can resume

**Key principle:** A committed partial implementation is infinitely better than an uncommitted stub, because git preserves it and the task file tells the next session where to pick up.

---

## Part 4: Specific Code Changes for jfc

### Immediate (can do now):

1. **`crates/jfc-ui/src/stream/messages/turns.rs`** — Add `validate_session_shape()` repair function for corrupted sessions loaded via `--continue`

2. **`crates/jfc-ui/src/event_loop.rs`** — Add `tracing::warn!` on all silent EndTurn default paths (lines where `unwrap_or(StopReason::EndTurn)`)

3. **`crates/jfc-ui/src/session/`** — On session save, add auto-summary generation that writes to `.jfc/memory/sessions/`

4. **`crates/jfc-ui/src/tools/subagent.rs`** — Enforce `max_turns` properly (the subagent currently can exceed it)

### Medium-term:

5. **New module: `crates/jfc-ui/src/sprint.rs`** — Sprint boundary detection:
   - Monitor token usage per task
   - When approaching 70% of remaining context budget: commit, save progress, move on
   - On max_turns approaching: same behavior

6. **Task persistence** — Write `.jfc/tasks.json` to disk (currently only in memory during session):
   - Auto-save on every TaskUpdate
   - Load on session start
   - Multiple sessions/agents can read/write (file locking)

7. **Verification runner** — After `TaskDone`, if `verification_command` exists:
   - Run it in background
   - If fails: auto-create fix task
   - If passes: emit success confirmation

### Long-term:

8. **Evaluator agent** — Spawn a separate agent that reviews completed work:
   - Reads the diff
   - Runs tests
   - Checks against acceptance criteria
   - Can reject and send back for revision

9. **MemoryOS-style retrieval** — On session start:
   - Load relevant session summaries (by topic/file overlap)
   - Load project architecture knowledge
   - Load user preferences/feedback
   - Inject into system prompt as working context

---

## References

1. Rajasekaran, P. (2026). "Harness design for long-running application development." Anthropic Engineering.
2. Liu, J. (2026). "GAN-Inspired Multi-Agent Harnesses for Long-Running Autonomous Software Engineering." Medium.
3. Kang, J. et al. (2025). "Memory OS of AI Agent." arXiv:2506.06326.
4. Yegge, S. (2025). "Introducing Beads: A coding agent memory system." Medium.
5. Huang, Z. et al. (2026). "Dive into Claude Code: The Design Space of Today's and Future AI Agent Systems." arXiv:2604.14228.
6. Claude Code Issues: #27298 (layered memory), #39663 (context loss on restart), #6159 (stops mid-task), #55754 (stop hook infinite loop), #4935 (incomplete task execution).
7. Claude Code Docs: "Todo Lists" — TaskCreate/TaskUpdate system (v2.1.142+).
8. Mem0 (2025). "Building production-ready AI agents with scalable long-term memory." arXiv:2504.19413.
