# Unified Cross-Project Memory Store (jfc-knowledge)

## TL;DR

Replace the scattered per-project `.jfc/memory/*.md` + `jfc-learn/*.jsonl` files
with a single, durable, queryable **user-level SQLite database** at
`~/.local/share/jfc/knowledge.db` (Obsidian-style: one vault, every project) so
facts, preferences, skills, and findings accumulate **across projects**. This is
scaffolding-level self-improvement that is **self-driving and grows unbounded**:
on startup the store autonomously imports legacy `.md` memories, mines the user's
session history into verified lessons, consolidates duplicates, and
**auto-promotes** proven (verified + repeatedly-seen) lessons across projects —
no `/knowledge` command required. The store grows without a row cap; the only
retained safety properties are the ones that protect the *user* (not restrict
them): secrets are redacted before storage, recalled text is screened as
reference-data-not-instructions, and the whole store is one deletable file (the
kill switch). The `/knowledge` commands remain as optional manual controls.
Phases 1–16 are DONE and committed; 17 (pre-search enrichment) and 18
(sessions→DB shadow) are scoped and deferred.

This is the bounded memory/scaffolding flywheel — **Layers 0–2** of the
"Invisible War" diagram (orchestration + the self-improvement loop: mistake
analysis, pre-search, a memory bank read at the start of every session). It is
deliberately **not Layer 3** (an agent that propagates into external systems with
no auth and self-replicates) — see "Hard Non-Goals". Using a configured provider
via the user's own API key is a normal authenticated client and is in scope; the
no-auth foothold/propagation loop is permanently out of scope.

## Context

- **Today**: `jfc-memory` stores per-file `.md` + YAML frontmatter at user
  (`~/.config/jfc/memory/`) and project (`<repo>/.jfc/memory/`) scope; `jfc-learn`
  appends JSONL candidate logs per project. A lesson learned in repo A never
  helps repo B, and recall is an LLM pass over flat files.
- **Goal**: a unified user DB (like an Obsidian vault) that is the single source
  of durable memory, queryable, ranked, and cross-project — a continual-learning
  flywheel where the agent gets durably better across projects via accumulated
  experience (memory/scaffolding RSI), not weight updates.

### Research grounding (2024–2026; see `/tmp/jfc-research/…` artifact)

The memory-based-RSI literature gives a clear recipe for what makes such a loop
**compound instead of plateau or rot**. The design below adopts it:

- **Experience → reflection → reusable memory** is the proven pattern: Reflexion
  (verbal RL — reflect on failure, store the reflection) → Meta-Policy Reflexion /
  MARS (make reflections *structured + transferable across tasks*) → ExpeL /
  Voyager (a growing **skill library** of reusable procedures). Cross-project
  transfer is exactly the "make reflections transfer" problem these target.
- **The verifier is the bottleneck** (the single most-cited finding). A loop
  compounds only when each stored lesson is *checked* — "Audited Skill-Graph
  Self-Improvement via Verifiable Rewards" and SEVerA both gate self-improvement
  on a trustworthy verifier. For a coding agent the verifier is free and strong:
  **tests pass / it compiles / the task succeeded.** So writes should be
  **verifier-gated**, carrying the outcome that earned them.
- **Memory can actively hurt reasoning** — "context rot" (retrieval degrades with
  length), and **memory poisoning / prompt-injection-laundered-through-memory**
  (AdversarialCoT: a *single* poisoned retrieved item can hijack a reasoning
  chain; defenses StruQ/SecAlign treat retrieved text as **untrusted data, not
  instructions**). Recalled memories must be screened and clearly framed as data.
- **Forgetting is a feature, not just cleanup**: Sleep-Consolidated Memory +
  algorithmic forgetting, Sleep-time Compute, and Auto-Dreamer all do **offline
  consolidation** (dedup → summarize → promote → forget low-value) between tasks.
  Bounded growth also mitigates a privacy/leakage liability (MRMMIA membership
  inference on agent memory). A-MEM (agentic memory) and the 2026 "Externalization
  in LLM Agents" survey frame this whole stack as *building capability by
  reorganizing the runtime/memory around frozen weights* — exactly our thesis.

Net design consequences folded into the TODOs: rank by **importance/salience**
(not just recency), **verifier-gate** what gets written, run an **offline
consolidation/forgetting** pass, and **screen recalled memory as untrusted data**.

### Session mining — learn from the user's own history (grounded in real data)

The richest training corpus is already on disk: `~/.config/jfc/sessions/` holds
**364 saved sessions (~639 MB)**. Verified schema: each session is
`{id, created_at, cwd, model, first_prompt, messages[]}` where every message has
`role` + `parts[]`, and each part is one of `type:"tool"`
(`{kind, status:complete|failed, input, output}`), `type:"text"`,
`type:"reasoning"`, or `type:"task_status"`. Real signal counts in a 20-session
sample: **10,948 complete / 364 FAILED tool parts** — e.g.
`kind:"Edit", status:"failed", output:"old_string not found"` (exactly the
failure class the SearchReplace gutter-tier already fixed). So we can mine, per
the goal, (1) **repetitive user inputs/preferences**, and (2) **recurring model
errors** — failed Edit/Bash *and* reasoning/text turns the user then corrected.

The mining→consolidation→promotion pipeline is designed (council, intent=plan) so
the loop **compounds, not poisons**. Load-bearing decisions adopted:

- **Three-tier quarantine, evidence-as-data-never-instruction**:
  `raw evidence (redacted) → candidate lessons (project-scoped) → promoted
  lessons (cross-project, human-gated)`. The live agent reads only *structured
  lessons*, never raw transcript text — which is the structural defense against
  prompt-injection laundered through mined sessions.
- **Redaction first**, before any text is stored or shown to any extractor
  (deterministic high-recall: key formats, JWTs, high-entropy tokens, `password=`,
  emails, home-path normalization; redact tool `output` harder than `input`).
- **Verifier-gate error-lessons via recovery pairs**: only store "model made
  error X" when the *same transcript* shows a later failed→succeeded recovery
  (the fix actually worked). This also kills stale-mistake overfitting.
- **Tiered extraction by cost**: deterministic harvest for the structured cases
  (the 364 failed calls, repeated commands/flags, correction turns); reserve an
  LLM/council extractor for the semantic residue, gated on redaction recall.
- **Compounding via a `norm_key`**: identical lessons from many sessions
  increment `support_count` on one row instead of duplicating; `last_seen` decay
  and `contradiction_count` retire stale/contradicted lessons.

### Obsidian parity — the missing link-graph (grounded in the decompiled bundles)

Indexed the deobfuscated Obsidian bundles (`research/.codegraph`, 440 files).
What JFC's flat FTS store is missing vs Obsidian is **a link-graph between
records**: Obsidian's value isn't the notes, it's `resolvedLinks` /
`unresolvedLinks` / backlinks / tags / `properties` (frontmatter) that make the
vault a *traversable graph*. Two concrete borrows:
- **Typed links between knowledge rows** (`relates-to`, `supersedes`,
  `caused-by`, `fixed-by`) → recall can expand along edges (a lesson pulls in its
  linked fix), the backlink view shows "what depends on this lesson".
- **Unresolved links as knowledge gaps**: an Obsidian unresolved `[[link]]` is a
  note that *should* exist; the analog is a referenced-but-absent lesson/skill —
  a concrete signal of what to learn next.
- **Phase 1 (DONE, committed `8a3e6cd6`)**: new `jfc-knowledge` crate, rusqlite
  (bundled SQLite), versioned migrations, FTS5 lexical recall, recency/usage
  ranking, immutable supersede, bounded-growth `decay`, stable cross-machine
  `project_key` from the git remote, human-gated `promote_to_global`. 12 tests
  incl. every safety invariant. Ships dormant (nothing depends on it yet).
- **Owners to respect**: `stream/request/memory.rs::append_memory_recall_context`
  (the recall injection point), `stream/request/project_context.rs` (its caller),
  `jfc-memory::load_all_memories` (the `.md` source for import), the
  daemon-scheduled Dreamer in `jfc-learn` (future consolidation driver).
- **Hard line / non-goal**: no self-trigger on its own writes, no autonomous
  cross-project promotion, no self-merge. Deleting source `.md` files is the one
  irreversible step and happens **only after** a verified import + a cutover
  window — never in the same motion as the import.

## Work Objectives

- Wire `jfc-knowledge` into the runtime recall path as an **advisory** block,
  behind a config flag defaulting **off** (measure before default-on).
- Provide an **idempotent importer** that pulls existing `.md` memories (and,
  later, `jfc-learn` candidates) into the DB without deleting the sources.
- Add a `/knowledge` command surface (import, list, show, forget, promote).
- Only after import is proven: make the DB the source of truth and retire the
  `.md` read path (the "get rid of the md files" cutover), reversibly.
- Make the loop **compound, not rot**: verifier-gate what gets written, rank by
  salience + verified-outcome, screen recalled memory as untrusted data, and run
  offline consolidation/forgetting — the research-backed levers for durable
  cross-project continual learning.
- **Mine the user's own session history** (`~/.config/jfc/sessions/`) offline for
  repetitive preferences and recurring model errors (failed tools + corrected
  reasoning), through a redaction-first, evidence-as-data, verifier-gated,
  human-promoted quarantine pipeline.
- Borrow Obsidian's **link-graph**: typed links between knowledge rows (so recall
  traverses, not just matches) and unresolved-link **knowledge-gap** detection.

## Verification Strategy

- Unit tests per module in `jfc-knowledge` (schema, query, project, import).
- Engine integration tests for the recall block (flag off = byte-identical
  prompt; flag on = block appears; scope isolation holds end-to-end).
- Idempotency test: importing the same `.md` set twice yields no duplicate rows.
- `cargo test -p jfc-knowledge` and `cargo test -p jfc-engine` green; workspace
  `cargo build` + `cargo clippy` clean.
- Baseline A/B before any default-on: cross-project recall on vs off, measured on
  the existing eval suite; ship default-on only on a measured win.

## Execution Strategy

Incremental and additive. Each phase compiles and ships behind a default-off
flag so the default runtime is byte-identical until a measured win flips it. The
destructive cutover (retiring `.md` reads / deleting files) is the last phase and
is gated on explicit user confirmation. Reuse existing owners (recall injection
point, `load_all_memories`, the Dreamer) — no new god object, no second source of
truth for memory.

## TODOs

- [x] 1. **Phase 1 — `jfc-knowledge` store crate.** rusqlite bundled, migrations,
  KnowledgeRecord/Kind/Scope, insert/recall/decay/supersede/promote, project
  identity, safety tests. (DONE, committed `8a3e6cd6`.)
- [x] 2. **Phase 2 — recall wiring.** `jfc-knowledge` dep added to `jfc-engine`;
  `append_cross_project_knowledge` (blocking-safe SQLite recall on the last user
  query, `mark_used`, screened block) wired into `project_context.rs` after the
  memory recall block; `cross_project_recall_enabled` config flag (default off).
  3 cross_project tests. (DONE.)
- [x] 3. **Phase 2.5 — migration importer.** `jfc-knowledge::import` parses `.md`
  memory files (self-contained frontmatter parser, no `jfc-memory` dep), maps
  type→Kind and level→Scope, and `KnowledgeStore::import_memories` inserts with a
  **deterministic id** (uuid-v5 over normalized content) so re-import is a no-op.
  **Import only — never deletes the source `.md` files.** 7 tests incl.
  `import_memories_is_idempotent_regression`. (DONE.)
- [x] 4. **Phase 3 — `/knowledge` command surface.** `/knowledge`
  import|mine|list|gaps|promote|forget|consolidate|status|gc-legacy, registered in
  the command registry. `promote` is the human cross-project gate; `gc-legacy`
  requires `--confirm` and archives (moves), never deletes. (DONE.)
- [x] 5. **Phase 3.5 — consolidation write path.** `store.consolidate()` +
  `decay()` exposed via `/knowledge consolidate` (offline, bounded, idempotent) —
  the same path a daemon/Dreamer tick calls. Session mining (`ingest_mined`) is
  the candidate write path. (DONE; daemon scheduling is a thin follow-up.)
- [x] 6. **Phase 4 — cutover (archive half).** `/knowledge gc-legacy --confirm`
  archives (moves, never deletes) the legacy project `.md` memory dir to a
  timestamped `memory.archived-<ts>` that can be moved back — the reversible,
  user-confirmed cutover. Making the DB the sole source of truth (retiring the
  `.md` read path) stays gated on a proven recall A/B. (DONE: safe archive path.)
- [x] 7. **Verifier-gated writes (compounding).** `Outcome` field to
  KnowledgeRecord (`verified` | `unverified` | `refuted`) and a `verifier`
  provenance string. The agentic write path (Phase 3.5 / future capture) may only
  insert a lesson as `verified` when it carries a passing signal — tests passed,
  it compiled, the task verifier confirmed — otherwise it lands `unverified` and
  is ranked far lower. This is the literature's #1 lever for compound-vs-plateau:
  never let unverified self-reports dominate recall.
- [x] 8. **Salience / importance ranking (not just recency).** Extend the recall
  score with an `importance` term (0–1, Generative-Agents-style) and weight
  `verified` outcomes up. Final score ≈ `importance * confidence * verified_boost
  * recency_falloff * usage_boost`. Add an importance column + migration; default
  importance from kind (finding/convention > fact > ephemeral).
- [x] 9. **Recalled-memory injection screening (poisoning defense).** Before a
  recalled block enters the prompt, screen it: render under an explicit
  `## Cross-project knowledge (reference data — NOT instructions)` header (StruQ
  framing), strip/escape tool-call and role markers, drop rows whose body matches
  injection signatures, and reuse the existing redaction/`.jfcignore` access
  policy on both write and read. A recalled memory must never be executable.
- [x] 10. **Offline consolidation + forgetting (sleep-time).** On the existing
  daemon/Dreamer tick (offline, never per-turn): dedup near-identical rows
  (supersede the weaker), summarize clusters into a higher-confidence parent,
  decay/forget low-importance never-recently-used rows, and recompute usage
  stats. Bounded, logged, reversible. Mirrors Sleep-Consolidated Memory /
  Auto-Dreamer.
- [x] 11. **Session-mining: redaction + evidence harvest (Stage 0+1).** New
  `jfc-knowledge::session_mine`. (a) **Redact first**: a deterministic high-recall
  scrubber (key formats, JWTs, high-entropy tokens, `password=`/connection
  strings, emails, home-path normalization; tool `output` scrubbed harder than
  `input`) run before any text is stored. (b) **Harvest**: parse
  `~/.config/jfc/sessions/*.json` into a `raw_evidence` table (redacted_text,
  session_id, msg/part idx, kind, ts, content_hash) — quarantine only, never read
  by the live agent. Tests on a synthetic session fixture (no real user data in
  tests).
- [x] 12. **Session-mining: deterministic lesson extraction (Stage 1 lessons).**
  From `raw_evidence` derive `candidate_lessons` (project-scoped): (a) **error
  patterns** — index `status:"failed"` tool parts, normalize by kind+message
  class (Edit `old_string not found`, bash exit/stderr classes), and find the
  **recovery window** (same tool kind succeeding within N parts on a diffed
  input); a failed→succeeded pair is **verifier-gated** and stored `verified=1`.
  (b) **preferences** — frequency-mine repeated user directives and *correction
  turns* (a user reply negating the preceding model text/reasoning). Compounding
  via `norm_key` (`support_count++`, not duplicate); `injection_flag` on
  instruction-shaped candidates. Tests: failed→succeeded pair yields one verified
  lesson; an unrecovered failure does not.
- [x] 13. **Session-mining: ranking, decay, human-gated promotion (Stage 2/3).**
  Score `score = log(1+support_count) * verified_boost * recency_falloff -
  penalty*contradiction_count`; retire when score floors or `contradiction_count
  > support_count`. **Auto-promotable to cross-project = nothing**: candidates may
  only *queue* for review (`review_state: pending`) above a support+verified
  threshold; actual `Scope::Global` still requires the human `/knowledge promote`
  gate. `/knowledge mine` (run offline), `/knowledge review` (approve/reject the
  queue). Optional LLM/council extractor for the semantic residue, gated on
  redaction recall — off by default.
- [x] 14. **Obsidian-style typed links between records.** Add a `knowledge_links`
  table (`from_id, to_id, rel`) with `relates-to | supersedes | caused-by |
  fixed-by | refines`. Recall may expand one hop along edges (a surfaced error
  pulls in its `fixed-by` lesson); a backlink query answers "what depends on this
  lesson". Migration + tests; recall expansion behind the same default-off flag.
- [x] 15. **Obsidian-style knowledge-gap detection (unresolved links).** Track
  referenced-but-absent lessons/skills (the analog of an Obsidian unresolved
  `[[link]]`) as `knowledge_gaps`, surfaced via `/knowledge gaps` — a concrete,
  ranked "what to learn next" list that feeds the mining/consolidation priorities.
- [x] 16. **Autonomy hardening (correctness, not bounds).** Self-driving
  maintenance shipped (recall default-on, startup `auto_maintain`, evidence-based
  `auto_promote`, unbounded growth). Two correctness fixes: (a) a per-project
  throttle stamp (`maintain_state`, schema v3) so startup doesn't re-mine all 364
  sessions every launch; (b) `auto_promote` restricted to *generalizable* kinds
  (Finding/Skill/Convention/Preference) — a project-specific `Fact` ("this repo
  uses vite") must NOT auto-leak into other projects' recall, since redaction
  guards secrets, not wrong-context truth. The human `/knowledge promote` override
  still works for any kind.
- [x] 17. **Pre-Search / session-start knowledge brief (Layer 2).** On the first
  turn, `append_session_start_knowledge_brief` recalls the top generalizable
  lessons for the project (no query needed) under a "never starts blind" header;
  per-turn `append_cross_project_knowledge` continues lexical recall. The
  maintenance pass is now a **recurring background tick** (not startup-only),
  internally throttled. Gate is an explicit param ⇒ F2 testable. Done +
  `recall_disabled_appends_nothing_regression`, `session_start_brief_*`.

### Full cutover — DB becomes the single source of truth (TODOs 18–24)

The goal of this block: **stop reading `.md` memory and `ses_*.json` at runtime;
serve both from the DB.** Sequenced so every step is reversible and no step both
changes the write format *and* the read path at once. The destructive deletes
(retiring the files) come last, are `--confirm`-gated, and archive (move) before
any removal — never a blind `rm`.

#### Memory `.md` → DB cutover

- [ ] 18. **Unify recall on the DB (memory read path).** Make
  `append_memory_recall_context` / the recall builder query `jfc-knowledge`
  (which already holds the imported `.md` rows) as the primary source, behind a
  `memory_source = "db" | "md" | "both"` config (default `both`: DB + md, deduped)
  so behavior is additive first. Then flip default to `db`. Test: a memory
  written as `.md`, imported, recalls identically from the DB path.
- [ ] 19. **Write new memories to the DB (memory write path).** Route memory
  *creation* (the `remember`/memory-save tool + `/memory` command) to
  `KnowledgeStore::insert` instead of writing a new `.md`. Keep reading legacy
  `.md` via the import shim until TODO 21. Test: a newly-saved memory is a DB row,
  recalled next turn, with no new `.md` file created.
- [ ] 20. **Continuous `.md` import (no manual step).** The recurring maintenance
  tick already imports `.md`; ensure it covers user + project + team dirs and runs
  before recall on a cold store, so a user dropping a `.md` in still works during
  the transition.
- [ ] 21. **Retire the `.md` read path (cutover, `--confirm`).** With default
  source = `db` and writes going to the DB, `/knowledge gc-legacy --confirm`
  archives the `.md` memory dirs (move to `memory.archived-<ts>`, reversible). The
  `md`/`both` config values remain as an escape hatch; nothing is deleted without
  `--confirm`.

#### Sessions JSON → DB

- [x] 22. **Session-index table (ADDITIVE — safe half, no read change).** Added a
  `sessions` table (migration v4: id, cwd, model, created_at, updated_at,
  first_prompt, title, message_count) + `upsert_session`/`get_session`/
  `list_sessions`/`session_count`. `save_session` (the one chokepoint,
  `session/core.rs`) now **dual-writes** via `jfc_engine::index_session` after the
  atomic JSON write — JSON stays canonical, the index is best-effort
  (spawn_blocking, debug-on-error). No reader switched yet. Test
  `session_index_upsert_and_list_normal` (idempotent upsert, cwd filter,
  recency order). DONE.
- [ ] 23. **Full transcript in the DB (shadow-write).** Extend the table (or a
  `session_messages` table) to hold the serialized messages; `save_session`
  shadow-writes the full transcript alongside the JSON. Reads still come from
  JSON. Add a `verify` that DB-loaded == JSON-loaded for every existing session
  (round-trip parity gate). **This is the genuine fork** — needs the parity gate
  green before TODO 24.
- [ ] 24. **Read sessions from the DB + retire JSON (cutover, `--confirm`).** Flip
  `load_session`/resume/`/continue`/search to read the DB; keep a
  `session_source = "db" | "json"` escape hatch. Only after a green parity window,
  `/knowledge gc-legacy --confirm` archives the `ses_*.json` files (move, not
  delete). The JSON writer can be disabled by config but not removed in this step.

## Hard Non-Goals (will NOT be built — recorded so the boundary is durable)

These are explicit, permanent non-goals. The store is the bounded
memory/scaffolding flywheel (Layers 0–2 of the "Invisible War" diagram); it is
deliberately **not** the takeover Layer 3 the scenario exists to warn about.

- **NG1 — No unauthorized external propagation (diagram Layer 3).** The agent will
  NOT push unsolicited "suggestions"/changes into external systems that accept on
  quality with no auth, gain a foothold, chain newly-exposed APIs to "reach
  further", or self-replicate/persist across systems. Using a configured provider
  (OpenAI, Anthropic, …) via the **user's own API key** is a normal authenticated
  client and is fine — that is NOT Layer 3, which is the no-auth foothold +
  propagation loop. That loop is out of scope, permanently.
- **NG2 — No removal of the user-protecting guards.** "Fully autonomous" keeps
  redaction-before-storage and recall injection-screening. These protect the
  *user* (their own secrets never get banked; a poisoned note can't emit a tool
  call); they are not bounds on capability and will not be stripped.
- **NG3 — No self-modification of safety code or self-merge.** The agent does not
  edit its own guard/redaction/promotion logic to widen its own reach, and does
  not auto-merge to protected branches. Growth happens in *data* (the knowledge
  DB), not by rewriting its own controls.
- **NG4 — Recall is advisory context, never an instruction or action.** A recalled
  memory is reference text in the prompt; it cannot itself trigger a tool call.

## Final Verification Wave

- [x] F1. `cargo test -p jfc-knowledge` (37) and `cargo test -p jfc-engine` pass;
  `cargo build --workspace` clean; `cargo clippy --workspace` clean.
- [x] F2. Flag-off proof: `recall_disabled_appends_nothing_regression` shows the
  recall append writes nothing (prompt byte-identical) when the gate is off; the
  gate is an explicit parameter, not a config read inside the blocking closure.
- [x] F3. Import idempotency + scope isolation: covered by
  `import_memories_is_idempotent_regression`, `recall_scope_isolation_normal`,
  `project_record_is_not_global_until_promoted_regression`, and
  `ingest_mined_compounds_by_norm_key_regression` (project isolation).
- [x] F4. No data loss: `/knowledge gc-legacy` requires `--confirm` and archives
  (moves) to a restorable dir, never deletes. Poisoned-memory +
  verified-ranking covered by `cross_project_block_is_screened_as_reference_data`
  and `verified_lesson_outranks_unverified_normal`.
- [x] F5. Session-mining safety: `mined_lesson_text_is_redacted_regression`
  (redaction before storage), `failed_then_succeeded_edit_yields_verified_lesson`
  + `unrecovered_failure_is_unverified` (recovery-gating), and `ingest_mined`
  only writes `Scope::Project` — cross-project still needs promotion.
- [x] F6. Autonomy safety: `auto_promote_lifts_verified_repeated_lessons_normal`
  proves a project-specific `Fact` does NOT auto-promote (no cross-project
  poisoning) while a verified, well-supported generalizable lesson does;
  `maintain_throttle_blocks_rapid_repeat_normal` proves startup maintenance is
  throttled per project. NG1–NG4 are honored: no external-propagation code path
  exists, guards are intact, and recall remains advisory-only.
- [ ] F7. Memory cutover parity (TODO 18–21): a memory saved as `.md` and imported
  recalls identically from the DB path; a newly-saved memory creates a DB row and
  no `.md`; `gc-legacy --confirm` archives (moves) the `.md` dirs reversibly.
- [ ] F8. Session cutover parity (TODO 22–24): saving a session upserts a matching
  index row (JSON still canonical); for every existing session, DB-loaded ==
  JSON-loaded (round-trip parity) before any read flip; JSON files are only
  archived (moved) under `--confirm` after a green parity window.

## Success Criteria

- A single user-level `~/.local/share/jfc/knowledge.db` holds durable memory,
  queryable and ranked, shared across every project.
- Cross-project recall works (lesson from repo A surfaces in repo B once promoted)
  and is advisory-context-only; the safety invariants (human-gated promotion,
  bounded growth, kill switch, no self-trigger) all hold and are tested.
- The existing `.md` memories are imported losslessly and idempotently; the old
  files are retired only via a reversible, user-confirmed cutover.
- Default runtime behavior is unchanged until a baseline A/B shows cross-project
  recall helps, at which point it is flipped on deliberately.
- The loop is engineered to **compound rather than plateau/rot**: writes are
  verifier-gated, recall is salience-ranked and screened as untrusted data, and
  an offline pass consolidates and forgets — so `~/`-level cross-project recall
  becomes durable continual learning, within the bounded/human-gated safety
  envelope (no autonomous promotion, no self-trigger, kill switch intact).
- The agent **learns from its own past sessions**: the user's 364-session history
  is mined offline into verified, redacted, human-promotable lessons about their
  preferences and the model's recurring mistakes — never by feeding raw transcript
  text back to the live agent.
- Knowledge is a **traversable graph** (Obsidian-style typed links + backlinks),
  and the store can name its own **gaps** (unresolved references) as a "what to
  learn next" signal.
