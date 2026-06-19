# Graph Context Engine - Learnings

## Task 1: Crate Scaffold

- petgraph 0.8.3 resolved cleanly with `serde-1` feature
- tree-sitter 0.25.10 and tree-sitter-rust 0.24.2 resolved
- bincode v2.0.1 pulled in (v3.0.0 available but task specifies v2)
- Workspace glob `crates/*` auto-includes jfc-graph after removing exclude
- No conflicts with existing jfc-ui dependencies
- Build time for jfc-graph alone: ~28s (mostly tree-sitter C compilation)
- Workspace lints inherited cleanly via `[lints] workspace = true`
