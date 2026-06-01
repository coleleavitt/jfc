# JFC Project Context

JFC is a Rust workspace for an agentic terminal coding assistant. It includes provider adapters, a ratatui UI, persistent memory, skills, subagents, task tracking, MCP integration, a code graph, background learning jobs, and cost/budget accounting.

## Working Rules

- Run `cargo build` from the workspace root for compile verification.
- Run `cargo test` from the workspace root before handing off non-trivial code changes.
- Run `cargo clippy --workspace` when changes touch shared abstractions or runtime behavior.
- Prefer CodeGraph tools for structural code questions and `rg` for literal text.
- Keep edits scoped to the crate that owns the behavior.
- Do not commit runtime state, crash dumps, profiling output, or vendored research changes unless explicitly requested.

## Design Biases

- Progressive disclosure beats large always-on prompt/tool surfaces.
- Verification should be adversarial and should leave durable findings when it discovers repeatable failures.
- Agent prompt or policy changes need eval coverage, not just prose review.
