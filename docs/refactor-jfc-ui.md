# Refactor: splitting `jfc-ui` into a layered crate stack

## Why

`jfc-ui` is ~128k lines in one crate (`find crates/jfc-ui/src -name '*.rs' | xargs wc -l`).
Rendering, orchestration, and domain logic are fused together. The big modules:

| module | lines | concern |
| --- | --- | --- |
| `tools/` | 14.5k | domain logic (tool dispatch) |
| `input/` | 14.2k | input widgets + key handling |
| `runtime/` | 11.7k | event loop + orchestration |
| `message_view/` | 9.5k | transcript view model |
| `stream/` | 8.1k | provider stream orchestration |
| `render/` | 7.9k | ratatui draw code |
| `app/` | 5.3k | `App` god-object (`state.rs` is 1755 lines of fields) |
| `cli/` | 5.1k | bootstrap |
| `swarm/` | 5.1k | subagent orchestration (domain) |
| `session/` | 4.2k | session persistence (domain) |

It is, effectively, **three crates wearing a trenchcoat**: a view layer, an
app/orchestration layer, and a thin binary.

## The binding constraint (read this first)

The thing that makes this hard is **not** file location. Moving files alone just
relocates the coupling. The real seams to cut are two direct borrows of the
god-object:

1. **`render/` borrows `&App` directly.** `render/messages.rs:59` calls
   `crate::message_view::RenderCtx::from_app(app)` — signature
   `fn from_app(app: &'a App) -> Self` at `message_view/core.rs:75`. The renderer
   reaches straight into `App` fields, so it can never live in a crate that
   doesn't also contain `App`.
2. **Event-loop handlers mutate `App` while orchestrating.** The
   `runtime/event_loop/handlers/*` mutate `App` directly as a side effect of
   doing provider/session work, so orchestration and UI state are interleaved.

Until those two seams are cut, a crate split is impossible. So the plan is a
**strangler fig**, not a big move: cut the seams in place first, carve crates
last.

## Target shape

```
crates/
  jfc/         # binary: main.rs + cli/ bootstrap, config load, logging, mode select
  jfc-tui/     # render/, message_view/, HistoryCell layer, input widgets, spinner — VIEW ONLY
  jfc-app/     # runtime/, app/, stream/, compact/, tools wiring, session/, swarm/, workflows/
  jfc-stream/  # LATER, only if stream/ stays too coupled after the split
```

`jfc-tui` consumes a `ViewModel` snapshot and emits typed `InputEvent`s; it has
**zero** provider/session/tool dependencies. That dependency absence is the test
that the seam is real.

## Phases

Each phase is independently shippable and leaves the tree green.

### Phase 0 — guardrails first
Deterministic unit tests for the exact behaviors just patched (the six fixes):
- compaction repins scroll to bottom on `handle_done`,
- auto-compact re-queue excludes the compact-boundary message and joins
  multi-text parts,
- restored-file context block is `Role::User` and inserted at index 1
  (after the summary boundary, ahead of the preserved tail).

Pure-logic unit tests — they run in CI, not locally (OOM constraint). They are
the regression net for everything below. (CLAUDE.md: behavior changes need eval
coverage.)

### Phase 1 — HistoryCell transcript layer (in-place, no new crate)
Extract a `HistoryCell` trait (each rendered unit owns its display lines + height
math, à la Codex `history_cell/mod.rs`) out of `message_view/` +
`render/messages.rs`. Introduce a `Transcript` owning `Vec<Cell>` +
scroll/`follow_bottom` state. Route compaction/stream events through transcript
methods instead of mutating `app.messages` + ad-hoc `scroll_offset` math.

This retires the debt behind findings 1–2: the `scroll_to_bottom()` repin
becomes `Transcript::reset_after_compact()`, and the content-addressed render
cache becomes per-cell. Highest value, fully internal.

### Phase 2 — define the view-model seam
Replace `RenderCtx::from_app(&App)` with an explicit `ViewModel` snapshot that
the controller produces each frame and the renderer consumes (owns its data, so
no borrow-checker dance against `App`). Mirror it on input: terminal events →
typed `InputEvent` the controller consumes (`runtime/events.rs:111`'s `UiEvent`
is already most of the way there). This severs render's `&App` borrow — the
precondition for the crate split.

### Phase 3 — carve `jfc-tui`
Move `render/`, `message_view/`, the `HistoryCell` layer, input widgets,
`spinner.rs`, theme glue. It depends only on `ViewModel` + emits `InputEvent`.
Zero provider/session/tool deps — that's the test the seam is real.

### Phase 4 — carve `jfc-app` + thin `jfc` binary
Move `runtime/`, `app/`, `stream/`, `compact/`, tool dispatch, `session/`,
`swarm/`, etc. into `jfc-app`. `jfc` shrinks to `main.rs` + `cli/` bootstrap
wiring `jfc-app` ↔ `jfc-tui`.

### Phase 5 (optional) — `jfc-stream`
Only if `stream/` orchestration is still tangled after Phase 4.

## Known hazards

- **`App` is a god-object** (`state.rs` ~1755 lines of fields). The `ViewModel`
  snapshot must be **cheap to build per frame** — likely borrowed slices + small
  owned scalars, **not** a deep clone. This is the riskiest part of Phase 2.
- **`tools/` (14.5k) and `swarm/` (5k) are domain logic** currently resident in
  the UI crate. They belong in `jfc-app`, but watch for hidden `render`/view
  imports when moving them (Phase 4).
- **Appetite checkpoint:** Phases 1–2 deliver most of the architectural win (and
  all the scroll/compaction cleanup) with none of the cross-crate move risk.
  Stopping after Phase 2 is still a real improvement.
