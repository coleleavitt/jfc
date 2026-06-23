# PLAN.md — Cross-Project Memory & Learning Store (bounded self-improvement)

Status: **proposal** · Owner crate: new `jfc-knowledge` (backed by SQLite) ·
Audience: JFC maintainers

## 0. Goal & explicit non-goal

**Goal.** Give JFC a single, durable, *cross-project* memory + learning store so
facts, preferences, skills, and verification findings accumulate across every
repo the user works in — not siloed per `.jfc/` folder. This is the
**scaffolding-level self-improvement flywheel** the research describes (agent
improves its own context/harness over time) — the *bounded* kind that
demonstrably works (Darwin/Huxley-Gödel-Machine lineage), not weight-level RSI.

**Non-goal (hard line).** This is **not** an unbounded recursive-self-improvement
loop, and it will not become one. No self-triggering on its own writes, no
self-merge, no editing of its own safety checks, no autonomous capability
acquisition. Every cross-project write is **gated** and every recall is
**advisory context**, never an action. The research the user supplied is the
justification for the guardrails, not for removing them: the documented failure
mode is *oversight erosion* (the human becomes a rubber stamp) and *objective
drift under self-modification*. The design below keeps the human in the loop and
keeps the store strictly read-as-context at runtime.

---

## 1. What exists today (grounded)

- **`jfc-memory`** (`crates/jfc-memory/src/store.rs`): per-file `.md` + YAML
  frontmatter. Two scopes: user-level `~/.config/jfc/memory/` and project-level
  `<project>/.jfc/memory/`. Memories are immutable (delete+recreate). Recall is
  a two-phase LLM pass (`recall.rs`).
- **`jfc-learn`** (`crates/jfc-learn/src/*`): JSONL append logs per project
  (`candidates.jsonl`, `reads.jsonl`, `quarantine.jsonl`), plus the Dreamer /
  curator / skill-induction jobs that consolidate them.
- **No `rusqlite`/`sqlx` anywhere yet** — today everything is flat files.

**The gap:** user-level memory is *global but tiny* (preferences only), and
learnings are *rich but project-local*. There is no queryable, ranked,
cross-project knowledge base. A lesson learned in repo A never helps repo B.

---

## 2. Design

### 2.1 Backend choice — `rusqlite` (bundled), synchronous, behind a blocking pool

- **`rusqlite` with the `bundled` feature**, not `sqlx`. Rationale: zero external
  toolchain, no async runtime entanglement (the rest of JFC is tokio, but the DB
  is tiny and local — wrap calls in `tokio::task::spawn_blocking`), simplest
  migration story, and no compile-time DB connection like `sqlx::query!`.
- **One file**, WAL mode, at the XDG data dir: `~/.local/share/jfc/knowledge.db`
  (resolve via `dirs::data_dir()`, fall back to `~/.local/share`). This is the
  durable, machine-global "everything we've learned" store — distinct from the
  per-project `.jfc/` working state.
- WAL + `busy_timeout` so concurrent JFC processes (multiple repos open) don't
  corrupt or block hard.

### 2.2 New crate `jfc-knowledge`

Keep this out of `jfc-memory`/`jfc-learn` (no god object): it is the *storage +
query* layer those two crates and the engine consume. Focused ownership:

```
crates/jfc-knowledge/
  src/lib.rs        // KnowledgeStore: open/migrate/handle
  src/schema.rs     // embedded migrations (v1, v2, …), version table
  src/record.rs     // KnowledgeRecord, Kind, Scope, Provenance
  src/query.rs      // insert/upsert, recall(query, filters, limit), decay
  src/project.rs    // project identity (git remote + root hash)
```

### 2.3 Schema (v1)

```sql
CREATE TABLE knowledge (
    id            TEXT PRIMARY KEY,           -- uuid
    kind          TEXT NOT NULL,              -- fact|preference|skill|finding|convention
    scope         TEXT NOT NULL,              -- user|project|global
    project_key   TEXT,                       -- NULL for global; else stable project id
    title         TEXT NOT NULL,
    body          TEXT NOT NULL,
    tags          TEXT NOT NULL DEFAULT '',   -- comma list (also FTS-indexed)
    source        TEXT,                       -- where it came from (file, session, tool)
    confidence    REAL NOT NULL DEFAULT 0.5,  -- 0..1
    created_at_ms INTEGER NOT NULL,
    last_used_ms  INTEGER,
    use_count     INTEGER NOT NULL DEFAULT 0,
    superseded_by TEXT,                       -- id of newer record; NULL if live
    promoted      INTEGER NOT NULL DEFAULT 0  -- 0=project-local, 1=human-promoted to global
);
CREATE VIRTUAL TABLE knowledge_fts USING fts5(
    title, body, tags, content='knowledge', content_rowid='rowid'
);
CREATE TABLE schema_version (version INTEGER NOT NULL);
```

- **`project_key`** = hash of `git remote get-url origin` (normalized) ⊕ repo
  root, computed in `project.rs`. Stable across clones/paths so the same project
  maps to one key on every machine checkout.
- **FTS5** gives fast lexical recall without an embedding pipeline (start lexical,
  mirror `slate::QueryClass`'s "lexical first" philosophy; embeddings are a later
  optional layer).
- **`superseded_by`** keeps history immutable (matches today's memory model)
  while letting recall filter to live rows only — directly answers "memory can
  become stale."

### 2.4 The promotion gate (this is the core safety boundary)

Cross-project leakage is the whole point **and** the whole risk. So:

- Writes default to **`scope='project'`** (or `user`), exactly like today.
- A record becomes **`scope='global'` only via explicit promotion** — either a
  `/knowledge promote <id>` slash command (human in the loop) or a Dreamer
  proposal the user approves. `promoted=1` is never set autonomously at runtime.
- Recall may *read* global rows as advisory context on any project; it may never
  *write* global rows as a side effect of a turn.

This is the rubber-stamp defense from the research made concrete: the agent can
*propose* a cross-project lesson, but a human (or an explicit, logged, reversible
gate) decides it generalizes.

### 2.5 Runtime integration (read path — advisory only)

- `jfc-memory::recall` gains a second source: after the existing project/user
  `.md` recall, query `jfc-knowledge` for `scope IN (user, global)` +
  `scope=project AND project_key=<this repo>`, rank by
  `confidence * recency_decay * log(use_count+1)`, take top-K, and fold into the
  same recall block already injected into the prompt seed. **No new prompt
  surface, no new tool the model calls to act** — it's context, like
  `## Current diagnostics`.
- On use, bump `use_count`/`last_used_ms` (write-back is a metric, not an action).

### 2.6 Write path (consolidation — bounded, batched, off the hot path)

- The existing **Dreamer** (`jfc-learn/src/dreamer.rs`, already daemon-scheduled)
  is the only thing that *promotes JSONL candidates → `jfc-knowledge` rows*. It
  already runs bounded and offline. No per-turn DB writes from the agent loop.
- Hard caps: max rows per project, max global rows, max body length; oldest
  low-confidence unused rows are decayed/pruned (a `decay()` pass), so the store
  can't grow without bound.

---

## 3. Safety invariants (must hold; each is testable)

1. **No self-trigger.** Nothing in the knowledge write path is triggered by a
   knowledge write. Consolidation runs only on the existing scheduled Dreamer
   tick or explicit `/knowledge` commands.
2. **Read-as-context only at runtime.** A recalled record can never become a tool
   call or an edit. It is text in the prompt, nothing more.
3. **Promotion is human-gated.** `promoted=1` / `scope='global'` requires an
   explicit command or approved proposal; assert no runtime code path sets it.
4. **Bounded growth.** Row/byte caps + decay pass; a property test fills past the
   cap and asserts size stays bounded.
5. **Reversible & inspectable.** `/knowledge list|show|forget|promote|demote`;
   every promotion is logged. A single SQLite file the user can delete to fully
   reset — the kill switch is `rm ~/.local/share/jfc/knowledge.db`.
6. **No secrets.** Reuse the existing redaction/`.jfcignore` access policy before
   any text is written; add a test that a credential-shaped string is refused.

---

## 4. Phases

- **Phase 1 — store + schema (no behavior change).** New `jfc-knowledge` crate,
  `rusqlite` bundled, migrations, `KnowledgeRecord`, insert/recall/decay, project
  identity. Pure library + unit tests. Nothing reads it yet. *Strict superset:
  builds and ships dormant.*
- **Phase 2 — read path.** Wire `jfc-memory::recall` to also query the store
  (behind a flag, default off → measure recall quality before defaulting on).
- **Phase 3 — write/consolidation path.** Dreamer promotes JSONL candidates into
  the store; `/knowledge` slash commands; the promotion gate.
- **Phase 4 — cross-project default-on + decay tuning**, only after a baseline
  A/B shows cross-project recall actually helps (don't ship on faith).

---

## 5. Eval / verification

- Unit: schema migration round-trip; insert/recall ranking; decay keeps size
  bounded; `superseded_by` hides stale rows; project_key stability across paths.
- Safety tests (one per §3 invariant) — these are the load-bearing ones.
- Integration: a fact promoted to global in repo A is recalled in repo B; a
  project-scoped fact in A is **not** recalled in B.
- Baseline A/B (Phase 4 gate): task success / rework with cross-project recall
  on vs off. Ship default-on only on a measured win; otherwise keep it opt-in.

## 6. Estimated effort

Phase 1 is a self-contained crate (~1 schema + ~1 store module + tests), additive
and dormant. Phases 2–3 touch the recall path and the Dreamer (existing owners),
no new god object. `rusqlite` `bundled` adds one vendored C dep — note it in the
public-build feature gate so the no-sensitive-features CI build still passes.

## 7. Why this is the right "RSI" to build

The supplied research is explicit: the empirically-working self-improvement loops
are **scaffolding/memory accumulation with a reliable gate**, while the dangerous,
unproven one is unbounded weight-level RSI with the human removed. This plan
builds the former and structurally refuses the latter. It makes JFC durably
better across projects (real leverage) while keeping every property the AI-control
literature says you must keep: bounded growth, human-gated promotion,
read-only-at-runtime recall, full inspectability, and a one-command kill switch.
