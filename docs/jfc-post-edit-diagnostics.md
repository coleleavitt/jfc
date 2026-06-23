# JFC Post-Edit Diagnostics Loop — Design

Status: **proposal** · Owner crate: `jfc-engine` · Audience: JFC maintainers

This document scopes the closest remaining behavioral gap between JFC and
JetBrains Junie ("Matterhorn"): Junie's **`ErrorCheckingService`**, which runs a
compiler/linter *automatically after each edit* and feeds the diagnostics back to
the agent so it self-corrects before moving on.

> **Read this first — the gap is ~80% already closed.** The honest finding from
> the investigation is that JFC already has a cargo-check producer, a global
> diagnostics snapshot, an LSP push path, diagnostics **already injected into the
> system prompt every turn**, *and* a post-edit guard pipeline that returns
> findings to the model inline. So this is **not** a new subsystem. The precise
> remaining delta is exactly two things:
> 1. **Timing of refresh** — diagnostics are recomputed on startup and on user
>    submit (`/check` + auto), **not after the agent's own edits within a turn**.
> 2. **No diagnostics member in the post-edit guard** — the pipeline that *does*
>    run after each edit checks slop/wiring, not "does this still compile."
>
> If you only read one section, read **§3 (precise delta)** and **§6 (the timing
> fork)** — and seriously weigh the §9 "maybe don't build it" exit. The value
> here is small and bounded; the risk is wrecking interactive latency by putting
> a compiler on the edit hot path. The design exists to make that trade explicit,
> not to assume the feature is worth shipping.

---

## 1. What Junie does (the gap)

After the agent edits a file, Junie's `ErrorCheckingService` (decompiled:
`agent/checking/`, `ErrorCheckResult`, `ErrorCheckTimeout`, `ErrorFormatter`,
`LineColumnError`/`OffsetError`) compiles/parses the changed file and returns
structured errors *into the agent's loop*, so the next step sees "you just
introduced E0432 at line 12" without the model having to think to ask. It is a
closed **edit → diagnose → fix** loop with a bounded timeout.

---

## 2. What JFC already has (verified)

The investigation found JFC is much closer than "no diagnostics loop":

### 2.1 A cargo-check producer — `crates/jfc-engine/src/diagnostics_producer.rs`
`run_once(cwd, tx)` spawns `cargo check --message-format=json`, parses each
`compiler-message` into a `DiagnosticEntry`, and emits
`ProviderEvent::DiagnosticsUpdated`. Already triggered:
- once on startup (`runtime/event_loop/mod.rs:757`, gated by `JFC_DISABLE_CARGO_CHECK`),
- on the `/check` slash command (`input/submit.rs:140`).

### 2.2 A global diagnostics cache — `crates/jfc-engine/src/diagnostics.rs`
`set_global_snapshot()` / `global_snapshot()` hold the latest entries;
`render_for_prompt()` formats them; `format_entry()` / `format_summary()` render
the UI row. There is also an LSP push path (`lsp_client.rs` →
`textDocument/publishDiagnostics` → snapshot).

### 2.3 Diagnostics are ALREADY injected into the system prompt
`stream/request/prompt_seed.rs:37-39,126` builds a `## Current diagnostics`
block from `global_snapshot()` on every request. So the model *does* see
diagnostics — but only whatever snapshot existed **when the turn started**.

### 2.4 A post-edit guard pipeline — `crates/jfc-engine/src/guards.rs`
`GuardPipeline` runs `Guard` impls over a `GuardContext { file_path,
new_content, old_content, cwd }` after every Write/Edit, via
`maybe_run_slop_guard` (`tools/safe_tools.rs`) called from
`tools/dispatch.rs:269,300,832`. Findings are appended to the tool's
`ExecutionResult.output` — i.e. **straight back to the model on the same turn**.
It is already timeout-bounded (2s) and panic-isolated. Members today: `SlopGuard`
(quality/slop), `WiringGuard` (advertise/dispatch wiring).

### 2.5 A model-invoked LSP tool — `crates/jfc-engine/src/tools/lsp.rs`
`execute_lsp("diagnostics"|"hover"|…)` can pull diagnostics on demand, but the
model must *choose* to call it.

---

## 3. The actual gap (precise)

The loop is open at exactly one seam: **nothing re-checks the file the agent just
edited and feeds fresh errors back on the same step.**

- The prompt-seed diagnostics (§2.3) are a **stale snapshot** from turn start —
  they never reflect the edit the model made *this* turn.
- The post-edit guard pipeline (§2.4) is the right vehicle and runs at the right
  time, but it has **no diagnostics guard** — it checks slop and wiring, not
  "does this still compile / parse."
- The cargo-check producer (§2.1) only runs on startup and `/check`, **not after
  an edit**, and its whole-workspace `cargo check` (seconds) is far too slow to
  block an edit's `ExecutionResult` (the guard budget is 2s).

So: JFC sees diagnostics *between* turns, but an agent can make three edits in
one turn that each break compilation and not learn until the next user submit.
Junie closes that within the edit step.

---

### 3.1 The hard constraint that shapes everything: `cargo check` ≫ the guard budget

The post-edit guard pipeline is **synchronous** — `maybe_run_slop_guard`
(`tools/safe_tools.rs:321`) wraps the whole run in a **2-second timeout** and
appends findings to the edit's `ExecutionResult` *before the model continues*. A
full `cargo check` (the only real type-checker JFC has) takes **seconds to
minutes**, not 2s. Junie's per-edit checker is incremental IDE diagnostics, not a
full build — JFC has no equivalent incremental type-checker.

**Therefore the per-edit guard can never run a real type-check.** Any design that
puts `cargo check` on the synchronous edit path is unbuildable (it would either
time out and surface nothing, or destroy interactive latency). This single fact
forces the split in §6: cheap/instant work goes in the synchronous guard;
anything that compiles goes on the existing async, between-turns path.

## 4. Proposed design — a `DiagnosticsGuard` in the existing pipeline

The whole feature is **one new `Guard` impl** plus a fast, file-scoped checker.
No new subsystem, no new state owner, no new trigger path.

### 4.1 New guard — `crates/jfc-engine/src/guards.rs` (+ a `diagnostics_check` helper)

```rust
/// Fast, file-scoped syntax/type check on the just-edited file. Unlike the
/// whole-workspace cargo-check producer (seconds), this must fit the pipeline's
/// 2s budget, so it runs the cheapest check that catches the common breakage:
///   - Rust:  `cargo check` is too slow → use a syntax/parse check (syn) or a
///            single-file `rustc --emit=metadata -Zparse-only`-style probe.
///   - Others: only when a sub-2s single-file checker is configured.
/// Returns findings in the shared `SlopFinding` shape so the pipeline merges
/// and surfaces them exactly like slop/wiring findings (no new output path).
pub struct DiagnosticsGuard;

#[async_trait::async_trait]
impl Guard for DiagnosticsGuard {
    fn name(&self) -> &'static str { "diagnostics" }

    async fn check(&self, ctx: &GuardContext<'_>) -> Vec<SlopFinding> {
        // 1. Cheapest: consult the EXISTING global snapshot for entries in this
        //    file (already-known errors), so we surface them inline on the edit
        //    even if the model didn't open the diagnostics tool.
        // 2. Optional (behind a flag): run a sub-budget single-file parse/type
        //    probe for the file's language and convert errors to findings.
        // Both reuse diagnostics::format_entry / SlopFinding — no new format.
    }
}
```

Wire it into the default pipeline beside the others:

```rust
// guards.rs::with_default_guards()
Self::new()
    .with_guard(Box::new(SlopGuard))
    .with_guard(Box::new(WiringGuard))
    .with_guard(Box::new(DiagnosticsGuard)) // new
```

That is the entire integration: because `maybe_run_slop_guard` already runs the
pipeline after Write/Edit/ApplyPatch and appends to `ExecutionResult.output`, the
diagnostics land back in the same tool result the model reads next — the closed
loop, with zero new plumbing.

### 4.2 Make the cargo-check producer *also* fire after a settled edit (optional, phase 3)

For the full-fidelity (cross-file) check Junie does, additionally debounce-trigger
the existing `diagnostics_producer::run_once` after an edit settles, so the
*next* turn's prompt-seed snapshot (§2.3) is fresh. This reuses the existing
producer and the existing `DiagnosticsUpdated` event — no new machinery — and
keeps the slow whole-workspace check **off** the per-edit hot path. Debounce
(e.g. 1.5s after the last edit in a burst) so a 5-edit turn triggers one check,
not five.

---

## 5. State ownership & scope (per `.claude/rules/architecture.md`)

- **No second source of truth:** diagnostics continue to live only in
  `diagnostics::global_snapshot()`. The guard *reads* it (and optionally a
  one-shot probe); it never stores a parallel copy.
- **No god object:** `DiagnosticsGuard` owns only its check, like `SlopGuard` /
  `WiringGuard`. `guards.rs` orchestration and `dispatch.rs` are untouched beyond
  registering one guard.
- **Hot-path safety:** the per-edit guard must stay inside the existing 2s
  pipeline budget (it's already timeout-wrapped in `maybe_run_slop_guard`). The
  slow whole-workspace `cargo check` stays on the existing startup/`/check`/
  debounced-after-edit paths, never blocking an `ExecutionResult`.
- **Scope boundary (per `.claude/rules/scope-boundaries.md`):** this is for the
  *main interactive agent's* edit tools. It does not add a new tool the model
  calls, does not change `tools/lsp.rs`, and does not turn the guard framework
  into a general task runner. Languages beyond Rust are opt-in and gated on a
  configured sub-2s single-file checker — no shelling out to arbitrary build
  systems on the edit path.

---

## 6. The timing fork — RESOLVED (the only real design decision)

The advisor's sharpest point: the two existing mechanisms have **incompatible
timing models**, and "post-edit diagnostics" must pick one. They are:

- **Synchronous guard** (`maybe_run_slop_guard`, 2s budget, inline to the edit's
  `ExecutionResult`) — fast, same-turn feedback, but **cannot run a compiler** (§3.1).
- **Async producer** (`diagnostics_producer::run_once` → `DiagnosticsUpdated` →
  snapshot → next turn's prompt seed) — can run a real `cargo check`, but the
  feedback lands **one turn later**.

This design does **not** try to merge them. It assigns each its tractable half:

| Work | Cost | Path | When the model sees it |
|---|---|---|---|
| Surface *known* snapshot errors for the edited file | ~0 | **sync guard** | same turn, inline |
| Single-file **syntax** parse (`syn`, Rust) | sub-ms | **sync guard** | same turn, inline |
| Cross-file **type** check (`cargo check`) | seconds | **async producer**, debounced after an edit burst | next turn, prompt seed |

**Decision: do NOT put any compile/type-check on the synchronous path.** The sync
guard is strictly cheap-and-instant (snapshot lookup + optional syntax parse);
real type-checking stays async. This is the only timing model that respects both
the 2s budget and interactive latency. A maintainer may still choose to ship
*only* the sync half (phases 1–2) and never wire phase 3.

### Sequencing
1. **Phase 1 — snapshot-only sync guard.** Surface already-known
   `global_snapshot()` entries for the edited file inline. Near-zero latency, no
   new process. Closes "the model didn't notice an existing error in this file."
2. **Phase 2 — + single-file syntax probe (Rust, `syn`).** Catches *newly
   introduced* syntax breakage on the same turn. Still no type-checking.
3. **Phase 3 (optional) — debounced async `cargo check` after an edit burst** so
   the *next* turn's prompt seed reflects the agent's own edits. Reuses the
   existing producer + event entirely; the only new code is a debounce trigger on
   the edit path. Off the hot path by construction.

Ship 1 behind a flag, measure (§7), add 2 only if newly-introduced syntax errors
are a demonstrated failure mode, and treat 3 as a separate change with its own
go-ahead.

---

## 7. Eval coverage (per `.claude/rules/testing.md`)

Unit (`guards.rs`):
- `diagnostics_guard_surfaces_known_snapshot_entry_for_edited_file_normal`.
- `diagnostics_guard_ignores_entries_for_other_files_robust`.
- `diagnostics_guard_empty_snapshot_is_silent_robust` (no findings ⇒ no output noise).
- `diagnostics_guard_respects_budget_regression` (returns within the pipeline timeout).
- If phase-2 syntax probe lands: `diagnostics_guard_flags_introduced_syntax_error_normal`,
  `diagnostics_guard_passes_valid_edit_robust`.

Integration (`tools/` dispatch tests):
- `edit_with_existing_error_in_file_appends_diagnostic_to_result_normal` — the
  `ExecutionResult.output` carries the diagnostic after an Edit.
- `clean_edit_adds_no_diagnostic_noise_regression`.

Behavioral eval (extend `docs/AGENT_EVALS.md`):
- An edit that introduces a compile error is followed by a corrective edit in the
  same turn (the loop actually closes), measured as a **baseline A/B**: A = today
  (diagnostics only between turns), B = guard on. Ship only if B reduces
  "left a broken edit at end of turn" without a latency regression on clean edits.
  If B ≈ A, record that the between-turn snapshot already suffices and stop.

---

## 8. Why this is the right next gap (vs. the alternatives)

From the broader Junie comparison, the other candidates are either already
covered or out of scope for a TUI:
- **Interaction-mode router** — designed separately (`jfc-interaction-mode-router.md`),
  but its own exit criterion flags it may be redundant with existing progressive
  disclosure.
- **Next-edit/typeahead prediction, IDE chronicles, ACP embedding** — IDE-inline
  or editor-coupled; out of scope for JFC's terminal/web surface.
- **Post-edit diagnostics loop (this doc)** — bounded, hermetically testable,
  reuses 4 existing subsystems, and directly complements the SearchReplace
  fuzzy-edit cascade already added (that makes edits *land*; this makes sure they
  *compile*). Highest value-to-risk of the remaining gaps.

## 9. Estimated effort & recommendation

Phase 1 (snapshot-only guard): ~1 `Guard` impl + registration + ~6 tests. Tiny,
additive, behind a flag, no hot-path risk. Phase 2 (debounced re-check) is a
separate, larger change touching the event loop's edit path.

**Recommendation — including the honest "maybe don't":** the gap this closes is
narrow (diagnostics already reach the model every turn; this only makes them
*same-turn* and adds a syntax check). Before building, weigh that JFC's
between-turns snapshot may already be good enough. Concretely: implement **phase 1
behind a flag**, run the §7 baseline A/B, and **ship only on a measured reduction
in "left a broken edit at end of turn" with no latency regression.** If the A/B
shows the between-turns snapshot already catches these, the correct outcome is to
*not* ship and record that JFC had effectively closed this gap already. As with
the mode router, implementation is a **separate, user-authorized step** — it
touches the runtime edit path, so it should not begin without a go-ahead.
