```markdown
# jfc Development Patterns

> Auto-generated skill from repository analysis

## Overview

This skill provides a comprehensive guide to the development patterns, coding conventions, and key workflows in the `jfc` Rust codebase. The repository is organized around modular Rust code, with a focus on UI logic, agent/skill systems, session/config management, and syntax highlighting. The guide covers file structure, naming conventions, workflow automation, and best practices for contributing new features or maintaining the project.

## Coding Conventions

- **File Naming:**
  Use `snake_case` for Rust file, module, function, and variable names.
  _Example:_
  ```
  message_view.rs
  main.rs
  agent_tools.rs
  ```

- **Import Style:**
  Mixed usage of absolute and relative imports, depending on context.
  _Example:_
  ```rust
  use crate::types::AgentType;
  use std::collections::HashMap;
  ```

- **Export Style:**
  Named exports are preferred.
  _Example:_
  ```rust
  pub struct Agent { /* ... */ }
  pub fn run_agent() { /* ... */ }
  ```

- **Module Organization:**
  - Core logic in `main.rs`
  - Feature-specific logic in dedicated modules (e.g., `tools.rs`, `agents.rs`)
  - Types in `types.rs`
  - Rendering in `render.rs` or `message_view.rs`
  - Input handling in `input.rs`
  - Session/config in `session.rs` and `config.rs`

- **Commit Messages:**
  - Use prefixes: `merge`, `fix`, `agent`, `feat`, `test`, `compact`
  - Keep messages concise (~57 characters on average)
  - Reference the feature, fix, or module affected

## Workflows

### Submodule Update: wgpui
**Trigger:** When upstream wgpui changes need to be integrated or a bugfix is required
**Command:** `/update-wgpui`

1. Update the `references/wgpui` submodule:
   ```sh
   git submodule update --remote references/wgpui
   ```
2. Optionally update integration code in `crates/jfc/src/main.rs`, `crates/jfc/src/app/`, or related UI files.
3. Commit changes with a message referencing the wgpui update:
   ```
   fix: update wgpui submodule to latest upstream
   ```

---

### Feature Addition: Multi-Module
**Trigger:** When adding a significant new feature, tool, or capability to the UI
**Command:** `/add-feature`

1. Add or update shared state/types in `crates/jfc/src/app/`, `crates/jfc-core`, or the owning crate.
2. Update or create main logic in `crates/jfc/src/app/`, `crates/jfc/src/runtime/`, or feature-specific modules.
3. Update rendering logic in `crates/jfc/src/render/` or `crates/jfc/src/message_view/`.
4. Update or create input handling in `crates/jfc/src/input/`.
5. Optionally update or add tests.

_Example:_
```rust
// types.rs
pub struct NewFeature { /* ... */ }

// main.rs
mod tools;
use tools::new_feature;

// render.rs
pub fn render_new_feature() { /* ... */ }
```

---

### Agent or Skill System Extension
**Trigger:** When expanding agent/skill/task capabilities or introducing new agent features
**Command:** `/add-agent`

1. Create or update `crates/jfc/src/render/agents.rs`, `crates/jfc/src/render/teammates_panel.rs`, or the owning agent/task crate.
2. Update shared types in `crates/jfc-core`, `crates/jfc-agent`, or the owning crate.
3. Update `crates/jfc/src/app/`, `crates/jfc/src/input/`, and `crates/jfc-daemon` scheduling code as needed to wire commands or agent logic.
4. Add or update YAML/frontmatter parsing logic.
5. Update help text and documentation as needed.

_Example:_
```rust
// agents.rs
pub fn register_new_agent() { /* ... */ }

// types.rs
pub enum AgentType {
    Existing,
    NewAgent,
}
```

---

### Session or Config Enhancement
**Trigger:** When improving session persistence, configuration, or CLI interface
**Command:** `/update-session`

1. Update `crates/jfc-session`, `crates/jfc-config`, or the session/config owner crate.
2. Update `crates/jfc/src/app/` and `crates/jfc/src/input/` to handle new session/config features.
3. Update `Cargo.toml` if new dependencies are needed.
4. Add or update tests for session/config behavior.

_Example:_
```rust
// session.rs
pub struct SessionConfig { /* ... */ }

// config.rs
pub fn load_config() { /* ... */ }
```

---

### Merge Worktree or Agent Branch
**Trigger:** When agent or feature work is completed in a branch/worktree and needs to be merged
**Command:** `/merge-agent`

1. Merge changes from worktree or agent branch:
   ```sh
   git merge feature/agent-branch
   ```
2. Resolve conflicts, especially in `agents.rs`, `input.rs`, `tools.rs`, `render.rs`, `session.rs`.
3. Retain or merge tests from both branches.
4. Commit with a merge message referencing the agent or feature:
   ```
   merge: integrate agent-branch into main
   ```

---

### Syntax Theme Bundle Extension
**Trigger:** When supporting new languages or improving syntax highlighting
**Command:** `/add-syntax`

1. Add new `.sublime-syntax` files to `crates/jfc-markdown/syntaxes/`.
2. Update `crates/jfc-markdown/src/lib.rs` or related files to register new grammars/themes.
3. Update `Cargo.toml` if new dependencies are required.
4. Commit with a message referencing the new languages/themes:
   ```
   feat: add syntax highlighting for Rust and TOML
   ```

---

## Testing Patterns

- **Framework:** Rust's built-in test harness (`#[test]`). The workspace is predominantly Rust; the `apps/design-web/` TypeScript sub-app uses Vitest-style `*.test.ts` files.
- **File Pattern:** Rust unit tests live in `#[cfg(test)]` modules co-located in source files; integration tests live under each crate's `tests/` directory (e.g. `crates/jfc-audit/tests/`). Only `apps/design-web/` uses `*.test.ts`.
- **Best Practices:**
  - Run `cargo test` from the workspace root; co-locate unit tests in `#[cfg(test)]` modules and put cross-module tests in `tests/`.
  - Use descriptive test names and cover both positive and negative cases.
  - If adding new features or config/session changes, ensure corresponding tests are updated or created.

## Commands

| Command         | Purpose                                                            |
|-----------------|--------------------------------------------------------------------|
| /update-wgpui   | Update the wgpui submodule and integrate upstream changes          |
| /add-feature    | Add a new feature or tool, updating multiple modules as needed     |
| /add-agent      | Add or extend agent/skill/task functionality                       |
| /update-session | Enhance session or configuration management                        |
| /merge-agent    | Merge a worktree or agent branch into main, resolving conflicts    |
| /add-syntax     | Add new syntax highlighting grammars or themes                     |
```
