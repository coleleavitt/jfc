# Graph-Based Context Engine

## TL;DR

> **Quick Summary**: Build a queryable code graph database (`crates/jfc-graph`) that turns codebases into a petgraph-backed property graph (functions=nodes, calls=edges), queryable via a minimal pipe-based DSL, with symbol-based semantic editing, virtual edit validation, and event-sourced persistence.
> 
> **Deliverables**:
> - New `crates/jfc-graph` workspace crate with full test coverage
> - Tree-sitter Rust adapter extracting functions, structs, enums, modules, traits + call/usage edges
> - Pipe-based mini-DSL parser and executor (≤7 operators)
> - `graph_query` tool integrated into jfc-ui's tool dispatch
> - Symbol-based semantic editing (LLM operates on handles, never file:line)
> - Virtual edit validation (inline call-site compatibility checking)
> - Partial struct selection with field-level granularity
> - Event-sourced persistence with snapshot + diffs + replay/undo
> - Language-agnostic adapter trait (Rust impl only for v1)
> - Modular capability tree for enabling/disabling analysis features
> 
> **Estimated Effort**: XL (5K+ lines new code across ~25 source files)
> **Parallel Execution**: YES - 4 waves
> **Critical Path**: Task 1 → 2 → 4 → 7 → 10 → 13 → 16 → 19 → 22 → F1-F4

---

## Context

### Original Request
Build a Graph-Based Context Engine per GitHub Milestone 1 (issues #1-#6, #18). The codebase should be treated as a graph database where the LLM can perform surgical context selection via DSL queries instead of naive file-level context loading. The system must track visited nodes to prevent cycles, support configurable traversal depth, and enable symbol-based editing where the LLM never deals with file paths or line numbers directly.

### Interview Summary
**Key Discussions**:
- **Architecture**: Separate crate (`crates/jfc-graph`), petgraph + custom pipe-based DSL
- **Persistence**: Event-sourced with BTRFS-like semantics (snapshot + append-only diffs + replay/undo)
- **Dual-source parsing**: Tree-sitter (fast, syntax-error tolerant) + LSP enrichment (semantic accuracy)
- **DSL access model**: Both — LLM calls `graph_query` tool explicitly AND system auto-generates context on edits
- **Cycle detection**: Visited-set per traversal with configurable max depth, MANDATORY

**Research Findings**:
- **Arbor** (MIT, Rust): Closest prior art. petgraph + tree-sitter + sled. Impact analysis, context slicing. Our design extends this with DSL, event-sourcing, virtual validation.
- **Joern CPG**: Merges AST + CFG + PDG. Their CPGQL inspired our DSL. Node schema: METHOD, CALL, LOCAL, TYPE_DECL etc.
- **codebadger** (2026): LLM + CPG via MCP tools — validates high-level tool approach over raw graph queries
- **LLMxCPG** (2025): CPG slicing reduces code 67-91% preserving vulnerability context — proves token savings
- **Taint-based slicing** (2025): 99% code reduction via slicing — proves surgical context works
- **tree-sitter-graph**: DSL for constructing graphs from AST — reference for language adapter design
- **stack-graphs** (GitHub): Incremental name resolution without full LSP — reference for scope resolution

### Metis Review
**Identified Gaps** (addressed):
- **LSP client is fire-and-forget** → Graph must work WITHOUT LSP initially (tree-sitter only). LSP enrichment deferred to Phase 3.
- **No request/response dispatch in LSP client** → LSP redesign is a separate phase, not a blocker for the graph engine.
- **Unresolved names without LSP** → Accept `EdgeKind::UnresolvedCall(name)` in early phases. LSP resolves later.
- **Symbol table invalidation after edits** → Re-parse single edited file (tree-sitter is <1ms per file).
- **Tree-sitter can't resolve cross-module names** → Explicit `UnresolvedCall` edge type. Scope-aware heuristic matching as intermediate.
- **Token budget for graph outputs** → Graph query results respect a max_tokens parameter.
- **Workspace multi-crate support** → Graph spans workspace, nodes carry crate identifier.
- **Closures/async blocks** → Closures as sub-nodes of containing function, not top-level nodes in v1.
- **Conditional compilation** → Include `#[cfg(test)]` code but tag nodes with `cfg_predicate` metadata.
- **External dependencies** → Not tracked as nodes. Calls to external crates are `EdgeKind::ExternalCall(crate_name, path)`.

---

## Work Objectives

### Core Objective
Create a graph-based context engine that enables an LLM to surgically select only the relevant code context (functions, types, call sites) via a mini DSL, dramatically reducing token usage while making it impossible to miss critical dependencies.

### Concrete Deliverables
- `crates/jfc-graph/` — new workspace crate (library)
- `crates/jfc-graph/src/graph.rs` — petgraph-backed CodeGraph
- `crates/jfc-graph/src/nodes.rs` — typed node definitions (Function, Struct, Enum, Module, Trait)
- `crates/jfc-graph/src/edges.rs` — typed edge definitions (Calls, UsesType, References, Contains, etc.)
- `crates/jfc-graph/src/adapter/mod.rs` — language adapter trait
- `crates/jfc-graph/src/adapter/rust.rs` — tree-sitter Rust implementation
- `crates/jfc-graph/src/dsl/mod.rs` — DSL parser + executor
- `crates/jfc-graph/src/symbols.rs` — symbol table (handle → location mapping)
- `crates/jfc-graph/src/validation.rs` — virtual edit validation
- `crates/jfc-graph/src/persistence.rs` — event-sourced storage
- `crates/jfc-graph/src/capabilities.rs` — modular feature tree
- `crates/jfc-graph/src/traversal.rs` — cycle-aware graph traversal algorithms
- `crates/jfc-graph/tests/fixtures/` — test fixture Rust files
- Integration: new `ToolKind::GraphQuery` in jfc-ui

### Definition of Done
- [ ] `cargo test -p jfc-graph` — all tests pass (>90% coverage of new code)
- [ ] `cargo build --workspace` — no errors or warnings
- [ ] Graph of jfc-ui's own source produces deterministic node/edge counts
- [ ] DSL query `fn("execute_tool") | callees | depth 1` returns correct function names
- [ ] Symbol-based edit of a test fixture function updates file correctly
- [ ] Virtual validation detects signature incompatibility at call sites
- [ ] Event log can replay to reconstruct graph state
- [ ] All 7 GitHub issues (#1-#6, #18) have at least one passing acceptance test

### Must Have
- Cycle detection with visited-set (HARD REQUIREMENT)
- Configurable max depth for traversal
- Graceful degradation without LSP (tree-sitter only mode fully functional)
- Partial struct selection with metadata indicating partial view
- Deterministic node IDs (same input → same graph)
- Token budget respect (query results capped by max_tokens parameter)
- Workspace-spanning graph (multiple crates)

### Must NOT Have (Guardrails)
- ❌ NO Turing-complete query language (no variables, no loops, no conditionals in DSL)
- ❌ NO modification of `lsp_client.rs` until Phase 3 (LSP enrichment)
- ❌ NO ratatui/TUI code in jfc-graph crate (pure library, no UI deps)
- ❌ NO multiple language adapter implementations (Rust only, trait for future)
- ❌ NO auto-context injection until explicit `graph_query` tool works end-to-end
- ❌ NO persistence before in-memory graph is queryable
- ❌ NO full type-checking in virtual validation (signature compat only)
- ❌ NO tracking external dependency internals (just `ExternalCall` edge)
- ❌ NO `Display` impls for internal-only types
- ❌ NO builder patterns for single-construction-site types
- ❌ NO feature flags before there's a second consumer
- ❌ DSL limited to ≤8 operators: `fn`, `callers`, `callees`, `type`, `depth`, `filter`, `show`, `taint`
- ❌ NO more than 5 node kinds in v1: Function, Struct, Enum, Module, Trait

---

## Verification Strategy

> **ZERO HUMAN INTERVENTION** - ALL verification is agent-executed. No exceptions.

### Test Decision
- **Infrastructure exists**: YES (cargo test)
- **Automated tests**: TDD (RED → GREEN → REFACTOR for core logic)
- **Framework**: Rust's built-in `#[test]` + `cargo test -p jfc-graph`
- **If TDD**: Each task writes test FIRST (failing), then implementation, then verify pass

### QA Policy
Every task MUST include agent-executed QA scenarios.
Evidence saved to `.sisyphus/evidence/task-{N}-{scenario-slug}.{ext}`.

- **Library/Module**: Use Bash (`cargo test -p jfc-graph -- <test_name> --nocapture`)
- **Integration**: Use Bash (`cargo build --workspace && cargo test --workspace`)
- **Tool testing**: Use Bash (invoke jfc binary with test fixture, verify output)

---

## Execution Strategy

### Parallel Execution Waves

```
Wave 1 (Foundation — start immediately, 5 parallel tasks):
├── Task 1: Crate scaffolding + Cargo.toml + workspace registration [general]
├── Task 2: Node & Edge type definitions with tests [general]
├── Task 3: Test fixtures (sample.rs, mutual_recursion.rs, etc.) [general]
├── Task 4: Language adapter trait definition [general]
└── Task 5: Traversal algorithms with cycle detection [general]

Wave 2 (Core Engine — after Wave 1, 6 parallel tasks):
├── Task 6: CodeGraph struct (petgraph wrapper, CRUD ops) (depends: 2) [general]
├── Task 7: Tree-sitter Rust adapter — function/struct extraction (depends: 3, 4) [general]
├── Task 8: Tree-sitter Rust adapter — call site + edge detection (depends: 3, 4, 7) [general]
├── Task 9: Graph construction pipeline (file→AST→nodes→edges) (depends: 6, 7, 8) [general]
├── Task 10: DSL lexer + parser (depends: 2) [general]
└── Task 11: Symbol table (handle→location mapping) (depends: 2, 6) [general]

Wave 3 (Features — after Wave 2, 8 parallel tasks):
├── Task 12: DSL executor + query engine (depends: 5, 6, 9, 10) [general]
├── Task 13: Partial struct selection (field-level granularity) (depends: 6, 7, 9) [general]
├── Task 14: Virtual edit validation (inline + signature check) (depends: 9, 11) [general]
├── Task 15: Event-sourced persistence (snapshot + diffs) (depends: 6, 9) [general]
├── Task 16: Modular capability tree (depends: 6) [general]
├── Task 17: Token-budgeted output formatting (depends: 9, 12) [general]
├── Task 18: Graph incremental update (re-parse single file) (depends: 9, 11) [general]
└── Task 24: Taint/propagation analysis DSL operator (depends: 5, 6, 9, 12) [general]

Wave 4 (Integration — after Wave 3, 8 parallel tasks):
├── Task 19: jfc-ui tool integration (ToolKind::GraphQuery) (depends: 12, 17) [general]
├── Task 20: Symbol-based edit tool (edit via handle) (depends: 11, 14, 18, 19) [general]
├── Task 21: LSP client redesign (request/response dispatch) (depends: none from graph) [general]
├── Task 22: LSP enrichment layer (resolve UnresolvedCall edges) (depends: 9, 21) [general]
├── Task 23: Auto-context injection on Edit/Write tools (depends: 12, 18, 19) [general]
├── Task 25: Sub-agent cascade on signature change (depends: 14, 19, 20) [general]
├── Task 26: Edit reason metadata forwarding (depends: 15, 20, 25) [general]
└── Task 27: TUI graph operation inspectability (depends: 19, 12) [general]

Wave FINAL (After ALL tasks — 4 parallel reviews, then user okay):
├── Task F1: Plan compliance audit (oracle)
├── Task F2: Code quality review (general)
├── Task F3: Real manual QA (general)
└── Task F4: Scope fidelity check (general)
-> Present results -> Get explicit user okay

Critical Path: T1 → T2 → T6 → T9 → T12 → T17 → T19 → T20 → T25 → F1-F4
Parallel Speedup: ~65% faster than sequential
Max Concurrent: 8 (Waves 3 & 4)
```

### Dependency Matrix

| Task | Depends On | Blocks | Wave |
|------|-----------|--------|------|
| 1 | — | 2-5 | 1 |
| 2 | 1 | 6, 10, 11 | 1 |
| 3 | 1 | 7, 8 | 1 |
| 4 | 1 | 7, 8 | 1 |
| 5 | 1 | 12 | 1 |
| 6 | 2 | 9, 11, 12, 13, 15, 16 | 2 |
| 7 | 3, 4 | 8, 9, 13 | 2 |
| 8 | 3, 4, 7 | 9 | 2 |
| 9 | 6, 7, 8 | 12, 13, 14, 15, 17, 18, 22 | 2 |
| 10 | 2 | 12 | 2 |
| 11 | 2, 6 | 14, 18, 20 | 2 |
| 12 | 5, 6, 9, 10 | 17, 19, 23 | 3 |
| 13 | 6, 7, 9 | — | 3 |
| 14 | 9, 11 | 20 | 3 |
| 15 | 6, 9 | — | 3 |
| 16 | 6 | — | 3 |
| 17 | 9, 12 | 19 | 3 |
| 18 | 9, 11 | 20, 23 | 3 |
| 19 | 12, 17 | 20, 23 | 4 |
| 20 | 11, 14, 18, 19 | — | 4 |
| 21 | — | 22 | 4 |
| 22 | 9, 21 | — | 4 |
| 23 | 12, 18, 19 | — | 4 |
| 24 | 5, 6, 9, 12 | — | 3 |
| 25 | 14, 19, 20 | 26 | 4 |
| 26 | 15, 20, 25 | — | 4 |
| 27 | 19, 12 | — | 4 |

### Agent Dispatch Summary

- **Wave 1**: 5 tasks → all `general` (category: `deep`)
- **Wave 2**: 6 tasks → all `general` (category: `deep`)
- **Wave 3**: 8 tasks → all `general` (category: `deep`)
- **Wave 4**: 8 tasks → 7 `general` (category: `deep`) + T27 `visual-engineering`
- **FINAL**: 4 tasks → F1 `oracle`, F2-F4 `general`

---

## TODOs

- [x] 1. Crate Scaffolding + Workspace Registration

  **What to do**:
  - Create `crates/jfc-graph/Cargo.toml` with: name="jfc-graph", edition="2024", dependencies=[petgraph, tree-sitter, tree-sitter-rust, serde, bincode]
  - Create `crates/jfc-graph/src/lib.rs` with module declarations (pub mod graph, nodes, edges, adapter, dsl, symbols, validation, persistence, capabilities, traversal)
  - Add `"crates/jfc-graph"` to workspace members in root `Cargo.toml`
  - Create `crates/jfc-graph/src/` with empty module files for each submodule
  - Create `crates/jfc-graph/tests/` directory
  - Create `.gitignore` in `crates/jfc-graph/research/` to exclude cloned repos (keep PDFs)
  - Verify: `cargo build -p jfc-graph` compiles

  **Must NOT do**:
  - Do NOT add ratatui, crossterm, or any TUI dependency
  - Do NOT add lsp-types (LSP enrichment is Phase 3)

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: [`rust-style`]
    - `rust-style`: Idiomatic Rust crate structure and Cargo.toml conventions

  **Parallelization**:
  - **Can Run In Parallel**: NO (first task, everything depends on it)
  - **Parallel Group**: Wave 1 lead
  - **Blocks**: Tasks 2, 3, 4, 5
  - **Blocked By**: None

  **References**:
  - `Cargo.toml:1-20` (root workspace) — Current workspace member list to extend
  - `crates/jfc-ui/Cargo.toml` — Reference for edition, dependency style
  - `crates/jfc-graph/research/arbor/crates/arbor-graph/Cargo.toml` — Reference for graph crate deps (petgraph version, tree-sitter version)
  - `crates/jfc-graph/research/petgraph/crates/petgraph/Cargo.toml` — petgraph's actual crate name and version

  **Acceptance Criteria**:
  - [ ] `cargo build -p jfc-graph` exits 0
  - [ ] `cargo test -p jfc-graph` exits 0 (even with no tests yet)
  - [ ] Root `Cargo.toml` lists `crates/jfc-graph` in workspace members
  - [ ] `crates/jfc-graph/Cargo.toml` has petgraph, tree-sitter, serde as deps
  - [ ] No ratatui or crossterm in jfc-graph deps

  **QA Scenarios**:
  ```
  Scenario: Crate builds in workspace
    Tool: Bash
    Preconditions: Repository at current HEAD
    Steps:
      1. Run `cargo build -p jfc-graph`
      2. Assert exit code 0
      3. Run `cargo build --workspace`
      4. Assert exit code 0
    Expected Result: Both commands succeed without errors
    Failure Indicators: Compilation error, missing dependency, workspace resolution failure
    Evidence: .sisyphus/evidence/task-1-crate-builds.txt

  Scenario: No forbidden dependencies
    Tool: Bash
    Preconditions: Cargo.toml written
    Steps:
      1. Run `grep -c "ratatui\|crossterm\|lsp-types" crates/jfc-graph/Cargo.toml`
      2. Assert output is "0"
    Expected Result: Zero matches for forbidden deps
    Failure Indicators: Any match found
    Evidence: .sisyphus/evidence/task-1-no-forbidden-deps.txt
  ```

  **Commit**: YES
  - Message: `feat(graph): scaffold jfc-graph crate with workspace registration`
  - Files: `Cargo.toml`, `crates/jfc-graph/Cargo.toml`, `crates/jfc-graph/src/lib.rs`, `crates/jfc-graph/src/*.rs`
  - Pre-commit: `cargo build --workspace`

- [x] 2. Node & Edge Type Definitions

  **What to do**:
  - Define `NodeKind` enum: `Function`, `Struct`, `Enum`, `Module`, `Trait` (exactly 5, no more)
  - Define `NodeData` struct: `id: NodeId`, `kind: NodeKind`, `name: String`, `qualified_name: String`, `file_path: PathBuf`, `span: Span`, `visibility: Visibility`, `metadata: HashMap<String, String>`
  - Define `NodeId` as deterministic hash: `hash(file_path + ":" + qualified_name + ":" + kind)`
  - Define `EdgeKind` enum: `Calls`, `UnresolvedCall(String)`, `UsesType`, `References`, `Contains`, `Implements`, `ExternalCall(String, String)`
  - Define `EdgeData` struct: `kind: EdgeKind`, `source_span: Span`, `weight: f32` (for ranking)
  - Define `Span` struct: `file: PathBuf`, `start_line: u32`, `start_col: u32`, `end_line: u32`, `end_col: u32`, `byte_range: Range<usize>`
  - Define `Visibility` enum: `Public`, `Crate`, `Super`, `Private`
  - Implement `PartialEq`, `Eq`, `Hash` on `NodeId`
  - Implement `Serialize`/`Deserialize` on all types (for persistence)
  - Write unit tests: create nodes, verify ID determinism, verify edge construction

  **Must NOT do**:
  - Do NOT add more than 5 node kinds (no Import, Export, Variable, Constant, TypeAlias)
  - Do NOT add builder patterns (direct construction is fine)
  - Do NOT add Display impls for internal types

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: [`rust-style`]
    - `rust-style`: Enum design, derive macros, idiomatic type definitions

  **Parallelization**:
  - **Can Run In Parallel**: YES (after Task 1)
  - **Parallel Group**: Wave 1 (with Tasks 3, 4, 5)
  - **Blocks**: Tasks 6, 10, 11
  - **Blocked By**: Task 1

  **References**:
  - `crates/jfc-graph/research/arbor/crates/arbor-graph/src/lib.rs` — Arbor's node/edge schema for reference
  - `crates/jfc-graph/research/joern/joern-cli/frontends/x2cpg/src/main/scala/io/joern/x2cpg/Defines.scala` — Joern CPG node types
  - `crates/jfc-graph/research/arbor/docs/GRAPH_SCHEMA.md` — Arbor's complete schema doc
  - Joern CPG docs (fetched): Node types METHOD, TYPE_DECL, MEMBER; Edge types CALL, AST, REACHING_DEF

  **Acceptance Criteria**:
  - [ ] `cargo test -p jfc-graph -- node` passes with tests for node creation and ID determinism
  - [ ] `NodeId::new("src/main.rs", "foo::bar", NodeKind::Function) == NodeId::new("src/main.rs", "foo::bar", NodeKind::Function)` (deterministic)
  - [ ] All types derive `Serialize, Deserialize`
  - [ ] Exactly 5 variants in `NodeKind` enum (grep-verifiable)

  **QA Scenarios**:
  ```
  Scenario: Node ID is deterministic
    Tool: Bash
    Preconditions: Types defined and test written
    Steps:
      1. Run `cargo test -p jfc-graph -- test_node_id_deterministic --nocapture`
      2. Assert exit code 0
      3. Test body: create same NodeId twice, assert_eq
    Expected Result: Test passes, same inputs produce same ID
    Failure Indicators: Test failure, assertion panic
    Evidence: .sisyphus/evidence/task-2-node-id-deterministic.txt

  Scenario: Edge kinds cover all required variants
    Tool: Bash
    Preconditions: Edge types defined
    Steps:
      1. Run `grep -c "Calls\|UnresolvedCall\|UsesType\|References\|Contains\|Implements\|ExternalCall" crates/jfc-graph/src/edges.rs`
      2. Assert all 7 edge kinds present
    Expected Result: All 7 EdgeKind variants exist
    Failure Indicators: Missing variant
    Evidence: .sisyphus/evidence/task-2-edge-kinds.txt
  ```

  **Commit**: YES (groups with 3, 4, 5)
  - Message: `feat(graph): define node/edge types, traversal, adapter trait, fixtures`
  - Files: `src/nodes.rs`, `src/edges.rs`
  - Pre-commit: `cargo test -p jfc-graph`

- [x] 3. Test Fixtures

  **What to do**:
  - Create `crates/jfc-graph/tests/fixtures/sample.rs` — a Rust file with known structure:
    - 3 functions (`foo`, `bar`, `baz`), `foo` calls `bar`, `bar` calls `baz`
    - 1 struct `Config` with 4 fields
    - 1 enum `Status` with 3 variants
    - 1 trait `Processor` with 2 methods
    - 1 impl block implementing `Processor` for `Config`
  - Create `crates/jfc-graph/tests/fixtures/mutual_recursion.rs`:
    - `fn ping() { pong() }` and `fn pong() { ping() }`
  - Create `crates/jfc-graph/tests/fixtures/deep_call_chain.rs`:
    - Chain of 10 functions: `a` calls `b` calls `c` ... calls `j`
  - Create `crates/jfc-graph/tests/fixtures/partial_struct.rs`:
    - Struct with 8 fields, function that only accesses 2 of them
  - Create `crates/jfc-graph/tests/fixtures/multi_file/mod.rs` + `multi_file/helper.rs`:
    - Cross-file function calls for workspace-spanning tests
  - Document expected node/edge counts for each fixture in comments at top of file

  **Must NOT do**:
  - Do NOT make fixtures overly complex — they're for deterministic testing
  - Do NOT use external crate dependencies in fixtures (pure Rust, no imports)

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: [`rust-style`]
    - `rust-style`: Clean Rust code patterns for test fixtures

  **Parallelization**:
  - **Can Run In Parallel**: YES (after Task 1)
  - **Parallel Group**: Wave 1 (with Tasks 2, 4, 5)
  - **Blocks**: Tasks 7, 8
  - **Blocked By**: Task 1

  **References**:
  - `crates/jfc-graph/research/arbor/crates/arbor-cli/tests/` — Arbor's integration test patterns

  **Acceptance Criteria**:
  - [ ] All fixture files exist and are valid Rust (pass `rustfmt --check`)
  - [ ] `sample.rs` has exactly: 3 functions, 1 struct (4 fields), 1 enum (3 variants), 1 trait (2 methods), 1 impl
  - [ ] `mutual_recursion.rs` has exactly 2 mutually recursive functions
  - [ ] Expected counts documented in comments at top of each file

  **QA Scenarios**:
  ```
  Scenario: Fixtures are valid Rust
    Tool: Bash
    Preconditions: Fixture files created
    Steps:
      1. Run `rustfmt --check crates/jfc-graph/tests/fixtures/sample.rs`
      2. Run `rustfmt --check crates/jfc-graph/tests/fixtures/mutual_recursion.rs`
      3. Assert both exit 0
    Expected Result: All fixtures pass rustfmt check
    Failure Indicators: Non-zero exit code, formatting diff
    Evidence: .sisyphus/evidence/task-3-fixtures-valid.txt
  ```

  **Commit**: YES (groups with 2, 4, 5)
  - Message: (grouped with Task 2)
  - Files: `tests/fixtures/*.rs`
  - Pre-commit: `rustfmt --check crates/jfc-graph/tests/fixtures/*.rs`

- [x] 4. Language Adapter Trait Definition

  **What to do**:
  - Define `trait LanguageAdapter` in `src/adapter/mod.rs`:
    ```rust
    pub trait LanguageAdapter: Send + Sync {
        fn language_id(&self) -> &str;
        fn file_extensions(&self) -> &[&str];
        fn parse_file(&self, path: &Path, content: &str) -> Result<ParsedFile>;
        fn extract_nodes(&self, parsed: &ParsedFile) -> Vec<NodeData>;
        fn extract_edges(&self, parsed: &ParsedFile, nodes: &[NodeData]) -> Vec<(NodeId, NodeId, EdgeData)>;
    }
    ```
  - Define `ParsedFile` struct: tree-sitter `Tree` + source text + file path
  - Define `AdapterRegistry` to hold registered adapters and select by file extension
  - Write unit test: verify trait object creation works (`Box<dyn LanguageAdapter>`)

  **Must NOT do**:
  - Do NOT implement the Rust adapter here (that's Task 7)
  - Do NOT add adapters for TypeScript, Python, etc.

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: [`rust-style`]
    - `rust-style`: Trait-based design, dynamic dispatch patterns

  **Parallelization**:
  - **Can Run In Parallel**: YES (after Task 1)
  - **Parallel Group**: Wave 1 (with Tasks 2, 3, 5)
  - **Blocks**: Tasks 7, 8
  - **Blocked By**: Task 1

  **References**:
  - `crates/jfc-graph/research/arbor/crates/arbor-core/src/lib.rs` — Arbor's parser trait design
  - `crates/jfc-graph/research/joern/joern-cli/frontends/x2cpg/` — Joern's X2CPG adapter pattern
  - `crates/jfc-graph/research/tree-sitter-graph/` — tree-sitter-graph's approach to language-agnostic graph construction

  **Acceptance Criteria**:
  - [ ] `trait LanguageAdapter` compiles with all 5 methods
  - [ ] `AdapterRegistry` can register and lookup adapters by extension
  - [ ] Test: create mock adapter, register, lookup by ".rs" extension

  **QA Scenarios**:
  ```
  Scenario: Adapter registry dispatches correctly
    Tool: Bash
    Preconditions: Trait and registry defined
    Steps:
      1. Run `cargo test -p jfc-graph -- test_adapter_registry --nocapture`
      2. Assert exit code 0
    Expected Result: Mock adapter registered and retrieved by extension
    Failure Indicators: Test panic, wrong adapter returned
    Evidence: .sisyphus/evidence/task-4-adapter-registry.txt
  ```

  **Commit**: YES (groups with 2, 3, 5)
  - Message: (grouped with Task 2)
  - Files: `src/adapter/mod.rs`
  - Pre-commit: `cargo test -p jfc-graph`

- [x] 5. Traversal Algorithms with Cycle Detection

  **What to do**:
  - Implement `TraversalConfig` struct: `max_depth: usize`, `max_nodes: usize`, `direction: Direction` (Incoming/Outgoing/Both)
  - Implement `traverse()` function that:
    - Takes a start node, graph reference, config, and edge filter predicate
    - Maintains `HashSet<NodeId>` of visited nodes (CYCLE DETECTION — HARD REQUIREMENT)
    - Respects `max_depth` (stops expansion beyond configured depth)
    - Respects `max_nodes` (caps total collected nodes for token budget)
    - Returns `TraversalResult`: collected nodes + edges + metadata (depth_reached, was_truncated, cycle_detected_at)
  - Implement `find_path()`: shortest path between two nodes (BFS)
  - Implement `subgraph()`: extract connected component containing a node up to depth N
  - Write tests:
    - Cycle detection test using `mutual_recursion.rs` graph structure (construct manually)
    - Depth limiting test using `deep_call_chain.rs` graph structure
    - Max nodes truncation test

  **Must NOT do**:
  - Do NOT make traversal async (it's purely in-memory, no I/O)
  - Do NOT implement pagerank/centrality (that's a capability extension, not core traversal)

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: [`rust-style`]
    - `rust-style`: Iterator-based algorithms, zero-cost abstractions

  **Parallelization**:
  - **Can Run In Parallel**: YES (after Task 1)
  - **Parallel Group**: Wave 1 (with Tasks 2, 3, 4)
  - **Blocks**: Task 12
  - **Blocked By**: Task 1

  **References**:
  - `crates/jfc-graph/research/petgraph/crates/petgraph/src/visit/` — petgraph's built-in traversal algorithms (Dfs, Bfs)
  - `crates/jfc-graph/research/petgraph/crates/petgraph/src/algo/` — Shortest path algorithms (dijkstra, astar, etc.)
  - `crates/jfc-graph/research/arbor/crates/arbor-graph/src/` — Arbor's graph module (reference for traversal patterns)

  **Acceptance Criteria**:
  - [ ] `cargo test -p jfc-graph -- test_cycle_detection` — traversal terminates on mutual recursion graph
  - [ ] `cargo test -p jfc-graph -- test_depth_limit` — traversal stops at configured depth (depth=2 on 10-chain returns exactly 3 nodes)
  - [ ] `cargo test -p jfc-graph -- test_max_nodes` — traversal truncates and sets `was_truncated=true`
  - [ ] `TraversalResult.cycle_detected_at` populated when cycle found

  **QA Scenarios**:
  ```
  Scenario: Mutual recursion terminates
    Tool: Bash
    Preconditions: Traversal implemented, test graph with cycle built
    Steps:
      1. Run `cargo test -p jfc-graph -- test_cycle_detection --nocapture`
      2. Assert exit code 0
      3. Verify output shows cycle_detected_at is Some(node_id)
    Expected Result: Traversal completes in finite time, reports cycle
    Failure Indicators: Timeout (>5s), stack overflow, test panic
    Evidence: .sisyphus/evidence/task-5-cycle-detection.txt

  Scenario: Depth limit respected
    Tool: Bash
    Preconditions: 10-node linear chain graph built
    Steps:
      1. Run `cargo test -p jfc-graph -- test_depth_limit --nocapture`
      2. Assert test creates chain a→b→c→...→j, starts at a with depth=2
      3. Assert result contains exactly {a, b, c} (3 nodes)
    Expected Result: Only nodes within depth=2 are collected
    Failure Indicators: More than 3 nodes returned, depth not respected
    Evidence: .sisyphus/evidence/task-5-depth-limit.txt
  ```

  **Commit**: YES (groups with 2, 3, 4)
  - Message: (grouped with Task 2)
  - Files: `src/traversal.rs`
  - Pre-commit: `cargo test -p jfc-graph`

- [x] 6. CodeGraph Struct (petgraph wrapper + CRUD)

  **What to do**:
  - Create `CodeGraph` struct wrapping `petgraph::DiGraph<NodeData, EdgeData>`
  - Maintain side-maps: `HashMap<NodeId, petgraph::NodeIndex>` for O(1) lookup by ID
  - Implement methods:
    - `add_node(&mut self, data: NodeData) -> NodeId`
    - `add_edge(&mut self, from: NodeId, to: NodeId, data: EdgeData) -> Result<()>`
    - `get_node(&self, id: &NodeId) -> Option<&NodeData>`
    - `get_edges_from(&self, id: &NodeId) -> Vec<(&NodeId, &EdgeData)>`
    - `get_edges_to(&self, id: &NodeId) -> Vec<(&NodeId, &EdgeData)>`
    - `remove_node(&mut self, id: &NodeId)` — also removes connected edges
    - `nodes_by_kind(&self, kind: NodeKind) -> Vec<&NodeData>`
    - `find_by_name(&self, name: &str) -> Vec<&NodeData>` — substring match
    - `node_count(&self) -> usize`
    - `edge_count(&self) -> usize`
  - Implement `Default` for empty graph
  - Write tests: add nodes/edges, lookup, removal, duplicate handling

  **Must NOT do**:
  - Do NOT implement serialization here (persistence is Task 15)
  - Do NOT add any traversal logic (that's Task 5)

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 7-11)
  - **Blocks**: Tasks 9, 11, 12, 13, 15, 16
  - **Blocked By**: Task 2

  **References**:
  - `crates/jfc-graph/research/petgraph/crates/petgraph/src/graph_impl/mod.rs` — petgraph's Graph API
  - `crates/jfc-graph/research/arbor/crates/arbor-graph/src/` — Arbor's graph wrapper (reference for wrapper design)

  **Acceptance Criteria**:
  - [ ] `cargo test -p jfc-graph -- test_graph_add_node` — add 5 nodes, verify count
  - [ ] `cargo test -p jfc-graph -- test_graph_add_edge` — add edges, verify connectivity
  - [ ] `cargo test -p jfc-graph -- test_graph_lookup_by_name` — find_by_name returns correct results
  - [ ] `cargo test -p jfc-graph -- test_graph_remove_node` — removal cascades to edges

  **QA Scenarios**:
  ```
  Scenario: Graph maintains integrity on node removal
    Tool: Bash
    Steps:
      1. Run `cargo test -p jfc-graph -- test_graph_remove_node --nocapture`
      2. Assert: after removing node B connected to A→B→C, edges A→B and B→C are gone
    Expected Result: Node and all connected edges removed, graph consistent
    Evidence: .sisyphus/evidence/task-6-node-removal.txt
  ```

  **Commit**: YES (groups with 7-11)
  - Message: `feat(graph): core engine — graph struct, tree-sitter adapter, DSL parser, symbols`
  - Pre-commit: `cargo test -p jfc-graph`

- [x] 7. Tree-sitter Rust Adapter — Function/Struct Extraction

  **What to do**:
  - Implement `RustAdapter` struct implementing `LanguageAdapter` trait
  - Use `tree-sitter` + `tree-sitter-rust` to parse Rust source files
  - Extract from AST:
    - `function_item` → `NodeKind::Function` (name, visibility, span, parameters)
    - `struct_item` → `NodeKind::Struct` (name, fields with types)
    - `enum_item` → `NodeKind::Enum` (name, variants)
    - `mod_item` → `NodeKind::Module` (name, inline vs file)
    - `trait_item` → `NodeKind::Trait` (name, methods as sub-nodes)
    - `impl_item` → generates `Contains` edges from struct→functions
  - Build qualified names by walking parent scope (module nesting)
  - Handle `pub`, `pub(crate)`, `pub(super)` visibility
  - Test against `sample.rs` fixture: verify exact node count

  **Must NOT do**:
  - Do NOT extract call sites here (that's Task 8)
  - Do NOT resolve names cross-file (that's LSP enrichment, Task 22)

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Task 6, 10, 11)
  - **Parallel Group**: Wave 2
  - **Blocks**: Tasks 8, 9, 13
  - **Blocked By**: Tasks 3, 4

  **References**:
  - `crates/jfc-graph/research/tree-sitter/lib/` — tree-sitter C library (Rust bindings)
  - `crates/jfc-graph/research/arbor/crates/arbor-core/src/languages/rust.rs` — Arbor's Rust extractor (verified exists)
  - `crates/jfc-graph/research/stack-graphs/stack-graphs/src/` — GitHub's stack graph core (name resolution reference)
  - Tree-sitter Rust grammar node types: `function_item`, `struct_item`, `enum_item`, `mod_item`, `trait_item`, `impl_item`

  **Acceptance Criteria**:
  - [ ] `cargo test -p jfc-graph -- test_rust_extract_sample` — parses `sample.rs`, finds exactly 3 Functions, 1 Struct, 1 Enum, 1 Trait, 1 Module (if in fixture)
  - [ ] Qualified names include module path (e.g., `sample::foo`)
  - [ ] Visibility correctly detected for each node
  - [ ] Struct fields stored as metadata (name + type string)

  **QA Scenarios**:
  ```
  Scenario: Parse sample.rs fixture and extract correct nodes
    Tool: Bash
    Steps:
      1. Run `cargo test -p jfc-graph -- test_rust_extract_sample --nocapture`
      2. Verify output lists: 3 functions (foo, bar, baz), 1 struct (Config), 1 enum (Status), 1 trait (Processor)
    Expected Result: Exact node count and names match fixture's documented expectations
    Evidence: .sisyphus/evidence/task-7-sample-extraction.txt
  ```

  **Commit**: YES (groups with 6, 8-11)
  - Pre-commit: `cargo test -p jfc-graph`

- [x] 8. Tree-sitter Rust Adapter — Call Site + Edge Detection

  **What to do**:
  - Extend `RustAdapter::extract_edges()` to detect:
    - `call_expression` nodes → `EdgeKind::Calls` or `EdgeKind::UnresolvedCall(name)`
    - `type_identifier` in function params/return → `EdgeKind::UsesType`
    - `impl_item` with `for` clause → `EdgeKind::Implements`
    - Scope-aware matching: if callee name matches a function in the same file, create resolved `Calls` edge; otherwise `UnresolvedCall`
  - For each call site, record: caller node, callee name, call span (line/col)
  - Handle method calls (`self.method()`) — resolve within impl block
  - Handle chained calls — only the terminal function name matters for the edge
  - Test against `sample.rs`: verify `foo` calls `bar`, `bar` calls `baz` (exact edge count)

  **Must NOT do**:
  - Do NOT attempt cross-file name resolution (tree-sitter can't do it)
  - Do NOT track macro invocations as calls (too unreliable without expansion)

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (after Task 7)
  - **Parallel Group**: Wave 2 (later in wave, depends on 7)
  - **Blocks**: Task 9
  - **Blocked By**: Tasks 3, 4, 7

  **References**:
  - `crates/jfc-graph/research/arbor/crates/arbor-core/src/languages/rust.rs` — Arbor's call detection logic
  - Tree-sitter Rust AST: `call_expression` has `function` child (identifier or field_expression)
  - `crates/jfc-graph/research/tree-sitter-graph/` — Declarative graph construction from AST

  **Acceptance Criteria**:
  - [ ] `cargo test -p jfc-graph -- test_rust_call_edges` — detects foo→bar and bar→baz calls
  - [ ] `cargo test -p jfc-graph -- test_unresolved_calls` — call to unknown function creates UnresolvedCall edge
  - [ ] `cargo test -p jfc-graph -- test_impl_edges` — impl block creates Implements edge
  - [ ] Call edges include source span (line where call occurs)

  **QA Scenarios**:
  ```
  Scenario: Call chain detection in sample.rs
    Tool: Bash
    Steps:
      1. Run `cargo test -p jfc-graph -- test_rust_call_edges --nocapture`
      2. Verify: exactly 2 Calls edges found (foo→bar, bar→baz)
      3. Verify: each edge has non-zero source_span
    Expected Result: Call edges match fixture's documented call chain
    Evidence: .sisyphus/evidence/task-8-call-edges.txt
  ```

  **Commit**: YES (groups with 6, 7, 9-11)
  - Pre-commit: `cargo test -p jfc-graph`

- [x] 9. Graph Construction Pipeline

  **What to do**:
  - Create `GraphBuilder` struct that orchestrates: file discovery → parse → extract → build
  - Method `build_from_directory(path: &Path, adapter: &dyn LanguageAdapter) -> Result<CodeGraph>`
  - Method `build_from_files(files: &[PathBuf], adapter: &dyn LanguageAdapter) -> Result<CodeGraph>`
  - Steps: 1) list files by extension, 2) parse each file, 3) extract nodes, 4) extract edges, 5) insert all into CodeGraph
  - Handle errors gracefully: if one file fails to parse (syntax error), log warning and skip it
  - Workspace awareness: accept multiple directories (for workspace crates)
  - Tag each node with its crate name (extracted from directory structure or Cargo.toml)
  - Test: build graph of `tests/fixtures/` directory, verify counts match fixture documentation

  **Must NOT do**:
  - Do NOT make this async (file I/O is fast enough synchronously for typical projects)
  - Do NOT implement incremental updates here (that's Task 18)

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: NO (depends on 6, 7, 8)
  - **Parallel Group**: Wave 2 (end of wave)
  - **Blocks**: Tasks 12, 13, 14, 15, 17, 18, 22
  - **Blocked By**: Tasks 6, 7, 8

  **References**:
  - `crates/jfc-graph/research/arbor/crates/arbor-graph/src/builder.rs` — Arbor's graph builder (file exists, verified)
  - `crates/jfc-graph/research/arbor/docs/ARCHITECTURE.md` — Build pipeline architecture

  **Acceptance Criteria**:
  - [ ] `cargo test -p jfc-graph -- test_build_fixtures_dir` — builds graph from fixtures/, node+edge counts match
  - [ ] Files with syntax errors are skipped (not panic)
  - [ ] `build_from_directory("tests/fixtures/")` includes nodes from ALL fixture files

  **QA Scenarios**:
  ```
  Scenario: Build graph from fixtures directory
    Tool: Bash
    Steps:
      1. Run `cargo test -p jfc-graph -- test_build_fixtures_dir --nocapture`
      2. Verify: graph has nodes from sample.rs, mutual_recursion.rs, deep_call_chain.rs, partial_struct.rs
      3. Verify total node count matches sum of all fixture documented counts
    Expected Result: Complete graph built from all parseable fixtures
    Evidence: .sisyphus/evidence/task-9-build-pipeline.txt
  ```

  **Commit**: YES (groups with 6-8, 10-11)
  - Pre-commit: `cargo test -p jfc-graph`

- [x] 10. DSL Lexer + Parser

  **What to do**:
  - Define DSL grammar (≤8 operators):
    - `fn("name")` — select function node by name
    - `type("name")` — select struct/enum/trait by name
    - `callers` — get all callers of selected node(s)
    - `callees` — get all callees of selected node(s)
    - `depth N` — limit traversal to N levels
    - `filter kind=X` — filter results by node kind
    - `show fields|signature|body` — control output projection
    - `taint "var_name"` — trace data propagation (implemented in Task 24, parsed here)
  - Pipe operator `|` chains operations: `fn("foo") | callees | depth 2 | show signature`
  - Implement lexer: tokenize DSL string into Token enum (Ident, String, Pipe, Number, Keyword)
  - Implement parser: produce `Vec<DslOp>` from token stream
  - Define `DslOp` enum matching the 7 operators
  - Handle parse errors: return structured error with position and suggestion
  - Write tests: parse valid queries, reject malformed queries with good errors

  **Must NOT do**:
  - Do NOT add variables, loops, conditionals, or assignment
  - Do NOT add more than 7 operators
  - Do NOT use a parser generator (nom, pest, etc.) — hand-roll for simplicity and control
  - Do NOT implement execution here (that's Task 12)

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 6-9, 11)
  - **Parallel Group**: Wave 2
  - **Blocks**: Task 12
  - **Blocked By**: Task 2

  **References**:
  - Joern CPGQL syntax (fetched): traversal steps like `.method`, `.call`, `.argument`, `.reachableBy`
  - Arbor's MCP tool definitions — simpler predefined operations vs our compositional DSL
  - DSL syntax from user discussion: `select fn("parse_message") | callsites | where uses_type("Buffer") | depth 2`

  **Acceptance Criteria**:
  - [ ] `cargo test -p jfc-graph -- test_dsl_parse_simple` — `fn("foo") | callees` parses to [SelectFn("foo"), Callees]
  - [ ] `cargo test -p jfc-graph -- test_dsl_parse_full` — full pipe chain with all operators parses correctly
  - [ ] `cargo test -p jfc-graph -- test_dsl_parse_error` — malformed input returns error with position
  - [ ] Exactly 8 DslOp variants (verified by grep)

  **QA Scenarios**:
  ```
  Scenario: DSL parses valid query
    Tool: Bash
    Steps:
      1. Run `cargo test -p jfc-graph -- test_dsl_parse_simple --nocapture`
      2. Verify: input `fn("execute_tool") | callees | depth 2` produces 3-element op vector
    Expected Result: Correct DslOp sequence
    Evidence: .sisyphus/evidence/task-10-dsl-parse.txt

  Scenario: DSL rejects invalid query with helpful error
    Tool: Bash
    Steps:
      1. Run `cargo test -p jfc-graph -- test_dsl_parse_error --nocapture`
      2. Verify: input `fn() | invalid_op` returns ParseError with position=7 and suggestion
    Expected Result: Structured error, not panic
    Evidence: .sisyphus/evidence/task-10-dsl-error.txt
  ```

  **Commit**: YES (groups with 6-9, 11)
  - Pre-commit: `cargo test -p jfc-graph`

- [x] 11. Symbol Table (Handle → Location Mapping)

  **What to do**:
  - Create `SymbolTable` struct: `HashMap<SymbolHandle, SymbolEntry>`
  - `SymbolHandle` — short opaque string (e.g., `"fn:foo::bar"`, `"struct:Config"`)
  - `SymbolEntry` — `{ node_id: NodeId, file_path: PathBuf, span: Span, qualified_name: String, kind: NodeKind }`
  - Method `build_from_graph(graph: &CodeGraph) -> SymbolTable` — generates handles for all nodes
  - Method `resolve(&self, handle: &str) -> Option<&SymbolEntry>` — lookup by handle
  - Method `resolve_fuzzy(&self, partial: &str) -> Vec<&SymbolEntry>` — fuzzy match for LLM use
  - Method `invalidate_file(&mut self, path: &Path)` — remove all entries for a file (re-parse trigger)
  - Method `update_from_graph(&mut self, graph: &CodeGraph, changed_file: &Path)` — incremental update
  - Handle generation is deterministic and human-readable (LLM will see these in tool output)
  - Write tests: build table from test graph, resolve handles, fuzzy match

  **Must NOT do**:
  - Do NOT implement actual file editing here (that's Task 20)
  - Do NOT store file content (just coordinates)

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 6-10)
  - **Parallel Group**: Wave 2
  - **Blocks**: Tasks 14, 18, 20
  - **Blocked By**: Tasks 2, 6

  **References**:
  - `crates/jfc-graph/research/arbor/crates/arbor-graph/src/symbol_table.rs` — Arbor's symbol resolution (verified exists)
  - Discussion notes: "The LLM never deals with file paths or line numbers, it just says edit symbol"

  **Acceptance Criteria**:
  - [ ] `cargo test -p jfc-graph -- test_symbol_table_build` — builds table from graph, all nodes have handles
  - [ ] `cargo test -p jfc-graph -- test_symbol_resolve` — exact handle resolves to correct entry
  - [ ] `cargo test -p jfc-graph -- test_symbol_fuzzy` — partial "foo" matches "fn:sample::foo"
  - [ ] `cargo test -p jfc-graph -- test_symbol_invalidate` — after invalidate_file, entries for that file are gone

  **QA Scenarios**:
  ```
  Scenario: Symbol handles are human-readable and resolvable
    Tool: Bash
    Steps:
      1. Run `cargo test -p jfc-graph -- test_symbol_resolve --nocapture`
      2. Verify: handle "fn:sample::foo" resolves to NodeKind::Function with correct span
    Expected Result: Bidirectional mapping works (node→handle, handle→node)
    Evidence: .sisyphus/evidence/task-11-symbol-resolve.txt
  ```

  **Commit**: YES (groups with 6-10)
  - Pre-commit: `cargo test -p jfc-graph`

- [x] 12. DSL Executor + Query Engine

  **What to do**:
  - Implement `QueryEngine` struct holding reference to `CodeGraph` and `SymbolTable`
  - Implement `execute(&self, ops: &[DslOp], config: &QueryConfig) -> Result<QueryResult>`
  - `QueryConfig`: `max_tokens: usize`, `max_nodes: usize`, `include_body: bool`
  - Each `DslOp` transforms a `NodeSet` (HashSet of NodeIds):
    - `SelectFn(name)` → filter graph for Function nodes matching name
    - `SelectType(name)` → filter graph for Struct/Enum/Trait matching name
    - `Callers` → for each node in set, get all incoming Calls edges
    - `Callees` → for each node in set, get all outgoing Calls edges
    - `Depth(n)` → expand set by traversing n levels (uses Task 5 traversal with cycle detection!)
    - `Filter(kind)` → filter set to only matching NodeKind
    - `Show(projection)` → control what metadata is included in output
  - `QueryResult`: nodes with their metadata, edges between them, truncation info
  - Pipe semantics: output of one op becomes input set for next op
  - Token budget: if result exceeds max_tokens, truncate with "...N more nodes" indicator
  - Write comprehensive tests: execute multi-step queries against fixture graph

  **Must NOT do**:
  - Do NOT format output for display here (just structured data)
  - Do NOT implement caching (premature optimization)

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: NO (depends on 5, 6, 9, 10)
  - **Parallel Group**: Wave 3 lead
  - **Blocks**: Tasks 17, 19, 23
  - **Blocked By**: Tasks 5, 6, 9, 10

  **References**:
  - `crates/jfc-graph/src/traversal.rs` (Task 5) — traverse() function with cycle detection
  - `crates/jfc-graph/src/dsl/mod.rs` (Task 10) — DslOp enum
  - `crates/jfc-graph/src/graph.rs` (Task 6) — CodeGraph API

  **Acceptance Criteria**:
  - [ ] `cargo test -p jfc-graph -- test_query_fn_callees` — `fn("foo") | callees` returns {bar}
  - [ ] `cargo test -p jfc-graph -- test_query_depth` — `fn("foo") | callees | depth 2` returns {bar, baz}
  - [ ] `cargo test -p jfc-graph -- test_query_cycle_safe` — query on mutual_recursion graph terminates
  - [ ] `cargo test -p jfc-graph -- test_query_max_tokens` — truncates when exceeding budget
  - [ ] DSL executor uses traversal's cycle detection (not reimplementing)

  **QA Scenarios**:
  ```
  Scenario: Multi-step query executes correctly
    Tool: Bash
    Steps:
      1. Build graph from sample.rs fixture
      2. Execute: fn("foo") | callees | depth 2
      3. Assert result contains nodes: bar, baz
      4. Assert result does NOT contain foo (outgoing only)
    Expected Result: Correct transitive callees up to depth 2
    Evidence: .sisyphus/evidence/task-12-query-callees.txt

  Scenario: Query on cyclic graph terminates
    Tool: Bash
    Steps:
      1. Build graph from mutual_recursion.rs fixture
      2. Execute: fn("ping") | callees | depth 10
      3. Assert: terminates in <1s
      4. Assert: result.truncation_info contains cycle indicator
    Expected Result: Terminates with cycle detection, doesn't infinite loop
    Evidence: .sisyphus/evidence/task-12-query-cycle.txt
  ```

  **Commit**: YES (groups with 13-18)
  - Message: `feat(graph): features — DSL executor, partial struct, validation, persistence, capabilities`
  - Pre-commit: `cargo test -p jfc-graph`

- [x] 13. Partial Struct Selection (Field-Level Granularity)

  **What to do**:
  - Add `fields: Vec<FieldInfo>` to Struct node metadata
  - `FieldInfo`: `name: String`, `type_str: String`, `span: Span`, `is_public: bool`
  - Implement `PartialView` struct: subset of fields with metadata flag `is_partial: bool`
  - During edge extraction (Task 8 extension): track which fields are accessed per call site
    - `field_expression` in tree-sitter → record `accessed_fields: HashSet<String>` on UsesType edge
  - Method `get_partial_struct(graph: &CodeGraph, struct_id: NodeId, accessing_fn: NodeId) -> PartialView`
    - Returns only fields actually accessed by that function
    - Includes `verbose: bool` flag (if true, show all fields with accessed ones highlighted)
  - DSL integration: `show fields` on a struct node respects partial view based on caller context
  - Test: `partial_struct.rs` fixture — verify only 2 of 8 fields selected

  **Must NOT do**:
  - Do NOT track field accesses through indirect references (e.g., `let x = s; x.field`)
  - Keep it simple: direct `struct.field` access only

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 12, 14-18)
  - **Parallel Group**: Wave 3
  - **Blocks**: None
  - **Blocked By**: Tasks 6, 7, 9

  **References**:
  - Discussion notes: "if it only uses A, then any input that uses A could be pulled instead of everything"
  - `crates/jfc-graph/tests/fixtures/partial_struct.rs` (Task 3) — test fixture with 8-field struct

  **Acceptance Criteria**:
  - [ ] `cargo test -p jfc-graph -- test_partial_struct` — struct with 8 fields, function accesses 2, partial view has 2
  - [ ] PartialView includes `is_partial: true` metadata
  - [ ] Verbose mode shows all 8 fields with 2 marked as accessed

  **QA Scenarios**:
  ```
  Scenario: Only accessed fields returned in partial view
    Tool: Bash
    Steps:
      1. Parse partial_struct.rs fixture
      2. Call get_partial_struct(Config, fn_that_uses_two_fields)
      3. Assert: returned PartialView has exactly 2 fields
      4. Assert: is_partial == true
    Expected Result: Field-level granularity works
    Evidence: .sisyphus/evidence/task-13-partial-struct.txt
  ```

  **Commit**: YES (groups with 12, 14-18)
  - Pre-commit: `cargo test -p jfc-graph`

- [x] 14. Virtual Edit Validation

  **What to do**:
  - Implement `VirtualValidator` struct
  - Method `validate_signature_change(graph: &CodeGraph, target: NodeId, new_signature: &str) -> ValidationResult`
  - Steps:
    1. Get all callers of target function (incoming Calls edges)
    2. Parse new signature (extract param count, param types)
    3. For each call site: check if argument count matches new param count
    4. For each call site: check if argument types are compatible (string matching, not full type system)
    5. Return `ValidationResult`: `compatible: Vec<NodeId>`, `incompatible: Vec<(NodeId, String)>` (reason)
  - Method `preview_edit(graph: &CodeGraph, target: NodeId, new_body: &str) -> Vec<AffectedCallSite>`
    - Returns list of call sites that MIGHT need updating
  - Handle: added params, removed params, renamed params, type changes
  - Test: edit `bar()` to take an extra param → all callers of bar flagged as incompatible

  **Must NOT do**:
  - Do NOT implement full type checking (no trait resolution, no lifetime checking)
  - Do NOT modify any files (validation only, actual editing is Task 20)
  - Do NOT use the LSP for validation (this must work offline)

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 12, 13, 15-18)
  - **Parallel Group**: Wave 3
  - **Blocks**: Task 20
  - **Blocked By**: Tasks 9, 11

  **References**:
  - Discussion notes: "inline the call site and then check each call site if this will work"
  - Metis review: "signature compat only, not full type-checking"

  **Acceptance Criteria**:
  - [ ] `cargo test -p jfc-graph -- test_validate_added_param` — adding param to bar(), callers flagged
  - [ ] `cargo test -p jfc-graph -- test_validate_compatible` — no-op change passes validation
  - [ ] ValidationResult includes specific call site locations and incompatibility reason

  **QA Scenarios**:
  ```
  Scenario: Signature change detected as incompatible
    Tool: Bash
    Steps:
      1. Build graph from sample.rs (foo calls bar)
      2. Validate: change bar() signature from `fn bar()` to `fn bar(x: i32)`
      3. Assert: foo flagged as incompatible caller (passes 0 args, needs 1)
    Expected Result: Incompatibility detected with clear reason
    Evidence: .sisyphus/evidence/task-14-validate-signature.txt
  ```

  **Commit**: YES (groups with 12, 13, 15-18)
  - Pre-commit: `cargo test -p jfc-graph`

- [x] 15. Event-Sourced Persistence

  **What to do**:
  - Define `GraphEvent` enum:
    - `NodeAdded(NodeData)`
    - `NodeRemoved(NodeId)`
    - `EdgeAdded(NodeId, NodeId, EdgeData)`
    - `EdgeRemoved(NodeId, NodeId, EdgeKind)`
    - `FileReindexed(PathBuf)` — marks a full re-parse of one file
  - Implement `EventLog` struct:
    - Append-only Vec of `(Timestamp, GraphEvent)`
    - Method `append(&mut self, event: GraphEvent)`
    - Method `snapshot(&self, graph: &CodeGraph) -> Snapshot` — serialize full graph state
    - Method `replay(snapshot: &Snapshot, events: &[GraphEvent]) -> CodeGraph` — reconstruct from snapshot + events
    - Method `undo(&mut self, graph: &mut CodeGraph)` — revert last event
    - Method `events_since(&self, timestamp: Timestamp) -> &[GraphEvent]`
  - `Snapshot`: bincode-serialized CodeGraph + timestamp
  - File-backed storage: snapshot as `.jfc-graph/snapshot.bin`, events as `.jfc-graph/events.binlog`
  - Auto-snapshot: after every 100 events, create new snapshot and truncate event log
  - Write tests: add events, snapshot, replay, verify equality; undo test

  **Must NOT do**:
  - Do NOT implement CRDT or distributed consensus
  - Do NOT make persistence mandatory (in-memory mode must still work without persistence)

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 12-14, 16-18)
  - **Parallel Group**: Wave 3
  - **Blocks**: None
  - **Blocked By**: Tasks 6, 9

  **References**:
  - Discussion: "BTRFS-like snapshotting: base snapshot + append-only diffs"
  - Event sourcing pattern: full state reconstructable from initial snapshot + ordered events

  **Acceptance Criteria**:
  - [ ] `cargo test -p jfc-graph -- test_event_log_replay` — snapshot + 5 events → replay produces identical graph
  - [ ] `cargo test -p jfc-graph -- test_event_undo` — undo last event, verify graph reverted
  - [ ] `cargo test -p jfc-graph -- test_auto_snapshot` — after 100 events, snapshot auto-created
  - [ ] Persistence files written to `.jfc-graph/` directory

  **QA Scenarios**:
  ```
  Scenario: Replay reconstructs graph from snapshot + events
    Tool: Bash
    Steps:
      1. Build graph, take snapshot at T=0
      2. Add 5 nodes (5 events)
      3. Replay from snapshot + events
      4. Assert: replayed graph == current graph (same node/edge counts)
    Expected Result: Perfect reconstruction
    Evidence: .sisyphus/evidence/task-15-replay.txt
  ```

  **Commit**: YES (groups with 12-14, 16-18)
  - Pre-commit: `cargo test -p jfc-graph`

- [x] 16. Modular Capability Tree

  **What to do**:
  - Define `Capability` enum: `CallGraph`, `TypeUsage`, `PartialStruct`, `VirtualValidation`, `Persistence`, `SymbolEditing`
  - Define `CapabilityTree` struct: tree of capabilities with enabled/disabled state
  - Method `is_enabled(&self, cap: Capability) -> bool`
  - Method `enable(&mut self, cap: Capability)` / `disable(&mut self, cap: Capability)`
  - Dependency relationships: `VirtualValidation` requires `CallGraph`; `PartialStruct` requires `TypeUsage`
  - If a dependency is disabled, dependents auto-disable with warning
  - Configuration: load from `jfc-graph.toml` or programmatic API
  - Default: all capabilities enabled
  - Test: disable CallGraph, verify VirtualValidation also disabled

  **Must NOT do**:
  - Do NOT implement hot-reload of capabilities (restart required)
  - Do NOT add plugin loading (static enum only)

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 12-15, 17-18)
  - **Parallel Group**: Wave 3
  - **Blocks**: None
  - **Blocked By**: Task 6

  **References**:
  - Issue #18: "Modular/tree-based capability extension system"
  - Discussion: "tree-based design for enabling/disabling analysis capabilities per project"

  **Acceptance Criteria**:
  - [ ] `cargo test -p jfc-graph -- test_capability_deps` — disabling CallGraph cascades to VirtualValidation
  - [ ] `cargo test -p jfc-graph -- test_capability_all_default` — all capabilities enabled by default
  - [ ] CapabilityTree is serializable (for config file)

  **QA Scenarios**:
  ```
  Scenario: Capability dependencies cascade
    Tool: Bash
    Steps:
      1. Create CapabilityTree with defaults (all enabled)
      2. Disable CallGraph
      3. Assert: VirtualValidation is now disabled
      4. Assert: warning message produced
    Expected Result: Dependents auto-disabled
    Evidence: .sisyphus/evidence/task-16-capability-cascade.txt
  ```

  **Commit**: YES (groups with 12-15, 17-18)
  - Pre-commit: `cargo test -p jfc-graph`

- [x] 17. Token-Budgeted Output Formatting

  **What to do**:
  - Implement `format_query_result(result: &QueryResult, budget: usize) -> FormattedOutput`
  - `FormattedOutput`: `text: String`, `token_count: usize`, `was_truncated: bool`, `nodes_shown: usize`, `nodes_total: usize`
  - Formatting strategy:
    1. For each node in result: format as `[handle] kind name (file:line)` + optional signature/body
    2. Estimate tokens (chars / 4 as rough estimate)
    3. If exceeds budget: truncate, add `"... and N more nodes (use 'depth 1' to reduce)"`
  - Projection modes from `show` DSL op:
    - `fields` — show struct fields only
    - `signature` — show function signature (params + return type)
    - `body` — show full function body (expensive)
  - Include edge information: show which nodes are connected and how
  - Test: format with small budget, verify truncation works

  **Must NOT do**:
  - Do NOT implement actual tokenizer (char/4 approximation is fine for v1)
  - Do NOT add syntax highlighting (that's a UI concern)

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 12-16, 18)
  - **Parallel Group**: Wave 3
  - **Blocks**: Task 19
  - **Blocked By**: Tasks 9, 12

  **References**:
  - `crates/jfc-ui/src/context.rs` — How context is currently formatted for the LLM (token awareness)
  - Metis review: "Graph query results respect a max_tokens parameter"

  **Acceptance Criteria**:
  - [ ] `cargo test -p jfc-graph -- test_format_truncation` — with budget=100, large result truncates with summary
  - [ ] `cargo test -p jfc-graph -- test_format_signature_mode` — show=signature includes param types
  - [ ] FormattedOutput.was_truncated == true when budget exceeded
  - [ ] Output includes symbol handles (for follow-up queries)

  **QA Scenarios**:
  ```
  Scenario: Output respects token budget
    Tool: Bash
    Steps:
      1. Query produces 20 nodes
      2. Format with budget=200 (enough for ~5 nodes)
      3. Assert: was_truncated == true
      4. Assert: output ends with "... and 15 more nodes"
    Expected Result: Clean truncation with count
    Evidence: .sisyphus/evidence/task-17-token-budget.txt
  ```

  **Commit**: YES (groups with 12-16, 18)
  - Pre-commit: `cargo test -p jfc-graph`

- [x] 18. Graph Incremental Update (Re-parse Single File)

  **What to do**:
  - Method `CodeGraph::update_file(&mut self, path: &Path, new_content: &str, adapter: &dyn LanguageAdapter) -> Vec<GraphEvent>`
  - Steps:
    1. Remove all nodes for `path` (and their edges)
    2. Re-parse file with adapter
    3. Re-extract nodes and edges
    4. Insert new nodes/edges
    5. Return list of GraphEvents for persistence
  - Method `SymbolTable::update_file(&mut self, path: &Path, graph: &CodeGraph)` — regenerate handles for changed file
  - Handle: file deleted (remove all nodes), file created (add all nodes), file modified (full re-parse of that file)
  - Performance: single-file re-parse should be <10ms (tree-sitter is sub-millisecond)
  - Emit events for persistence layer (Task 15)
  - Test: modify fixture file in-memory, update graph, verify correct delta

  **Must NOT do**:
  - Do NOT implement file watching (that's the UI's responsibility to trigger updates)
  - Do NOT implement diff-based update (full file re-parse is fast enough and simpler)

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 12-17)
  - **Parallel Group**: Wave 3
  - **Blocks**: Tasks 20, 23
  - **Blocked By**: Tasks 9, 11

  **References**:
  - Metis review: "Re-parse single edited file (tree-sitter is <1ms per file)"
  - `crates/jfc-graph/src/graph.rs` (Task 6) — remove_node() cascades to edges

  **Acceptance Criteria**:
  - [ ] `cargo test -p jfc-graph -- test_incremental_update` — modify function name, verify graph reflects change
  - [ ] `cargo test -p jfc-graph -- test_incremental_delete` — delete file, verify nodes removed
  - [ ] update_file() returns correct GraphEvent sequence
  - [ ] SymbolTable updated (old handles gone, new ones present)

  **QA Scenarios**:
  ```
  Scenario: File modification updates graph correctly
    Tool: Bash
    Steps:
      1. Build graph from sample.rs
      2. Modify sample.rs content (rename function foo→foo2)
      3. Call update_file("sample.rs", new_content)
      4. Assert: old node "fn:sample::foo" is gone
      5. Assert: new node "fn:sample::foo2" exists
      6. Assert: call edges updated (bar still called, from foo2)
    Expected Result: Graph correctly reflects single-file change
    Evidence: .sisyphus/evidence/task-18-incremental-update.txt
  ```

  **Commit**: YES (groups with 12-17)
  - Pre-commit: `cargo test -p jfc-graph`

- [x] 19. jfc-ui Tool Integration (ToolKind::GraphQuery)

  **What to do**:
  - Add `jfc-graph` as dependency in `crates/jfc-ui/Cargo.toml` (path = "../jfc-graph")
  - Add `GraphQuery` variant to `ToolKind` enum in `crates/jfc-ui/src/types.rs`
  - Add `ToolInput::GraphQuery { query: String, max_tokens: Option<usize> }` variant
  - Implement tool execution in `crates/jfc-ui/src/tools.rs`:
    1. Parse DSL string from `query` field
    2. Execute against the active `CodeGraph` (stored in app state)
    3. Format result with token budget
    4. Return formatted output as tool result
  - Add tool description to the tool list sent to LLM (short DSL syntax guide in description)
  - Initialize `CodeGraph` on app startup: build from workspace directory using `GraphBuilder`
  - Store graph in `AppState` (or similar global state)
  - Update graph when Edit/Write tools modify files (trigger `update_file()`)
  - Test: invoke GraphQuery tool with fixture, verify output

  **Must NOT do**:
  - Do NOT add auto-context yet (that's Task 23)
  - Do NOT add graph initialization UI (silent background build on startup)

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 20-23)
  - **Parallel Group**: Wave 4
  - **Blocks**: Tasks 20, 23
  - **Blocked By**: Tasks 12, 17

  **References**:
  - `crates/jfc-ui/src/tools.rs` — Existing tool dispatch pattern (match on ToolKind, execute, return result)
  - `crates/jfc-ui/src/types.rs:ToolKind` — Current tool enum variants
  - `crates/jfc-ui/src/types.rs:ToolInput` — Current tool input types
  - `crates/jfc-ui/src/context.rs` — How to access app state for the graph

  **Acceptance Criteria**:
  - [ ] `cargo build --workspace` compiles with jfc-graph dependency
  - [ ] GraphQuery appears in tool list sent to LLM
  - [ ] Invoking `graph_query` with `fn("execute_tool") | callees` returns actual callees from jfc-ui source
  - [ ] After an Edit tool modifies a file, subsequent graph_query reflects the change

  **QA Scenarios**:
  ```
  Scenario: GraphQuery tool returns results
    Tool: Bash
    Steps:
      1. Build workspace: `cargo build --workspace`
      2. Run integration test that invokes the GraphQuery tool handler directly
      3. Input: { query: "fn(\"foo\") | callees", max_tokens: 1000 }
      4. Assert: output contains function names from the queried graph
    Expected Result: Tool executes DSL query and returns formatted results
    Evidence: .sisyphus/evidence/task-19-tool-integration.txt

  Scenario: Graph updates after file edit
    Tool: Bash
    Steps:
      1. Build graph
      2. Simulate Edit tool modifying a file (add new function)
      3. Run graph_query for the new function
      4. Assert: new function found in graph
    Expected Result: Graph stays in sync with file changes
    Evidence: .sisyphus/evidence/task-19-graph-sync.txt
  ```

  **Commit**: YES (groups with 20-23)
  - Message: `feat(graph): integration — tool dispatch, semantic editing, LSP enrichment, auto-context`
  - Pre-commit: `cargo test --workspace`

- [x] 20. Symbol-Based Edit Tool

  **What to do**:
  - Add `SymbolEdit` variant to `ToolKind`: edit code by symbol handle instead of file:line
  - Input: `{ handle: String, new_content: String, validate: bool }`
  - Execution flow:
    1. Resolve handle → SymbolEntry (file path + span)
    2. If `validate=true`: run VirtualValidator on the change first
    3. If validation fails: return error with incompatible call sites listed
    4. If validation passes (or validate=false): apply edit to file at span coordinates
    5. Re-parse edited file (Task 18's update_file)
    6. Update symbol table
    7. Return: success + list of call sites that may need updating
  - The LLM receives: `"Successfully edited fn:sample::bar. 2 call sites may need updating: [fn:sample::foo at line 5, fn:other::baz at line 12]"`
  - Test: edit function via handle, verify file changed and graph updated

  **Must NOT do**:
  - Do NOT auto-fix call sites (just report them — LLM decides what to do)
  - Do NOT implement multi-symbol batch editing (one at a time)

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (after Task 19 exists)
  - **Parallel Group**: Wave 4
  - **Blocks**: None
  - **Blocked By**: Tasks 11, 14, 18, 19

  **References**:
  - `crates/jfc-ui/src/tools.rs` — Edit tool implementation (file-based editing)
  - Discussion: "edit symbol and the system resolves where that is"
  - `crates/jfc-graph/src/symbols.rs` (Task 11) — SymbolTable::resolve()
  - `crates/jfc-graph/src/validation.rs` (Task 14) — VirtualValidator

  **Acceptance Criteria**:
  - [ ] `cargo test --workspace -- test_symbol_edit` — edit function by handle, file modified correctly
  - [ ] Validation prevents broken edits when validate=true
  - [ ] Graph and symbol table reflect change after edit
  - [ ] Response includes affected call site list

  **QA Scenarios**:
  ```
  Scenario: Edit via symbol handle modifies file
    Tool: Bash
    Steps:
      1. Resolve handle "fn:sample::bar" → file path + span
      2. Apply edit: change "fn bar()" to "fn bar(x: i32)"
      3. Read file, assert new signature present at correct location
      4. Query graph for "fn:sample::bar" — verify updated signature
    Expected Result: File modified at correct location, graph in sync
    Evidence: .sisyphus/evidence/task-20-symbol-edit.txt

  Scenario: Validation rejects incompatible edit
    Tool: Bash
    Steps:
      1. Edit "fn:sample::bar" to add parameter, with validate=true
      2. Assert: returns error with list of incompatible callers
      3. Assert: file NOT modified (validation prevented it)
    Expected Result: Edit blocked, clear error message with call sites
    Evidence: .sisyphus/evidence/task-20-validation-reject.txt
  ```

  **Commit**: YES (groups with 19, 21-23)
  - Pre-commit: `cargo test --workspace`

- [x] 21. LSP Client Redesign (Request/Response Dispatch)

  **What to do**:
  - Redesign `LspClient` in `crates/jfc-ui/src/lsp_client.rs` to support request/response (not just notifications)
  - Add pending request map: `HashMap<RequestId, oneshot::Sender<JsonValue>>`
  - Implement `send_request(&self, method: &str, params: Value) -> Result<Value>`:
    1. Generate unique request ID
    2. Create oneshot channel
    3. Store sender in pending map
    4. Send JSON-RPC request
    5. Await response on receiver (with timeout)
  - Handle incoming messages: check if it's a response (has `id`), route to pending map
  - Add concrete methods:
    - `goto_definition(file: &Path, line: u32, col: u32) -> Vec<Location>`
    - `find_references(file: &Path, line: u32, col: u32) -> Vec<Location>`
    - `call_hierarchy_incoming(item: CallHierarchyItem) -> Vec<IncomingCall>`
    - `call_hierarchy_outgoing(item: CallHierarchyItem) -> Vec<OutgoingCall>`
    - `document_symbols(file: &Path) -> Vec<DocumentSymbol>`
  - Maintain backward compat: existing notification-only flow still works
  - Test: mock LSP server, send request, receive response

  **Must NOT do**:
  - Do NOT break existing diagnostic flow
  - Do NOT add lsp-types crate (hand-roll the types we need, matching current style)
  - Do NOT implement all LSP methods (just the 5 above)

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (independent of graph crate)
  - **Parallel Group**: Wave 4
  - **Blocks**: Task 22
  - **Blocked By**: None (but logically after graph engine works without LSP)

  **References**:
  - `crates/jfc-ui/src/lsp_client.rs` — Current LSP client (notification-only, fire-and-forget)
  - `crates/jfc-ui/src/lsp_rpc.rs` — JSON-RPC message framing
  - `crates/jfc-graph/research/lsp-types/src/` — Protocol type definitions reference
  - LSP spec: textDocument/definition, textDocument/references, callHierarchy/*

  **Acceptance Criteria**:
  - [ ] `cargo build --workspace` — compiles without breaking existing functionality
  - [ ] New request/response method works (tested with mock)
  - [ ] Existing diagnostic notifications still flow correctly
  - [ ] Timeout handling: returns error after 5s if no response

  **QA Scenarios**:
  ```
  Scenario: Request/response round-trip works
    Tool: Bash
    Steps:
      1. Run integration test with mock LSP server
      2. Send textDocument/definition request
      3. Mock responds with Location
      4. Assert: goto_definition() returns correct file/line
    Expected Result: Full request/response cycle completes
    Evidence: .sisyphus/evidence/task-21-lsp-request-response.txt
  ```

  **Commit**: YES (groups with 19, 20, 22, 23)
  - Pre-commit: `cargo test --workspace`

- [x] 22. LSP Enrichment Layer

  **What to do**:
  - Define `trait LspDataProvider` in `jfc-graph` (NOT depending on jfc-ui):
    ```rust
    pub trait LspDataProvider: Send + Sync {
        fn goto_definition(&self, file: &Path, line: u32, col: u32) -> Result<Vec<Location>>;
        fn find_references(&self, file: &Path, line: u32, col: u32) -> Result<Vec<Location>>;
        fn call_hierarchy_incoming(&self, file: &Path, line: u32, col: u32) -> Result<Vec<IncomingCall>>;
        fn call_hierarchy_outgoing(&self, file: &Path, line: u32, col: u32) -> Result<Vec<OutgoingCall>>;
    }
    ```
  - Implement `LspEnricher` struct in `jfc-graph` that accepts `&dyn LspDataProvider`
  - Method `enrich_call_edges(&mut self, graph: &mut CodeGraph, lsp: &dyn LspDataProvider)`:
    1. For each `UnresolvedCall(name)` edge in graph
    2. Use `goto_definition` at the call site span
    3. If definition found within workspace: resolve to actual NodeId, replace edge with `EdgeKind::Calls`
    4. If definition is external (outside workspace): replace with `ExternalCall(crate, path)`
  - Method `enrich_type_references(&mut self, graph: &mut CodeGraph, lsp: &dyn LspDataProvider)`:
    1. For type nodes: use `find_references` to discover additional UsesType edges missed by tree-sitter
  - In `jfc-ui`: implement `LspDataProvider` for `LspClient` (adapter pattern, avoids circular dep)
  - Enrichment is OPTIONAL and INCREMENTAL — graph works without it
  - Run enrichment in background after initial tree-sitter graph is built
  - Test: mock `LspDataProvider` impl, verify UnresolvedCall edges get resolved

  **Must NOT do**:
  - Do NOT import `jfc-ui` from `jfc-graph` (CIRCULAR DEPENDENCY — FORBIDDEN)
  - Do NOT block graph availability on LSP enrichment (async enhancement only)
  - Do NOT require LSP to be running (graceful degradation)

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (after Task 21)
  - **Parallel Group**: Wave 4
  - **Blocks**: None
  - **Blocked By**: Tasks 9, 21

  **References**:
  - `crates/jfc-graph/src/edges.rs` (Task 2) — EdgeKind::UnresolvedCall
  - `crates/jfc-ui/src/lsp_client.rs` (Task 21) — jfc-ui implements LspDataProvider for its LspClient
  - `crates/jfc-graph/research/lsp-types/src/` — Protocol type definitions reference
  - Metis review: "LSP enrichment deferred to Phase 3... Accept UnresolvedCall edges in early phases"

  **Acceptance Criteria**:
  - [ ] `cargo build --workspace` compiles (NO circular dependency)
  - [ ] `cargo test -p jfc-graph -- test_lsp_resolve_calls` — UnresolvedCall → Calls after enrichment (using mock provider)
  - [ ] `cargo test -p jfc-graph -- test_lsp_external` — external call correctly identified
  - [ ] Graph is queryable BEFORE enrichment runs (doesn't block)
  - [ ] If LspDataProvider returns error, enrichment silently skips that edge

  **QA Scenarios**:
  ```
  Scenario: UnresolvedCall edges get resolved by LSP
    Tool: Bash
    Steps:
      1. Build graph with tree-sitter (has UnresolvedCall edges for cross-file calls)
      2. Create mock LspDataProvider that returns definition locations
      3. Run enrich_call_edges(graph, &mock_provider)
      4. Assert: UnresolvedCall("helper_fn") → Calls(node_id_of_helper_fn)
    Expected Result: Cross-file calls resolved without circular dependency
    Evidence: .sisyphus/evidence/task-22-lsp-enrichment.txt

  Scenario: No circular dependency in workspace
    Tool: Bash
    Steps:
      1. Run `cargo build --workspace`
      2. Assert exit code 0
      3. Verify jfc-graph does NOT depend on jfc-ui: `grep "jfc-ui" crates/jfc-graph/Cargo.toml`
      4. Assert: zero matches
    Expected Result: Clean dependency direction (jfc-ui → jfc-graph, never reverse)
    Evidence: .sisyphus/evidence/task-22-no-circular-dep.txt
  ```

  **Commit**: YES (groups with 19-21, 23)
  - Pre-commit: `cargo test --workspace`

- [x] 23. Auto-Context Injection on Edit/Write Tools

  **What to do**:
  - When the LLM uses the `Edit` or `Write` tool to modify a file:
    1. After the edit is applied, identify which functions/structs were modified (by span overlap)
    2. Auto-run graph query: `fn("<modified_fn>") | callers | depth 1`
    3. Inject result into the NEXT system prompt as "Graph Context" section
    4. Include: "The following call sites use the function you just edited: [list with handles]"
  - Implementation location: hook into tool execution pipeline in `tools.rs`, post-execution
  - This is the "system auto-generates queries based on what the LLM is editing" feature
  - Configurable: can be disabled via CapabilityTree
  - Only inject if there ARE callers (don't inject empty context)
  - Respect token budget (auto-context capped at 500 tokens)
  - Test: simulate Edit tool, verify auto-context injected in next turn

  **Must NOT do**:
  - Do NOT inject context for trivial edits (comments, whitespace)
  - Do NOT inject if graph is not yet built (first-run case)
  - Do NOT exceed 500 token budget for auto-context

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (after Task 19 exists)
  - **Parallel Group**: Wave 4
  - **Blocks**: None
  - **Blocked By**: Tasks 12, 18, 19

  **References**:
  - `crates/jfc-ui/src/tools.rs` — Tool execution pipeline (where to hook)
  - `crates/jfc-ui/src/context.rs` — How system prompt/context is assembled
  - Discussion: "the system auto-generates context queries when the LLM does edits"
  - Metis review: "Token budget for graph outputs — Graph query results respect a max_tokens parameter"

  **Acceptance Criteria**:
  - [ ] After Edit tool modifies a function, next system prompt includes "Graph Context" section
  - [ ] Auto-context lists callers of the modified function with symbol handles
  - [ ] If no callers exist, no auto-context injected (no noise)
  - [ ] Auto-context ≤500 tokens
  - [ ] Disabled when CapabilityTree has auto-context off

  **QA Scenarios**:
  ```
  Scenario: Auto-context injected after function edit
    Tool: Bash
    Steps:
      1. Build graph of test project
      2. Simulate Edit tool modifying function "bar" (which is called by "foo")
      3. Check next system prompt assembly
      4. Assert: contains "Graph Context" section
      5. Assert: lists "fn:sample::foo" as caller of edited function
    Expected Result: Relevant callers automatically surfaced
    Evidence: .sisyphus/evidence/task-23-auto-context.txt

  Scenario: No injection for isolated function
    Tool: Bash
    Steps:
      1. Edit a function with zero callers
      2. Check next system prompt
      3. Assert: NO "Graph Context" section present
    Expected Result: No noise for isolated edits
    Evidence: .sisyphus/evidence/task-23-no-injection.txt
  ```

  **Commit**: YES (groups with 19-22)
  - Pre-commit: `cargo test --workspace`

- [x] 24. Taint/Propagation Analysis DSL Operator

  **What to do**:
  - Add 8th DSL operator: `taint` — traces how far a piece of data reaches through the code
  - Syntax: `fn("process_input") | taint "user_string" | depth 5`
  - Semantics: starting from a function, identify a named parameter/variable, then follow all downstream functions that receive or transform that data
  - Implementation:
    1. Start at selected function node
    2. Identify the named variable/parameter in that function's metadata
    3. Walk outgoing Calls edges where that variable (or a derivative) is passed as argument
    4. Recursively follow: if function B receives tainted data as param N, find where param N flows to in B's callees
    5. Collect all nodes touched by the tainted data
    6. Return: ordered list of (node, param_position, distance_from_source)
  - Uses traversal with cycle detection (Task 5) internally
  - Token budget respected on output
  - Test: fixture with data flowing through 4 functions, verify taint reaches all 4

  **Must NOT do**:
  - Do NOT implement inter-procedural alias analysis (too complex for v1)
  - Do NOT track through struct field assignments (direct param passing only)
  - Do NOT track through closures/callbacks

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 13-18)
  - **Parallel Group**: Wave 3
  - **Blocks**: None
  - **Blocked By**: Tasks 5, 6, 9, 12

  **References**:
  - `crates/jfc-graph/research/taint-slicing-llm-2501.15029.pdf` — Taint-based slicing achieving 99% code reduction
  - `crates/jfc-graph/src/traversal.rs` (Task 5) — Cycle-aware traversal with visited set
  - Discussion notes: "see how far piece of data reaches in the code... select a piece of data, get all the functions that accept it"
  - Joern CPG: REACHING_DEF edges for data flow tracking

  **Acceptance Criteria**:
  - [ ] `cargo test -p jfc-graph -- test_taint_basic` — data flows through 4 functions, all 4 found
  - [ ] `cargo test -p jfc-graph -- test_taint_cycle_safe` — taint on recursive code terminates
  - [ ] `cargo test -p jfc-graph -- test_taint_depth_limit` — respects depth parameter
  - [ ] DSL parser accepts `taint "var_name"` as 8th operator

  **QA Scenarios**:
  ```
  Scenario: Taint traces data through call chain
    Tool: Bash
    Steps:
      1. Build graph from fixture: fn a(input) calls b(input), b calls c(input), c calls d(input)
      2. Execute: fn("a") | taint "input" | depth 5
      3. Assert: result contains nodes a, b, c, d in order with distance 0,1,2,3
    Expected Result: Full taint chain discovered
    Evidence: .sisyphus/evidence/task-24-taint-trace.txt

  Scenario: Taint terminates on cycle
    Tool: Bash
    Steps:
      1. Build graph with mutual recursion passing same param
      2. Execute: fn("ping") | taint "x" | depth 10
      3. Assert: terminates, doesn't infinite loop
    Expected Result: Cycle detected, finite result
    Evidence: .sisyphus/evidence/task-24-taint-cycle.txt
  ```

  **Commit**: YES (groups with 12-18)
  - Pre-commit: `cargo test -p jfc-graph`

- [x] 25. Sub-Agent Cascade on Signature Change

  **What to do**:
  - When a symbol-based edit (Task 20) changes a function signature:
    1. Identify all call sites that are now incompatible (from VirtualValidator, Task 14)
    2. Group call sites by file (overlapping edits in same file → single agent)
    3. For each group, create a `CascadeTask` struct:
       - `original_edit: EditDescription` (what changed, why, how)
       - `affected_call_sites: Vec<CallSiteInfo>` (handle, file, span, current args)
       - `new_signature: String`
       - `instruction: String` (human-readable: "Update this call site to pass the new parameter X")
    4. Return cascade tasks to the orchestrating agent for dispatch
  - The graph engine does NOT execute the sub-agents itself — it produces the structured cascade description
  - jfc-ui's swarm system handles actual dispatch to sub-agents
  - If call sites overlap (same file, within 10 lines), unify into single task to prevent conflicts
  - Test: signature change on function with 3 callers in 2 files → produces 2 CascadeTasks

  **Must NOT do**:
  - Do NOT auto-execute the cascade (just produce the task descriptions)
  - Do NOT dispatch agents from within jfc-graph (that's jfc-ui's concern)
  - Do NOT handle merge conflicts (if two agents edit same file, that's a swarm concern)

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 22, 23, 26, 27)
  - **Parallel Group**: Wave 4
  - **Blocks**: Task 26
  - **Blocked By**: Tasks 14, 19, 20

  **References**:
  - `crates/jfc-ui/src/tools.rs` — Swarm/team system for dispatching sub-agents
  - Discussion: "for each call site, launch a sub-agent that receives the new edit and implements the changes"
  - Discussion: "if there is overlaps in the context... that should be unified and given to a single agent"
  - `crates/jfc-graph/src/validation.rs` (Task 14) — VirtualValidator identifying incompatible sites

  **Acceptance Criteria**:
  - [ ] `cargo test -p jfc-graph -- test_cascade_generation` — signature change produces CascadeTasks
  - [ ] `cargo test -p jfc-graph -- test_cascade_overlap_unify` — same-file call sites unified into one task
  - [ ] CascadeTask contains: original edit reason, new signature, affected call sites with current args
  - [ ] Number of cascade tasks ≤ number of affected files (unification works)

  **QA Scenarios**:
  ```
  Scenario: Cascade tasks generated for signature change
    Tool: Bash
    Steps:
      1. Build graph: fn bar() called by fn foo() in file1.rs and fn baz() in file2.rs
      2. Simulate signature change: bar() → bar(x: i32)
      3. Call generate_cascade(graph, bar_id, new_sig)
      4. Assert: 2 CascadeTasks produced (one per file)
      5. Assert: each task contains instruction + affected call site info
    Expected Result: Structured cascade ready for swarm dispatch
    Evidence: .sisyphus/evidence/task-25-cascade-gen.txt
  ```

  **Commit**: YES (groups with 19-23, 26, 27)
  - Pre-commit: `cargo test --workspace`

- [x] 26. Edit Reason Metadata Forwarding

  **What to do**:
  - Extend `GraphEvent` (Task 15) with `reason: Option<EditReason>` field
  - Define `EditReason` struct:
    - `description: String` (human-readable: "Added parameter x: i32 for connection pooling")
    - `original_context: String` (snippet of what the LLM was trying to achieve)
    - `parent_event_id: Option<EventId>` (links cascaded edits back to triggering edit)
    - `cascade_depth: u8` (0 = original edit, 1 = first cascade, 2 = cascade of cascade)
  - When a CascadeTask (Task 25) is executed, the resulting edit event carries:
    - `parent_event_id` pointing to the original edit
    - `cascade_depth` incremented
    - `description` summarizing what was changed and why
  - Method `EventLog::trace_cascade(event_id: EventId) -> Vec<GraphEvent>` — follow parent chain to see full cascade tree
  - This enables: "why was this call site changed?" → "because fn bar's signature was updated to add connection pooling"
  - Test: create edit chain (original → 2 cascades), verify trace_cascade returns full chain

  **Must NOT do**:
  - Do NOT enforce cascade depth limit (that's the orchestrator's concern)
  - Do NOT auto-generate description (the executing agent provides it)

  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (after Task 25)
  - **Parallel Group**: Wave 4
  - **Blocks**: None
  - **Blocked By**: Tasks 15, 20, 25

  **References**:
  - `crates/jfc-graph/src/persistence.rs` (Task 15) — GraphEvent enum and EventLog
  - Discussion: "the context under which the first edit occurred is carried forward so that any downstream edits know what changed, why changed, how changed"

  **Acceptance Criteria**:
  - [ ] `cargo test -p jfc-graph -- test_edit_reason_chain` — trace_cascade follows parent chain
  - [ ] GraphEvent serializes with reason field (backward compatible — Option)
  - [ ] cascade_depth correctly increments through chain

  **QA Scenarios**:
  ```
  Scenario: Edit reason chain is traceable
    Tool: Bash
    Steps:
      1. Create original edit event with reason "Added x param for pooling"
      2. Create cascade event with parent_event_id pointing to original
      3. Call trace_cascade(cascade_event_id)
      4. Assert: returns [cascade_event, original_event] in order
    Expected Result: Full provenance chain reconstructable
    Evidence: .sisyphus/evidence/task-26-reason-chain.txt
  ```

  **Commit**: YES (groups with 19-25, 27)
  - Pre-commit: `cargo test --workspace`

- [x] 27. TUI Graph Operation Inspectability

  **What to do**:
  - In jfc-ui, when a `graph_query` tool result is displayed:
    1. Each node in the result is a clickable/jumpable element
    2. Pressing a jump key (configurable, e.g., `g` for graph) on a node:
       - Opens the file at the node's span location (reuse existing file-open logic)
       - OR shows expanded node detail inline (signature, callers count, callees count)
    3. Each graph_query result stores a `QueryRecord` in session state:
       - `query_text: String`
       - `result_nodes: Vec<NodeId>`
       - `timestamp: Instant`
  - Add TUI element: "Graph History" panel (toggle with keybind)
    - Shows last N graph queries with results
    - Can re-run any previous query
  - When a CascadeTask (Task 25) is generated, show it in TUI as expandable tree:
    - Root: original edit
    - Children: affected call sites with their cascade tasks
    - Status: pending/running/complete per cascade item
  - Integration with existing jump/navigation system in jfc-ui

  **Must NOT do**:
  - Do NOT build a full graph visualization (no node-edge diagram rendering)
  - Do NOT add new TUI dependencies (use existing ratatui widgets)
  - Do NOT block on this for the graph engine to work (pure UI enhancement)

  **Recommended Agent Profile**:
  - **Category**: `visual-engineering`
  - **Skills**: [`rust-style`]

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Tasks 20-26)
  - **Parallel Group**: Wave 4
  - **Blocks**: None
  - **Blocked By**: Tasks 19, 12

  **References**:
  - `crates/jfc-ui/src/` — Existing TUI rendering and navigation system
  - Discussion: "you need to be able to go in your tree and select something it did and see the structure"
  - Discussion: "press a jump key and navigate to the different read/write things it does"

  **Acceptance Criteria**:
  - [ ] Graph query results show node handles as navigable elements
  - [ ] Jump key on a node opens the corresponding file at correct line
  - [ ] Graph History panel shows last 10 queries
  - [ ] Cascade tree displays in TUI when signature change generates cascade

  **QA Scenarios**:
  ```
  Scenario: Jump from graph result to source file
    Tool: Bash (integration test or tmux)
    Steps:
      1. Run graph_query producing result with node "fn:sample::foo"
      2. Verify result output includes navigable handle
      3. Simulate jump action on that handle
      4. Assert: file opened at correct line number
    Expected Result: Seamless navigation from graph result to code
    Evidence: .sisyphus/evidence/task-27-graph-jump.txt
  ```

  **Commit**: YES (groups with 19-26)
  - Pre-commit: `cargo test --workspace`

---

## Final Verification Wave (MANDATORY — after ALL implementation tasks)

> 4 review agents run in PARALLEL. ALL must APPROVE. Present consolidated results to user and get explicit "okay" before completing.

- [x] F1. **Plan Compliance Audit** — `oracle`
  Read the plan end-to-end. For each "Must Have": verify implementation exists (`cargo test -p jfc-graph`, grep for cycle detection, grep for depth parameter, etc.). For each "Must NOT Have": search codebase for forbidden patterns (ratatui imports in jfc-graph, more than 5 node kinds, DSL operators beyond the 7 allowed). Check evidence files exist in `.sisyphus/evidence/`. Compare deliverables against plan.
  Output: `Must Have [N/N] | Must NOT Have [N/N] | Tasks [N/N] | VERDICT: APPROVE/REJECT`

- [x] F2. **Code Quality Review** — `general`
  Run `cargo clippy -p jfc-graph -- -D warnings` + `cargo test -p jfc-graph` + `cargo build --workspace`. Review all jfc-graph source files for: `unwrap()` in non-test code, `todo!()` left behind, unused imports, dead code, missing pub API documentation, over-abstraction. Check: no ratatui dependencies in jfc-graph's Cargo.toml.
  Output: `Build [PASS/FAIL] | Clippy [PASS/FAIL] | Tests [N pass/N fail] | Issues [list] | VERDICT`

- [x] F3. **Real Manual QA** — `general`
  Build the full workspace. Run every QA scenario from every task — exact commands, capture output. Test cross-task integration: build graph of jfc-ui source, run DSL query, verify results match actual code structure (cross-reference with grep). Test edge cases: empty file, file with syntax errors, mutual recursion fixture. Save evidence.
  Output: `Scenarios [N/N pass] | Integration [N/N] | Edge Cases [N tested] | VERDICT`

- [x] F4. **Scope Fidelity Check** — `general`
  For each task: read "What to do", read actual implementation (git diff). Verify 1:1 — everything in spec was built (no missing), nothing beyond spec was built (no creep). Check "Must NOT do" compliance per task. Flag: any DSL operator beyond the 7 allowed? Any node kind beyond the 5? Any language adapter beyond Rust?
  Output: `Tasks [N/N compliant] | Scope Violations [CLEAN/N issues] | VERDICT`

---

## Commit Strategy

| After Tasks | Message | Files | Pre-commit |
|-------------|---------|-------|------------|
| 1 | `feat(graph): scaffold jfc-graph crate` | Cargo.toml, src/lib.rs | `cargo build -p jfc-graph` |
| 2-5 | `feat(graph): define node/edge types, traversal, adapter trait` | src/*.rs, tests/ | `cargo test -p jfc-graph` |
| 6-11 | `feat(graph): core engine — graph struct, tree-sitter adapter, DSL parser, symbols` | src/*.rs | `cargo test -p jfc-graph` |
| 12-18 | `feat(graph): features — DSL executor, partial struct, validation, persistence, capabilities` | src/*.rs | `cargo test -p jfc-graph` |
| 19-23 | `feat(graph): integration — tool dispatch, LSP enrichment, auto-context` | jfc-ui/src/tools.rs, jfc-graph/src/*.rs | `cargo test --workspace` |

---

## Success Criteria

### Verification Commands
```bash
cargo test -p jfc-graph              # All graph tests pass
cargo clippy -p jfc-graph -- -D warnings  # Zero warnings
cargo build --workspace              # Full workspace builds
cargo test --workspace               # All workspace tests pass
```

### Final Checklist
- [ ] All 7 GitHub issues (#1-#6, #18) have implementation + tests
- [ ] `fn("execute_tool") | callees` returns correct function list when run on jfc-ui source
- [ ] Cycle detection terminates on `mutual_recursion.rs` fixture
- [ ] Partial struct shows only accessed fields with metadata
- [ ] Symbol-based edit updates file correctly and re-parses
- [ ] Virtual validation catches signature mismatch
- [ ] Event log replays correctly
- [ ] Graph works WITHOUT LSP running (tree-sitter only mode)
- [ ] ≤8 DSL operators, ≤5 node kinds
- [ ] No ratatui dependency in jfc-graph
