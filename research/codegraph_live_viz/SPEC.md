# Live Architecture Visualization + Mermaid Render — Idea Spec (NOT YET BUILT)

Status: **captured idea, do not build until approved.** Owner decision pending.

This is a two-part idea. Part 1 is small and self-contained; Part 2 is the
ambitious "watch the architecture evolve like a builder game" vision. Capture
only — no implementation has been done.

## Motivation

As a codebase grows, its architecture gets large and hard to hold in your head.
The user wants to *see* how the application fits together, visually, and watch it
update as the model and the code change over time — in the spirit of a 2D builder
game (Minecraft / Satisfactory / Redstone): modules are blocks, calls/deps are
belts/wires, and the world re-flows as you build.

## Part 1 — Mermaid / DOT rendering in message review

Goal: when an assistant message (or any reviewed content) contains a fenced
```` ```mermaid ```` (or ```` ```dot ```` / graphviz) block, render it to an actual
diagram in the JFC TUI message view instead of showing raw markup.

- Owner crate: `jfc-markdown` (parsing/extraction of fenced blocks) +
  `jfc` TUI (the render surface). Keep the renderer out of engine crates.
- TUI is ratatui (terminal) — true vector rendering isn't native. Options:
  1. ASCII/box-drawing layout of simple graphs (flowchart/sequence subset) — pure
     terminal, no deps, limited fidelity.
  2. Render to an image (via a mermaid/dot engine) and show through a terminal
     image protocol (kitty/sixel/iterm) when the terminal supports it; fall back
     to (1) or raw text otherwise.
- Scope guard: a *subset* of Mermaid first (flowchart + sequence). Do not pull a
  full browser/JS mermaid runtime into the TUI. Detect terminal capability before
  choosing a backend.

## Part 2 — Live, self-updating CodeGraphRS architecture view

Goal: an interactive 2D "builder-game" view of the architecture that updates in
real time as the codebase changes.

- Reuse, don't rebuild: CodeGraph already indexes every symbol, edge, and file
  with a ~1s file watcher (this repo's `.codegraph/` SQLite graph; sibling project
  `~/RustProjects/active/codegraph-rs`). `jfc-graph` owns graph execution + the
  code graph. **This is a visualization layer over the existing graph — NOT a
  second graph store.** (Architecture rule: no second source of truth.)
- Data source: query the existing graph for modules (nodes/blocks), and
  call/dependency edges (belts/wires). Subsystem grouping already exists via
  `codegraph_arch`.
- Live update: subscribe to the same file-watcher re-index signal the graph uses;
  on re-index, diff the graph and animate node/edge add/remove/move so you can
  *watch* the structure change as the model edits code.
- Layout: incremental/stable force-directed or layered layout so the world doesn't
  reshuffle wildly on each tick (stability is the whole point of "watching it
  grow"). Cluster by crate/module; collapse/expand subsystems (architectures get
  big — must support zoom + level-of-detail).
- Surface options (decide later): (a) a web view served by `jfc-web` (richest:
  canvas/WebGL, pan/zoom, the real "builder game" feel); (b) a TUI panel (limited);
  (c) export to Mermaid/DOT and reuse Part 1. Part 2 most naturally lives in
  `jfc-web`, driven by `jfc-graph` queries.

## Open questions before building

- TUI vs web surface for Part 2 (web is the natural fit for the builder-game UX).
- Which Mermaid subset for Part 1, and terminal-image vs ASCII fallback policy.
- Scope: is this a JFC product feature or a CodeGraphRS feature? (The live view is
  arguably a CodeGraphRS capability JFC embeds.)
- Performance/LOD strategy for very large graphs (collapse, filter, focus+context).

## Explicit non-goals (for now)

- No new graph store; read the existing CodeGraph index only.
- No full mermaid JS runtime in the TUI.
- No always-on background rendering cost in normal sessions — opt-in view.
