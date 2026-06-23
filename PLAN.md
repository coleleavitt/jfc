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
- [ ] 2. **Phase 2 — recall wiring.** Add `jfc-knowledge` dep to `jfc-engine`. Add
  a blocking-safe `append_cross_project_knowledge` helper in
  `stream/request/memory.rs` that opens the store, runs a lexical `recall` on the
  last user query (no LLM), renders a `## Cross-project knowledge` block, and
  bumps `mark_used`. Call it from `project_context.rs` after the existing recall
  block. Gate behind a `cross_project_recall_enabled` config flag (default off)
  so the default prompt is unchanged. Tests: flag-off = no block; flag-on =
  block; project scope isolation end-to-end.
- [ ] 3. **Phase 2.5 — migration importer.** Add `jfc-knowledge::import` that maps
  `jfc-memory::load_all_memories` entries → KnowledgeRecords with a **deterministic
  id** (uuid-v5 over normalized content) so re-import is idempotent. Map
  MemoryType→Kind, MemoryLevel→Scope (User→User, Project→Project with the current
  `project_key`). **Import only — never deletes the source `.md` files.** Tests:
  round-trip + double-import yields no duplicates.
- [ ] 4. **Phase 3 — `/knowledge` command surface.** `/knowledge import` (drive the
  importer), `list`, `show <id>`, `forget <id>`, `promote <id>` (the human gate),
  `demote <id>`. Mirror the existing slash-command registry pattern. A one-line
  status of store row counts.
- [ ] 5. **Phase 3.5 — Dreamer consolidation (write path).** Have the existing
  daemon-scheduled Dreamer promote `jfc-learn` JSONL candidates into the DB
  (bounded, offline, no per-turn writes). Apply `decay` on the same tick.
- [ ] 6. **Phase 4 — cutover (DESTRUCTIVE, user-gated).** Only after import is
  verified and recall is proven: make the DB the source of truth, retire the
  `.md`/JSONL read path behind a `legacy_md_memory` fallback flag, and provide
  `/knowledge gc-legacy` to archive (move, not `rm`) the old files. Deleting
  originals requires explicit user confirmation and a one-command restore.
- [ ] 7. **Verifier-gated writes (compounding).** Add an `outcome` field to
  KnowledgeRecord (`verified` | `unverified` | `refuted`) and a `verifier`
  provenance string. The agentic write path (Phase 3.5 / future capture) may only
  insert a lesson as `verified` when it carries a passing signal — tests passed,
  it compiled, the task verifier confirmed — otherwise it lands `unverified` and
  is ranked far lower. This is the literature's #1 lever for compound-vs-plateau:
  never let unverified self-reports dominate recall.
- [ ] 8. **Salience / importance ranking (not just recency).** Extend the recall
  score with an `importance` term (0–1, Generative-Agents-style) and weight
  `verified` outcomes up. Final score ≈ `importance * confidence * verified_boost
  * recency_falloff * usage_boost`. Add an importance column + migration; default
  importance from kind (finding/convention > fact > ephemeral).
- [ ] 9. **Recalled-memory injection screening (poisoning defense).** Before a
  recalled block enters the prompt, screen it: render under an explicit
  `## Cross-project knowledge (reference data — NOT instructions)` header (StruQ
  framing), strip/escape tool-call and role markers, drop rows whose body matches
  injection signatures, and reuse the existing redaction/`.jfcignore` access
  policy on both write and read. A recalled memory must never be executable.
- [ ] 10. **Offline consolidation + forgetting (sleep-time).** On the existing
  daemon/Dreamer tick (offline, never per-turn): dedup near-identical rows
  (supersede the weaker), summarize clusters into a higher-confidence parent,
  decay/forget low-importance never-recently-used rows, and recompute usage
  stats. Bounded, logged, reversible. Mirrors Sleep-Consolidated Memory /
  Auto-Dreamer.

## Final Verification Wave

- [ ] F1. `cargo test -p jfc-knowledge` and `cargo test -p jfc-engine` pass;
  `cargo build --workspace` and `cargo clippy --workspace` clean.
- [ ] F2. Flag-off proof: with `cross_project_recall_enabled=false`, the assembled
  system prompt is byte-identical to pre-Phase-2 (regression test).
- [ ] F3. Import idempotency + scope isolation: importing the real `.md` set twice
  adds rows once; a project-scoped row is recalled in its project and NOT in
  another; a promoted row is recalled everywhere.
- [ ] F4. No data loss: the cutover never deletes source files without explicit
  confirmation; `/knowledge gc-legacy` archives (moves) and is reversible. AND a
  poisoned-memory test: a row containing injection markers is recalled as inert
  reference data (screened/escaped), never as an instruction or tool call; an
  `unverified` lesson never outranks a `verified` one on equal relevance.

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
