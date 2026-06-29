# Auto-Review Triggering: Diagnosis & Design Note

Status: design proposal (no code changes yet)
Owner area: `crates/jfc-engine/src/auto_review.rs`, `crates/jfc-engine/src/workflows/registry.rs` (`code-review`)
Last updated: 2026-06-14

## TL;DR

The background code-review fires a large (~22-agent at `level=high`) multi-agent
fan-out on essentially **every Rust-touching turn**. That is *time-triggered*
control with a tiny period: it over-actuates, reviews half-finished states, and
re-reviews code it already cleared. The fix is not "review on every commit"
(that just swaps one external clock for another, burstier one). It is to make the
review an **event-triggered control loop with memory**: a cheap per-turn monitor
accumulates a risk signal, and the expensive review fires only when that signal
crosses a barrier (debounced), with commit as an upper-bound forcing trigger, and
with verified findings memoized by content hash so unchanged code is never
re-reviewed. Where the deterministic monitor is ambiguous, a single cheap LLM
gate (mirroring the existing `auto_mode::classify` forced-tool pattern) makes the
review/skip call — so the model has the final say exactly where heuristics are
weak, while the clear-skip/clear-review majority stays zero-LLM.

## Current behaviour (verified against source)

Trigger path:

- `runtime/event_loop/handlers/stream_done.rs:563` calls
  `auto_review::maybe_spawn_after_turn` — but only when `turn_genuinely_done`
  (StopReason::EndTurn, no pending tools/approvals/classifications). So the cadence
  is **once per completed user turn**, not per keystroke. That part is correct.
- `auto_review.rs:32` `maybe_spawn_after_turn`: returns early if mode is
  `Off`/`Manual` or `turn_edited_files` is empty; otherwise computes a trigger
  reason and dispatches the `code-review` workflow as a background task.

Why it feels like "every turn":

1. **Default mode is `Smart`** — `auto_review.rs:314` `auto_review_mode()` falls
   through to `Smart` with no env var, and `~/.config/jfc/config.toml` has no
   `auto_review` key.
2. **`Smart` triggers on almost any Rust edit** — `auto_review.rs:361`
   `smart_auto_review_trigger`: returns `Some(..)` for `>=3` files, OR any `.rs`
   file, `Cargo.toml`/`Cargo.lock`, anything under `crates/`,
   `.github/workflows/`, OR any risk token in file/diff
   (`unsafe`, `auth`, `token`, `Mcp`, ...). In *this* repo that is ~every turn, so
   **Smart ≈ Always here.**

Why each run is heavy:

- Default level is `high` (`auto_review.rs:114`, `JFC_AUTO_REVIEW_LEVEL` unset).
- `registry.rs:266` maps `high` → `{ maxVerify: 14, sweepMax: 4,
  angles: ['bugs','tests','regressions'] }`. The workflow runs:
  `scope` (1) + `find:<angle>` (3) + `verify:<file:line>` (up to 14) + `sweep` (1)
  + `verify-sweep` (up to 4) + `synthesize` (1) ≈ **~22 agent calls per review**.
  This matches the observed Scope/Find/Verify fan-out.

Why repeat turns re-run it:

- Dedup is by `review_signature` (`auto_review.rs:475`): SHA-256 of the file-set +
  `git status --short` + `git diff --numstat HEAD`. It only blocks an **identical**
  diff from re-dispatching. Each new turn edits something new → new signature →
  fresh full fan-out. There is **no reuse** of prior *verified findings*; the
  workflow always rebuilds candidates and re-verifies from scratch.

Already-landed work (commit `8dfed37`) fixed *stability* — phantom-`Failed` agent
leaks, dedup bounding, retention, path normalization — but left the **trigger
cadence** and **per-run cost** unchanged.

Existing knobs (env):

- `JFC_AUTO_REVIEW` = `off` | `manual` | `smart` (default) | `always`.
- `JFC_AUTO_REVIEW_LEVEL` = `low` | `medium` | `high` (default) | `xhigh` | `max`.
- `JFC_AUTO_REVIEW_PROOF_ORACLES` = `0` to skip the pre-review cargo test/clippy pass.

## Theory: this is a control-loop scheduling problem, not a code-review problem

The relevant literature is control theory, not CR tooling:

- **MAPE-K loop** (Kephart & Chess autonomic computing; Rutten, Marchand, Simon,
  *Feedback Control as MAPE-K Loop in Autonomic Computing*, 2013,
  doi:10.1007/978-3-319-74183-3_12; Brun et al., *Engineering Self-Adaptive
  Systems through Feedback Loops*, 2009, doi:10.1007/978-3-642-02161-9_3). Auto-review
  *is* a MAPE-K loop: Monitor edits → Analyze risk → Plan a review → Execute
  agents. **It is missing the K (Knowledge):** no memory of what was already
  reviewed clean, so every actuation re-derives state from zero.
- **Event-triggered vs time-triggered control** (event-triggered control corpus,
  e.g. Ong & Cortés, *Performance-Barrier-Based Event-Triggered Control*,
  arXiv:2108.12702; Delimpaltadakis et al., ETCetera, arXiv:2203.01623). Core
  result: do **not** actuate on a fixed clock; actuate when an *error/risk signal*
  crosses a barrier. This minimizes expensive actuations while staying inside a
  quality envelope. Today's review is time-triggered on the turn clock.
- **Agent loop dynamics** (ReAct / Reflexion / Self-Refine; Tacheny, *Dynamics of
  Agentic Loops in LLMs: A Geometric Theory of Trajectories*, arXiv:2512.10350).
  A loop can over- or under-actuate; the review loop is over-actuating.
- **Practice confirms the boundary** — real LLM reviewers run at the diff/PR
  boundary, not per intermediate edit: Bugdar (arXiv:2503.17302), SWE-PRBench
  (arXiv:2603.26130). CI test selection is the budgeted-actuation analogue:
  Spieker et al. RL for test prioritization in CI (arXiv:1811.04122), DeepOrder
  (arXiv:2110.07443).

## Why "review on every commit" is not the answer by itself

| Trigger axis | Control analogue | Problem |
|---|---|---|
| Every turn (today) | Time-triggered, tiny period | Over-actuates; reviews half-finished states; `Smart ≈ Always` here |
| Every commit | Event-triggered on one external event | Coherent states, but commits are bursty (10 commits in 5 min → 10 fan-outs) and a long mid-feature session has *no* commit for hours |
| Threshold on accumulated risk | True event-triggered (performance-barrier) | Fires when enough risky change has buffered, independent of turn/commit timing |

Commit is a good **upper-bound forcing trigger**, not the scheduling scheme.

## Should the model decide whether to review? (gate question)

Yes — but as the **second** stage of a two-stage gate, not the first. The
control-loop framing wants a cheap deterministic *monitor* that runs every turn,
and an expensive *actuation* that runs rarely. An LLM "should I review?" judgement
sits between them: it is far cheaper than the ~22-agent review but far more
expensive than a token scan, so it should only run when the deterministic monitor
is *ambiguous*.

This is not speculative — the primitive already exists in-tree and is the exact
shape needed:

- `auto_mode::classify` (`crates/jfc-engine/src/auto_mode.rs:320`) runs a small
  model with a **forced classifier tool** (`classify_result`,
  `auto_mode.rs:124`), `max_tokens(1024)`, parses a structured `{should_block,
  reason}` decision, and **fails safe** on any provider/parse error
  (`auto_mode.rs:341-351`). A review gate is the same call with a
  `{should_review, level, reason}` schema.
- A cheap model is already used for ancillary calls (`claude-haiku-4-5`,
  `exploration.rs:749`); the gate would resolve to the same fast tier rather than
  the session's `EngineState.model` (`engine_state.rs:481`).
- The trigger already holds everything the gate needs:
  `EngineState.provider` (`engine_state.rs:479`) and the edited-file diff.

Tiered gate (cheap → expensive):

1. **Deterministic monitor (every turn, free):** the existing risk-token scans +
   numstat. Three outcomes instead of today's boolean:
   - clearly trivial (e.g. comment/doc-only, tiny non-risky diff) → **skip**, no
     LLM;
   - clearly risky (`unsafe`/auth/`Mcp`, large or many-file diff) → **review
     directly**, no gate call needed;
   - ambiguous middle → escalate to step 2.
2. **LLM gate (only on ambiguity, one cheap call):** mirror `auto_mode::classify`
   with a `should_review`+`level` schema; fail-safe = review (never silently drop
   a risky change because the gate errored).
3. **Review actuation (rare):** the `code-review` workflow at the gate-chosen
   level.

Why not let the LLM gate every turn unconditionally: it still costs a model call
per turn and adds latency to the turn-end path; the deterministic monitor already
resolves the clear-skip and clear-review majority for free, so the gate call is
reserved for the genuinely uncertain minority. This keeps the common path
zero-LLM while giving the model the final say exactly where heuristics are weak —
the same division of labour `auto_classifier` (deterministic,
`auto_classifier.rs:161`) vs `auto_mode::classify` (LLM) already uses for tool
approval.

## Proposed design: two-level event-triggered loop with memory

### Level 1 — cheap monitor (every turn, no LLM)

Reuse the diff that is already computed. Accumulate a scalar `risk` across turns
from signals already available in `auto_review.rs`:

- changed line count (`git diff --numstat`, already read in `review_signature`),
- number of files touched,
- risk-token hits (`file_content_has_review_signal` / `git_diff_has_review_signal`
  already exist — `unsafe`, `auth`, `token`, `Mcp`, `+pub `, ...),
- (optional later) AST/cyclomatic delta from `jfc-graph`.

This is the MAPE Monitor+Analyze step, effectively free. When the monitor is
*ambiguous* (neither clear-skip nor clear-review), escalate to the LLM gate
described above rather than defaulting to a full review.

### Level 2 — expensive review (event-triggered)

Fire the `code-review` workflow when **any** of:

- accumulated `risk >= BARRIER` (event-triggered actuation), **or**
- a commit boundary is detected (HEAD moved since last review) — forcing trigger,

subject to a **debounce / quiet-period**: hold for a short idle window and
coalesce a burst of qualifying turns into one run; if a new edit arrives while a
review is in flight, **supersede** the stale run (the cancel token already supports
this — `state.cancel_token.child_token()` at `auto_review.rs:112`).

### Level 3 — add the K: memoize verified findings by content hash

- Key verified findings to a content hash at function/AST-node granularity (via
  `jfc-graph`), not whole-file.
- On a new review, only feed the finder/verifier agents the nodes whose hash
  changed since the last clean review; reuse cached findings for unchanged nodes.
- Invalidate along graph edges (a changed node invalidates dependents' cached
  taint/reachability findings).
- Report **marginal findings only** — diff the new finding set against the cached
  set and surface newly introduced issues, so legacy issues are not re-flagged
  every run.

### Adaptive level (replaces hardwired `high`)

Let the Level-1 risk signal pick the level instead of always `high`: a 5-line
non-risky tweak → `low` (~4 verify agents); a large or `unsafe`/auth-touching diff
→ `high`/`xhigh`. The level knob already exists (`registry.rs:266`); only the
selection is hardwired.

## Expected effect

- Reviews fire on the order of once per *coherent change set* (risk barrier or
  commit), not once per turn → large reduction in fan-out count.
- Each fired review re-verifies only changed nodes (memoization) and at a level
  matched to risk → large reduction in per-run agent count for routine edits.
- Quality envelope preserved: high-risk diffs still escalate to the full ensemble;
  commit acts as a guaranteed upper-bound review point.

## Suggested implementation order (each independently shippable)

1. **Adaptive level** from the existing risk signal (smallest change, biggest
   routine-cost win; no new infra).
2. **Three-way deterministic monitor** — split today's boolean
   `smart_auto_review_trigger` into skip / review / ambiguous, so the clear-skip
   majority stops triggering at all.
3. **LLM review gate** on the ambiguous bucket — mirror `auto_mode::classify`
   with a `{should_review, level, reason}` forced-tool schema on a cheap model
   (`claude-haiku-4-5`), fail-safe = review. Reuses `EngineState.provider`.
4. **Debounce + supersession** around `maybe_spawn_after_turn` (cancel token
   already present).
5. **Risk-barrier + commit forcing trigger** replacing the `Smart` "any .rs"
   heuristic (introduce an accumulator on `EngineState`; owner = engine state, no
   second source of truth).
6. **Finding memoization + marginal reporting** via `jfc-graph` (largest, defers
   to last).

## Verification plan (when implemented)

- Unit: `auto_review_mode` parsing (exists), risk-accumulator threshold crossing,
  debounce coalescing, commit-boundary detection, memoization cache hit/invalidate.
- Integration (per `.claude/rules/architecture.md`): old `/review` manual path
  still works alongside the new trigger; session reload mid-review; Ctrl+C
  supersession; background/foreground interaction.
- `cargo build`, `cargo test`, `cargo clippy --workspace`.

## References

- Rutten, Marchand, Simon (2013) — Feedback Control as MAPE-K Loop. doi:10.1007/978-3-319-74183-3_12
- Brun et al. (2009) — Engineering Self-Adaptive Systems through Feedback Loops. doi:10.1007/978-3-642-02161-9_3
- Ong & Cortés — Performance-Barrier-Based Event-Triggered Control. arXiv:2108.12702
- Delimpaltadakis et al. — ETCetera: beyond Event-Triggered Control. arXiv:2203.01623
- Tacheny (2025) — Dynamics of Agentic Loops in LLMs. arXiv:2512.10350
- Naulty et al. (2025) — Bugdar: AI-Augmented Secure Code Review for GitHub PRs. arXiv:2503.17302
- SWE-PRBench (2026) — Benchmarking AI Code Review Quality. arXiv:2603.26130
- Spieker et al. (2018) — RL for Test Case Prioritization/Selection in CI. arXiv:1811.04122
- Sharif et al. (2021) — DeepOrder: DL for Test Prioritization in CI. arXiv:2110.07443
