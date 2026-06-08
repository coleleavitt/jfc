# Project Memory

## Durable Facts

- JFC is a Rust workspace for an agentic terminal coding assistant.
- Root `CLAUDE.md` is intentionally present so JFC can dogfood context loading.
- `.claude/` is ignored by git in this repo and is suitable for local Claude Code
  runtime files.
- Prefer CodeGraph tools for structural code questions and `rg` for literal search.
- Keep generated runtime state, crash dumps, profiling output, and vendored research
  out of source changes unless the task explicitly concerns them.
- Local Claude/JFC guidance should guard against the common AI failure mode where
  many individually successful features do not work together.
- Before accepting feature velocity, check architecture, state ownership, cross-view
  integration, and scope boundaries.

## Useful Verification Defaults

- Build: `cargo build`
- Test: `cargo test`
- Lint: `cargo clippy --workspace`
