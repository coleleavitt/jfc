# Unified Cross-Project Memory Store (jfc-knowledge)

## TL;DR

Replace the scattered per-project `.jfc/memory/*.md` + `jfc-learn/*.jsonl` files
with a single, durable, queryable **user-level SQLite database** at
`~/.local/share/jfc/knowledge.db` (Obsidian-style: one vault, every project) so
facts, preferences, skills, and findings accumulate **across projects**. This is
bounded, scaffolding-level self-improvement — recall is advisory context only,
cross-project promotion is human-gated, growth is capped, and the whole store is
one deletable file (kill switch). It is explicitly **not** an unbounded
weight-level RSI loop. Phase 1 (the store) is DONE and committed; the remaining
phases wire recall, import the existing `.md` files, then cut over.

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

## Final Verification Wave

- [x] F1. `cargo test -p jfc-knowledge` (37) and `cargo test -p jfc-engine` pass;
  `cargo build --workspace` clean; `cargo clippy --workspace` clean.
- [ ] F2. Flag-off proof: with `cross_project_recall_enabled=false`, the assembled
  system prompt is byte-identical to pre-Phase-2 (regression test).
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
  only writes `Scope::Project` — cross-project still needs `/knowledge promote`.

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
