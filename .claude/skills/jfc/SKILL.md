```markdown
# jfc Development Patterns

> Auto-generated skill from repository analysis

## Overview

This skill provides a comprehensive guide to the development patterns, coding conventions, and key workflows in the `jfc` Rust codebase. The repository is organized around modular Rust code, with a focus on UI logic, agent/skill systems, session/config management, and syntax highlighting. The guide covers file structure, naming conventions, workflow automation, and best practices for contributing new features or maintaining the project.

## Coding Conventions

- **File Naming:**  
  Use `camelCase` for file names.  
  _Example:_  
  ```
  messageView.rs
  main.rs
  agentTools.rs
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
  - Rendering in `render.rs` or `messageView.rs`
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
2. Optionally update integration code in `crates/jfc-ui/src/main.rs` or related UI files.
3. Commit changes with a message referencing the wgpui update:
   ```
   fix: update wgpui submodule to latest upstream
   ```

---

### Feature Addition: Multi-Module
**Trigger:** When adding a significant new feature, tool, or capability to the UI  
**Command:** `/add-feature`

1. Add or update types in `crates/jfc-ui/src/types.rs`.
2. Update or create main logic in `crates/jfc-ui/src/main.rs` and/or feature-specific modules (e.g., `tools.rs`, `agents.rs`, `tasks.rs`).
3. Update rendering logic in `crates/jfc-ui/src/render.rs` or `messageView.rs`.
4. Update or create input handling in `crates/jfc-ui/src/input.rs`.
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

1. Create or update `crates/jfc-ui/src/agents.rs` and/or `skills/tasks` modules.
2. Update types in `crates/jfc-ui/src/types.rs`.
3. Update `main.rs`, `input.rs`, and possibly `scheduler.rs` to wire up new commands or agent logic.
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

1. Update `crates/jfc-ui/src/session.rs` and/or `config.rs`.
2. Update `main.rs` and `input.rs` to handle new session/config features.
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

1. Add new `.sublime-syntax` files to `crates/jfc-ui/syntaxes/`.
2. Update `crates/jfc-ui/src/markdown.rs` or related files to register new grammars/themes.
3. Update `Cargo.toml` if new dependencies are required.
4. Commit with a message referencing the new languages/themes:
   ```
   feat: add syntax highlighting for Rust and TOML
   ```

---

## Testing Patterns

- **Framework:** Unknown (no explicit Rust test framework detected, but test files follow a `.test.ts` pattern, suggesting some TypeScript-based harness or integration).
- **File Pattern:** `*.test.ts`
- **Best Practices:**
  - Place tests alongside the module or in a dedicated `tests/` directory.
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
