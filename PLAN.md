# Migrate jfc-knowledge from rusqlite to sqlx (fully async)

## TL;DR

Replace the synchronous `rusqlite` backend of `jfc-knowledge` with async
`sqlx` (`SqlitePool`), make every `KnowledgeStore` method `async`, and cascade
`async` outward through all 238 call sites in the 7 dependent crates
(`jfc-session`, `jfc-engine`, `jfc-learn`, `jfc-memory`, `jfc-agents`,
`jfc-config`, `jfc-daemon`). Use **runtime `sqlx::query`/`query_as` only — no
compile-time macros** (avoids the `DATABASE_URL`/FTS5 macro-verification
friction), port the existing hand-rolled ordered migrator to async (preserve
versioning logic exactly), and keep the on-disk SQLite file format byte-compatible
(same engine, WAL, FTS5). The `sqlx` source of truth is the local checkout at
`~/RustProjects/forks/sqlx` (v0.9.0).

## Context

- `jfc-knowledge` holds an owned `rusqlite::Connection` behind `KnowledgeStore`.
  API surface in use: 64 `.execute`, 33 `.prepare`, 27 `.query_map`,
  14 `.query_row`, 5 `.transaction`, 3 `.pragma_update`, 3 `.execute_batch`,
  1 `.last_insert_rowid`, 1 `.busy_timeout`.
- Schema = 10 ordered DDL migrations gated on a `schema_version` table, applied
  in a transaction (`schema.rs`). Includes **FTS5 virtual tables + triggers**
  (`knowledge_fts`, `session_messages_fts`, `definitions_fts`).
- 238 call sites across 7 crates. Heaviest: `jfc-engine` (130),
  `jfc-session` (41), `jfc-memory` (18), `jfc-config` (15), `jfc-daemon` (14),
  `jfc-agents` (11), `jfc-learn` (9).
- Genuinely-sync call contexts that need explicit bridging:
  `jfc-session/src/task_store.rs` (`persist_unlocked`, `load_inner_from_db`,
  `reload_if_changed`), `task_history.rs`, `inbox.rs`, `search.rs`, `catalog.rs`.
  These are reached from sync `&self` methods and the `TaskStore` mutex path.
- Binary entrypoint (`crates/jfc/src/main.rs`) is already `#[tokio::main]`, so a
  tokio runtime is always present at the top.

## Work Objectives

- `jfc-knowledge` depends on `sqlx` (sqlite, runtime-tokio, no macros), not `rusqlite`.
- `KnowledgeStore` wraps a `SqlitePool`; every public method is `async`.
- Schema migrator ported to async, same version semantics, FTS5 + triggers intact.
- All 7 dependent crates updated so their call sites `.await` the new API.
- Genuinely-sync callers bridged via one documented helper
  (`block_on_knowledge`) using `tokio::task::block_in_place` +
  `Handle::current().block_on`, or refactored to async where the caller is async.
- `cargo build` and `cargo test` pass workspace-wide; `rg rusqlite` returns
  nothing under `crates/`.

## Verification Strategy

- Per-phase: `cargo build -p <crate>` after each crate is converted.
- `cargo test -p jfc-knowledge` after Phase 2 (store + schema parity).
- `cargo test -p jfc-session` after the task-store bridge (covers the earlier
  resurrection regression tests too).
- Workspace `cargo build` then `cargo test` in the Final Verification Wave.
- `cargo clippy --workspace` since this touches shared runtime abstractions.
- Grep gate: `rg -l rusqlite crates/` must be empty.

## Execution Strategy

Bottom-up, compiling at each boundary so breakage is localized:
1. Foundation (deps, error, pool, schema) — keep the crate compiling in isolation.
2. Convert the query layer + `KnowledgeStore` methods to async.
3. Cascade async through dependents, leaf crates first (`jfc-learn`,
   `jfc-memory`, `jfc-config`, `jfc-agents`, `jfc-daemon`), then `jfc-session`,
   then `jfc-engine` (largest).
4. Remove rusqlite, full verification wave.

Bridging rule: prefer making the caller `async` and `.await`ing. Only use the
`block_on_knowledge` sync bridge where the call site is structurally sync (a
`Drop`, a `Mutex`-guarded `persist_unlocked`, or a non-async trait method) and
cannot be made async without a second viral cascade.

## TODOs

- [ ] 1. Add `sqlx` (features: `runtime-tokio`, `sqlite`, `chrono`) and remove
  `rusqlite` in `crates/jfc-knowledge/Cargo.toml`, pointing at workspace dep;
  add `sqlx` to `[workspace.dependencies]` referencing the local fork path.
- [ ] 2. Rewrite `error.rs`: replace `rusqlite::Error` `#[from]` with
  `sqlx::Error`; keep `Migration`/`Io`/`InvalidRecord` variants.
- [ ] 3. Port `schema.rs` to async: `apply_pragmas`/`migrate` take `&SqlitePool`
  (or `&mut SqliteConnection`), run the same ordered DDL + `schema_version`
  gating in a transaction; preserve FTS5 tables/triggers verbatim. Configure
  pragmas (WAL, synchronous=NORMAL, foreign_keys=ON, busy_timeout) via
  `SqliteConnectOptions`.
- [ ] 4. Convert `KnowledgeStore` in `lib.rs` to hold `SqlitePool`; make
  `open`/`open_default`/`open_in_memory` async constructors; convert every
  method body from `self.conn` rusqlite calls to `sqlx::query*().await`.
- [ ] 5. Convert the remaining query modules (`query.rs`, `memory.rs`,
  `definitions.rs`, `record.rs`, `import.rs`, `session_mine.rs`, `project.rs`,
  `redact.rs`) and `agent_events/*` to async sqlx; map `query_map`→`fetch_all`
  + `try_get`, `query_row`→`fetch_one/optional`, `last_insert_rowid`→
  `last_insert_rowid()` on the `SqliteQueryResult`.
- [ ] 6. Update `jfc-knowledge` internal tests to `#[tokio::test]` and `.await`;
  `cargo test -p jfc-knowledge` green.
- [ ] 7. Add the `block_on_knowledge` sync bridge helper (in `jfc-session`, or a
  shared spot) using `block_in_place` + `Handle::current().block_on`; document
  the invariant that it must run inside the tokio runtime.
- [ ] 8. Cascade async through `jfc-learn` (9 sites): make callers async/await or
  bridge; `cargo build -p jfc-learn`.
- [ ] 9. Cascade through `jfc-memory` (18 sites); `cargo build -p jfc-memory`.
- [ ] 10. Cascade through `jfc-config` (15 sites); `cargo build -p jfc-config`.
- [ ] 11. Cascade through `jfc-agents` (11 sites); `cargo build -p jfc-agents`.
- [ ] 12. Cascade through `jfc-daemon` (14 sites); `cargo build -p jfc-daemon`.
- [ ] 13. Cascade through `jfc-session` (41 sites): bridge the `TaskStore`
  persist/reload/load paths and `task_history`/`inbox`/`search`/`catalog`;
  `cargo build -p jfc-session` and `cargo test -p jfc-session`.
- [ ] 14. Cascade through `jfc-engine` (130 sites): convert async call paths to
  `.await`; bridge structurally-sync ones; `cargo build -p jfc-engine`.
- [ ] 15. Remove the `rusqlite` dep entirely; `rg -l rusqlite crates/` empty.

## Final Verification Wave

- [ ] F1. `cargo build` (workspace) passes.
- [ ] F2. `cargo test` (workspace) passes, including the task-store resurrection
  regression tests and the knowledge schema parity tests.
- [ ] F3. `cargo clippy --workspace` clean (no new warnings on touched crates).
- [ ] F4. `rg -l rusqlite crates/` returns nothing; `~/.local/share/jfc/knowledge.db`
  opens and migrates cleanly via the sqlx path (smoke: open_default + a recall).

## Success Criteria

- `jfc-knowledge` uses `sqlx` exclusively; no `rusqlite` anywhere under `crates/`.
- Every `KnowledgeStore` method is async; the dependent crates await them, with
  only the documented `block_on_knowledge` bridge at structurally-sync sites.
- Schema/version semantics and FTS5 search behavior are preserved (existing DB
  files keep working).
- `cargo build` + `cargo test` + `cargo clippy --workspace` all green.
