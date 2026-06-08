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
- Architecture beats feature velocity. Before adding a feature, identify the owner
  of shared state, the integration path with existing features, and the scope boundary.
- Avoid god objects. Do not stuff new behavior into one central struct/module just
  because it is the shortest prompt path; preserve focused ownership boundaries.
- Treat fast AI-generated features as scope-risky. Check whether the request rebuilds
  an existing capability or expands beyond the intended audience before implementing.

## Claude Runtime Layout

- Keep shared, stable project instructions here in root `CLAUDE.md`.
- Keep focused always-on guidance in `.claude/rules/*.md`.
- Keep reusable slash-command prompts in `.claude/commands/*.md`.
- Keep custom subagents in `.claude/agents/*.md`.
- Keep larger repeatable workflows, scripts, and bundled references in `.claude/skills/<name>/SKILL.md`.
- Keep machine-specific notes in `CLAUDE.local.md`; it is ignored by git.
- Treat `AGENTS.md`, `AI.md`, `.github/copilot-instructions.md`, `.cursor/rules/`, `.cursorrules`, `.windsurfrules`, `.clinerules`, `code-style.md`, `testing.md`, and `security.md` as import/init sources, not the canonical runtime config.
