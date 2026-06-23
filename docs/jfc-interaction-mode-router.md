# JFC Interaction-Mode Router — Design

Status: **proposal** · Owner crate: `jfc-engine` · Audience: JFC maintainers

This document specifies a per-turn **interaction-mode router** for JFC, the one
architectural idea JFC is missing relative to JetBrains Junie ("Matterhorn").
It is grounded in JFC's existing prompt-assembly and tool-gating code, reuses
the existing per-turn classifier, and introduces no new source of truth.

---

## 1. What Junie does (the gap)

Junie classifies **every user turn** into a behavioral *interaction mode* and
then swaps both the **system prompt** and the **allowed tool set** for that turn.
Confirmed in the decompiled release (`AbstractPromptProvider.buildModeDescription`,
`PromptInteractionMode`, `ModePromptBuilder.buildDecisionTree`):

| Junie mode | Intent |
|---|---|
| `CODE` | Multi-step implementation with edits. |
| `FAST` | Quick, few-step change; minimal ceremony. |
| `CHAT` | Answer/explain; no edits. |
| `RUN` | Execute/inspect; run commands, report. |
| `BRAINSTORM` | Large new feature, requirements unclear → ask first. |
| `ADVANCED` | Long, multi-subsystem work. |
| `NICHE` / `SETUP` | Specialized/onboarding paths. |

A lightweight router (`TaskRouter` + `buildModesFromRouterSelection`) picks the
mode set; `buildModeDescription` injects the matching guidance and gates tools.

**JFC today:** `slate.rs::QueryClass::from_query` already classifies each turn —
but only to pick a **model tier** (via `SlateRouter::route`). It does *not*
change agent behavior or tool exposure. That is the whole gap: JFC has the
classifier and the gating machinery, but they are not wired to a behavior layer.

---

## 2. JFC's existing machinery (the integration surface)

Everything needed already exists and is single-owner. Verified call sites:

### 2.1 Per-turn classifier — `crates/jfc-engine/src/slate.rs`
`QueryClass::from_query(text) -> QueryClass` is a purely lexical O(n) classifier
already returning `Trivial | Exploration | CodeEdit | Refactor | Research |
LongContext`. The public `SlateRouter::route` / `route_explained` map a class →
model internally (the class→model step `route_class` is a private detail — *not*
an extension point). We reuse only the public **classifier** `QueryClass::from_query`
and add an independent class → **mode** projection. No new NLP, no LLM call.

### 2.2 System-prompt assembly — `crates/jfc-engine/src/stream/request/prepare.rs`
`prepare_stream_request()` is the single owner that builds `system_prompt`:
`build_prompt_seed()` → `append_project_context()` →
`append_runtime_prompt_sections()` → `append_turn_prompt_sections()` →
brief-mode block (`prepare.rs:85-100`). **A mode section is appended here**,
right before `prepare_advertised_tools` (`prepare.rs:101`), exactly mirroring how
`brief_mode` already injects a `## Brief User Messages` block.

### 2.3 Tool gating — `crates/jfc-engine/src/stream/request/tool_catalog.rs`
`prepare_advertised_tools()` is the single owner of "what tools does the model
see this turn". It already layers: progressive selection
(`tools::progressive_tool_defs`), `allowed_tools` allowlist (which **short-
circuits** progressive selection, `tool_catalog.rs:26-39`), `disallowed_tools`
denylist, permission-automation deny, and the non-action read-only fallback
(`preserve_non_action_tool`, `tool_catalog.rs:160-169`). The interaction mode
adds at most a *prompt section* here; it deliberately does **not** add a second
read-only tool stripper — that job belongs to `PermissionMode::Plan` (§2.6).

### 2.4 Per-turn override carrier — `crates/jfc-engine/src/runtime/events.rs`
`StreamRequestOverrides` (events.rs:174) already carries `brief_mode`,
`allowed_tools`, `disallowed_tools`, `tool_choice`. It is built once per turn in
`stream/continuation.rs:256`. **The selected mode rides here** as one new field —
no new plumbing, no second state owner.

### 2.5 State owner — `EngineState`
The user's explicit mode (sticky toggle) lives on `EngineState` next to the
existing `brief_mode: bool` and `permission_mode: PermissionMode`
(`app/engine_state.rs:723`), and is copied into `StreamRequestOverrides` at
`continuation.rs:256` like every other per-turn flag.

### 2.6 PRE-EXISTING read-only mode — `PermissionMode` (do **not** duplicate)
**Critical:** JFC already has a sticky, `Shift+Tab`-cycled, status-badged mode
enum on `EngineState`: `PermissionMode` (`app/permissions.rs:6`) with variants
`Default | Plan | AcceptEdits | BypassPermissions | Auto`. `PermissionMode::Plan`
is *already* the read-only mode: at **execution** time `auto_approves()`
(`app/permissions.rs:101-189`) approves a broad read/no-mutation allow-list
(Read/Glob/Grep/Search/Lsp/WebFetch/WebSearch/Advisor/Task*/ToolSearch/CodeGraph
MCP/AskUserQuestion/…) plus read-only Bash, and **denies** Write/Edit/ApplyPatch.
It also already owns the `Shift+Tab` cycle (`next()`), a label, and a `📋` symbol.

**Caveat (don't overstate it):** Plan is "no file mutation," not a hard sandbox.
It approves `EnterPlanMode`/`ExitPlanMode` (`permissions.rs:130-139`) — by design
`ExitPlanMode` is the *only* way the agent leaves Plan — and it approves
read-only Bash. So "Chat delegates read-only to Plan" means "the agent won't
edit files," not "the agent is incapable of mutation": a model that calls
`ExitPlanMode` leaves Plan. The §7 auto-enter option must account for this
(the mode hint should tell the model to stay in read-only, and `ExitPlanMode`
from a `Chat`-driven Plan should re-prompt the user, not silently free the agent).

This is the single biggest constraint on the design: the interaction mode is
about **behavioral guidance + which tools are advertised** (intent shaping),
while `PermissionMode` is about **whether an advertised tool may execute**
(safety gating). They are orthogonal axes and must compose, not collide. The
read-only behavior a "Chat" mode wants is **already implemented** by
`PermissionMode::Plan` — so interaction-mode must *delegate* read-only
enforcement to it rather than add a second, advertise-time read-only stripper.

---

## 3. Proposed design

### 3.1 New type — `InteractionMode` (`crates/jfc-engine/src/interaction_mode.rs`)

```rust
/// Behavioral guidance for a single agent turn — an *intent-shaping* layer.
/// Orthogonal to two existing axes it must NOT duplicate:
///   • `slate::QueryClass` picks the *model* (cost/quality tier).
///   • `PermissionMode` gates whether an advertised tool may *execute* (safety).
/// `InteractionMode` only shapes the *system-prompt guidance* for the turn and,
/// for `Chat`, defers read-only enforcement to `PermissionMode::Plan`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionMode {
    /// Implement: multi-step edits expected. Default — emits no extra guidance.
    #[default]
    Code,
    /// Quick edit: prompt nudges toward the smallest correct, few-step change.
    Fast,
    /// Answer/explain: prompt says "don't edit this turn". Read-only ENFORCEMENT
    /// is delegated to `PermissionMode::Plan` (see §3.4), not re-implemented.
    Chat,
    /// Explore the unknown: ask clarifying questions before large new features.
    Brainstorm,
}
```

Four modes, not Junie's eight: JFC already owns `RUN` behavior via Bash + read-
only-bash auto-approve, and `NICHE`/`SETUP`/`ADVANCED` are Junie-internal
onboarding paths JFC covers with skills/subagents. **Scope boundary (per
`.claude/rules/scope-boundaries.md`):** this is for the *main interactive agent
only* — subagents keep their own prompt/tool policy (`jfc-agents`), and this must
not become a general behavior framework. Start at four; add a mode only when a
concrete turn type is demonstrably mishandled (eval-driven, §5).

### 3.2 Selection — ONE classifier, projected; resolved once per USER turn

There is exactly **one** lexical classifier in the system — `slate::QueryClass`.
`InteractionMode` is a *projection* of that single classification, never a second
heuristic. This is the explicit answer to "one classifier or two": one. If
`QueryClass` is later replaced (e.g. an LLM router), the mode projection rides
along for free.

```rust
impl InteractionMode {
    /// PROJECT the existing single classification → a default behavior mode.
    /// Takes the already-computed class so we never run a second classifier.
    pub fn from_class(class: slate::QueryClass) -> Self {
        use slate::QueryClass::*;
        match class {
            Trivial | Exploration | Research => Self::Chat,
            CodeEdit => Self::Fast,
            Refactor | LongContext => Self::Code,
        }
    }

    /// Resolve the effective mode: an explicit sticky toggle always wins;
    /// otherwise project the (already-computed) class for this user turn.
    pub fn resolve(explicit: Option<Self>, class: slate::QueryClass) -> Self {
        explicit.unwrap_or_else(|| Self::from_class(class))
    }
}
```

**When is this resolved? Once per *user* turn — NOT per request.** This is
load-bearing (advisor caught it): `prepare_stream_request` /
`continuation.rs:256` run on *every* continuation, including post-tool resumes
whose trailing message is a synthetic `tool_result` (not user intent).
Reclassifying there would let a multi-step `Code` task flip to `Chat` mid-loop.
So:

- The **effective mode is computed when the user submits a prompt** (in the
  submit path, where `QueryClass::from_query(user_text)` is already/also used for
  model routing) and **stored on `EngineState`** as `active_interaction_mode`.
- `continuation.rs:256` then only **copies** `state.active_interaction_mode` into
  `StreamRequestOverrides` — it does **not** classify. Continuations inherit the
  user turn's mode unchanged, so a tool loop can never re-mode itself.
- A new user prompt recomputes it (explicit toggle still wins).

- **Explicit** sticky toggle: a `/mode` slash command or `Shift+Tab`-adjacent
  control, stored on `EngineState`, exactly like `permission_mode`. `None` ⇒ infer.
- **Inferred** reuses `slate::QueryClass` — zero new classifier surface.
- **Default** is `Code` (today's behavior): with no toggle and the `Code`
  projection emitting no prompt text and no tool change, output is byte-identical
  to current. The feature is a strict superset.

### 3.3 Prompt section — `interaction_mode.rs::prompt_section`

Each mode returns a short prompt fragment, mirroring `personas.rs::Persona::prompt`
and the existing brief-mode block at `prepare.rs:85-100`. Appended in `prepare.rs`
immediately after that block (≈line 100), before `prepare_advertised_tools`:

```rust
// prepare.rs, after the brief-mode block, before prepare_advertised_tools:
if let Some(section) = overrides.interaction_mode.prompt_section() {
    system_prompt.push_str("\n\n");
    system_prompt.push_str(section);
}
```

This is the mode's **only** mutation of the request — pure additive prompt text,
exactly like brief mode. No tool list is touched here.

Example fragments (final wording lives in code, eval-gated):
- `Chat`: "Answer and explain. Do not modify files this turn; use read-only
  navigation. If the user clearly wants a change, say so and offer to switch."
- `Fast`: "Make the smallest correct change. Prefer one focused edit; skip
  refactors and scope expansion unless asked."
- `Brainstorm`: "Requirements may be incomplete. Before large new work, ask up
  to 3 clarifying questions (use AskUserQuestion). Do not scaffold yet."
- `Code`: empty (default behavior; no extra text → no token cost).

### 3.4 Read-only enforcement — DELEGATE to `PermissionMode::Plan`, don't re-add

The first draft of this design added a parallel read-only `retain` in
`tool_catalog.rs`. **That was wrong** — it would be a third overlapping
read-only mechanism beside `PermissionMode::Plan` (execution-time gating) and the
non-action `preserve_non_action_tool` fallback. Per
`.claude/rules/architecture.md` ("do not introduce a second source of truth")
the design instead **composes**:

- **`InteractionMode` never strips tools itself.** Its only effect on the catalog
  is via the prompt section (§3.3) telling the model not to edit this turn.
- **Read-only ENFORCEMENT is `PermissionMode::Plan`**, which already blocks
  write/exec at execution time (`app/permissions.rs:101`). The product behavior
  "I asked a question, the agent shouldn't edit" is delivered by being in
  `Plan` permission mode, which already exists and already has the `Shift+Tab`
  cycle and `📋` badge.
- **The link between them:** selecting `Chat` may *suggest* switching to `Plan`
  permission mode (a one-line status hint), and JFC already has precedent for
  auto-switching permission mode by intent — `runtime/ops.rs` auto-switches into
  `Plan` for `Intent::AutoPlanModeRequest` when auto-plan is enabled
  (`runtime/ops.rs:867`). `Chat` can reuse
  that exact path rather than inventing gating. Whether `Chat` *auto*-enters
  `Plan` or merely *recommends* it is the one open product decision (§7), not an
  architectural one.

This means **§2.3's tool pipeline is touched only to append a prompt section**,
never to add a mode-specific `retain`. The mid-tool-loop hazard from the first
draft disappears entirely, because nothing new strips tools.

**Allowlist precedence (advisor #3 — a security boundary):** when
`overrides.allowed_tools` is non-empty it already short-circuits progressive
selection (`tool_catalog.rs:26-39`) and is applied as a hard filter (`:90-120`).
The interaction mode is **prompt-only** and changes none of that, so a
managed/user allowlist *always* wins over any mode guidance — stated explicitly
so a future edit can't quietly let a mode re-expand a locked-down catalog.

### 3.5 Wiring (the only edits)

1. `interaction_mode.rs` — new file (type + `from_class` + `resolve` +
   `prompt_section`). ~100 lines incl. tests. No tool/`retain` logic.
2. `runtime/events.rs` — add `pub interaction_mode: InteractionMode` to
   `StreamRequestOverrides` (defaults to `Code`).
3. `app/engine_state.rs` — add two fields next to `permission_mode`:
   `pub interaction_mode: Option<InteractionMode>` (sticky explicit toggle;
   `None` = infer) and `pub active_interaction_mode: InteractionMode` (the
   resolved mode for the current user turn, held across its continuations).
4. **Submit path** (where the user prompt is accepted and `QueryClass::from_query`
   already runs for model routing) — set
   `state.active_interaction_mode = InteractionMode::resolve(state.interaction_mode, class)`.
5. `stream/continuation.rs:256` — copy (do **not** classify):
   `interaction_mode: state.active_interaction_mode`.
6. `stream/request/prepare.rs:~100` — append `prompt_section()`.
7. `commands/` — `/mode [code|fast|chat|brainstorm]` slash command + status-row
   indicator (mirror `/brave` and the existing `permission_mode` badge).

`tool_catalog.rs` is **not** edited (read-only is `PermissionMode`'s job).
No other files change. No struct is split; no behavior leaves its owner.

---

## 4. State ownership & integration checks (per `.claude/rules/architecture.md`)

- **Single source of truth:** the *explicit* mode lives only on `EngineState`;
  the *active* mode is resolved once per user turn (also on `EngineState`) and
  merely copied into `StreamRequestOverrides`. The prompt section is a pure
  function of that one value. Read-only state has exactly one owner —
  `PermissionMode` — which this design reuses, not duplicates.
- **No god object:** `InteractionMode` owns only its prompt projection in its own
  module; `prepare.rs` *calls* it exactly as it calls brief-mode today.
  `tool_catalog.rs` is untouched.
- **Orthogonal axes, explicitly:** model tier (`QueryClass`→`SlateRouter`),
  execution gating (`PermissionMode`), and turn guidance (`InteractionMode`) are
  three independent dials. The doc names all three so a future contributor can't
  collapse them by accident.
- **Refresh point:** active mode is set at user-prompt submit and held for that
  turn's continuations, so a tool loop cannot re-mode itself; the next user
  prompt recomputes it. Explicit toggle always wins.
- **Integration combinations to test:**
  - `Chat` + a managed `allowed_tools` allowlist → allowlist still wins (mode is
    prompt-only, changes no tool filtering).
  - `Chat` turn then `Code` next turn → guidance does not leak (active mode is
    recomputed per user prompt).
  - Mid-tool-loop continuation → unaffected (no new tool stripping exists; the
    existing `action_expected` path is unchanged).
  - `Chat` + `PermissionMode` interplay → if `Chat` recommends/auto-enters
    `Plan`, verify a subsequent edit attempt is blocked by `Plan`, not by the
    mode; if it only recommends, verify edits still gate normally.
  - `brief_mode` + any interaction mode (independent flags, both append sections).
  - Subagent turns: unaffected (they don't build `StreamRequestOverrides` here).

---

## 5. Eval coverage (per `.claude/rules/testing.md` — prompt/policy changes need evals)

Unit (`interaction_mode.rs`):
- `from_class_*`: each `QueryClass` → expected mode (`*_normal`).
- `resolve_explicit_wins_over_inferred_normal`; `resolve_defaults_to_code_robust`.
- `code_mode_prompt_section_is_empty_regression` (guards the zero-token-cost default).

Integration (`stream/request/` tests):
- `code_mode_request_matches_default_byte_for_byte_regression` — with mode `Code`,
  the assembled `system_prompt` **and** advertised tool list are identical to
  today's (the strict-superset guarantee; this is the most important test).
- `mode_section_appended_after_brief_block_normal` — ordering vs brief mode.
- `mode_is_prompt_only_tool_catalog_unchanged_normal` — `Chat` vs `Code` produce
  the **same** advertised tool set (proves read-only is delegated, not duplicated).
- `active_mode_held_across_continuation_regression` — a continuation copies
  `state.active_interaction_mode` and does not reclassify.

Behavioral eval (extend existing agent-eval harness, `docs/AGENT_EVALS.md`):
- A "explain how X works" prompt in `Chat` makes **no** edit tool calls.
- A "fix the typo" prompt in `Fast` makes exactly one edit and no refactor.

**Baseline comparison (advisor #4 — required before shipping, not just unit tests):**
The premise "mode-switching beats JFC's current progressive-disclosure +
`brief_mode`" is an *empirical claim and must be measured*, not assumed. Gate the
feature on an A/B over the existing eval suite:
- **A = today** (no interaction mode); **B = mode router on**.
- Metrics per scenario set: unwanted-edit rate on question prompts, clarifying-
  question rate on under-specified prompts, edits-per-task on "small fix" prompts,
  and total tokens.
- **Ship only if B shows a measurable win on at least one target metric with no
  regression on `Code`-class tasks.** If B ≈ A, the honest conclusion is that
  JFC's existing progressive disclosure already covers this gap and the router is
  not worth its complexity — record that and stop. This is the design's
  falsifiable exit criterion.

---

## 6. Why not just use subagents / skills?

Subagents (`jfc-agent`) already scope tools+prompt, but only via *delegation*
into a separate context — too heavy for "this one turn is a question, not a
change". Skills are content, not behavior gating. `PermissionMode` gates
execution (safety), not intent. The interaction mode is the missing
**lightweight, same-context, per-turn prompt-guidance** layer — the niche Junie's
`PromptInteractionMode` fills. It *complements* all three (subagents, skills,
`PermissionMode`) and duplicates none: it adds prompt text and, optionally,
recommends an existing `PermissionMode`.

---

## 7. Open decision (the one genuine fork) + out of scope

**Open product decision — `Chat` ↔ `PermissionMode::Plan` coupling.** Two options,
both architecturally clean (the doc does not pre-decide; needs a maintainer call):
1. **Recommend** — `Chat` only emits the "don't edit this turn" prompt section and
   shows a hint; the user stays responsible for `Plan`. Lowest surprise, but a
   model can still edit if it ignores the guidance.
2. **Auto-enter** — selecting `Chat` flips `permission_mode` to `Plan` (reusing
   the existing `runtime/ops.rs:867` auto-switch path), and leaving `Chat`
   restores the prior mode. Stronger guarantee, but couples two user-visible
   dials, and is **not airtight**: Plan approves `ExitPlanMode`
   (`permissions.rs:139`), so a model can still leave read-only by calling it.
   For (2) to mean what users expect, an `ExitPlanMode` call that originated from
   a `Chat`-driven auto-Plan should re-prompt the user rather than silently
   freeing edits. Recommended default: **(2)** with that guard, because "I asked
   a question and it edited anyway" is the failure mode users complain about — but
   this is a UX call, and the doc does not pre-decide it.

**Out of scope (explicitly):**
- Junie's `RUN`/`SETUP`/`NICHE`/`ADVANCED` modes (covered by Bash/skills/subagents).
- Live-debugger control, mobile-device tooling, IDE/ACP embedding (separate gaps).
- Any change to subagent prompt/tool policy, `PermissionMode`'s gating table, or
  `SlateRouter` model selection.
- LLM-based mode classification (start lexical; revisit only if the §5 baseline
  shows the lexical projection mis-modes real turns).

---

## 8. Estimated effort

~1 new module (~100 lines) + 6 small edits (events/state/submit/continuation/
prepare/commands — `tool_catalog.rs` untouched) + ~10 tests, plus the §5 baseline
A/B run. No migration, no persisted-format change (the active mode is transient;
the sticky toggle is in-memory like `permission_mode`). Low blast radius: every
change is additive behind a `Code` default, and the most important test
(`code_mode_request_matches_default_byte_for_byte`) proves that.

## 9. Recommendation

The integration is clean and cheap, but **the design's own §5 exit criterion is
the point**: build B behind a flag, run the A/B, and ship only on a measured win.
The realistic risk — flagged honestly — is that JFC's existing progressive
disclosure + `brief_mode` + `PermissionMode::Plan` already cover most of what
Junie's modes buy, in which case the right outcome is to *not* ship the router and
instead spend the effort on a gap JFC genuinely lacks (live-debugger control, or
the SearchReplace fuzzy-correction cascade). Implementation is a **separate,
user-authorized step** — this touches a security-adjacent request path, so it
should not begin without a go-ahead.
