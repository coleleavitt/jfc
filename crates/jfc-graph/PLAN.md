# jfc-graph Enhancement Plan

## Goal

Implement the features present in `codegraph` (optave/ops-codegraph-tool) that
are missing from `jfc-graph`. This bridges the gap while preserving jfc-graph's
architectural strengths (in-process, content-addressed NodeIds, pipe DSL, zero
IPC).

---

## Phase 1: Complexity Metrics (Priority: HIGH)

**What codegraph has:** Per-function cognitive complexity, cyclomatic complexity,
max nesting depth, Halstead metrics (vocabulary, length, volume, difficulty,
effort, bugs), LOC metrics (loc, sloc, comment lines), maintainability index.

**What jfc-graph has:** Nothing — no per-node metrics.

### Implementation

Create `crates/jfc-graph/src/complexity.rs`:

```rust
pub struct ComplexityMetrics {
    pub cognitive: u32,
    pub cyclomatic: u32,
    pub max_nesting: u32,
    pub halstead: Option<HalsteadMetrics>,
    pub loc: Option<LocMetrics>,
    pub maintainability_index: Option<f64>,
}

pub struct HalsteadMetrics {
    pub n1: u32,       // distinct operators
    pub n2: u32,       // distinct operands
    pub big_n1: u32,   // total operators
    pub big_n2: u32,   // total operands
    pub vocabulary: u32,
    pub length: u32,
    pub volume: f64,
    pub difficulty: f64,
    pub effort: f64,
    pub bugs: f64,
}

pub struct LocMetrics {
    pub loc: u32,
    pub sloc: u32,
    pub comment_lines: u32,
}
```

**Approach:** Use tree-sitter AST walking with per-language rules (mirroring
codegraph's `LangRules` pattern). Store metrics on `NodeData` as
`Option<ComplexityMetrics>`.

**Per-language rules needed:**
- Rust: `if_expression`, `match_expression`, `for_expression`, `while_expression`,
  `loop_expression`, `&&`/`||` in `binary_expression`
- TypeScript/JS: `if_statement`, `switch_statement`, `for_statement`,
  `while_statement`, `ternary_expression`, `&&`/`||`/`??`
- Python: `if_statement`, `for_statement`, `while_statement`, `match_statement`,
  `and`/`or` in `boolean_operator`
- Go: `if_statement`, `for_statement`, `expression_switch_statement`,
  `select_statement`, `&&`/`||`

**Files to create/modify:**
- NEW: `src/complexity.rs` (~600 LOC)
- NEW: `src/complexity_rules.rs` (~200 LOC, per-language rule tables)
- MODIFY: `src/nodes.rs` — add `complexity: Option<ComplexityMetrics>` to `NodeData`
- MODIFY: `src/adapter/rust.rs` — compute metrics during extraction
- MODIFY: `src/adapter/typescript.rs` — same
- MODIFY: `src/adapter/python.rs` — same
- MODIFY: `src/adapter/go.rs` — same

**DSL integration:** Add `complexity` operator to surface metrics in query results.
`fn("execute_tool") | complexity` → shows cognitive/cyclomatic for matched fns.

**Estimated LOC:** ~800

---

## Phase 2: Control Flow Graph (Priority: HIGH)

**What codegraph has:** Per-function CFG with labeled blocks and typed edges
(normal, branch-true, branch-false, loop-back, exception, break, continue, return).

**What jfc-graph has:** Nothing — traversal is at the call-graph level only.

### Implementation

Create `crates/jfc-graph/src/cfg.rs`:

```rust
pub struct CfgBlock {
    pub id: u32,
    pub label: String,
    pub start_line: u32,
    pub end_line: u32,
    pub kind: CfgBlockKind,
}

pub enum CfgBlockKind {
    Entry,
    Exit,
    Normal,
    Branch,
    Loop,
    Exception,
}

pub struct CfgEdge {
    pub from: u32,
    pub to: u32,
    pub kind: CfgEdgeKind,
}

pub enum CfgEdgeKind {
    Normal,
    BranchTrue,
    BranchFalse,
    LoopBack,
    Exception,
    Break,
    Continue,
    Return,
}

pub struct FunctionCfg {
    pub blocks: Vec<CfgBlock>,
    pub edges: Vec<CfgEdge>,
}
```

**Approach:** Walk each function's body building a basic-block graph.
Per-language rules for if/else/match/for/while/loop/try/catch nodes
(same `CfgRules` pattern as codegraph).

**Files to create/modify:**
- NEW: `src/cfg.rs` (~800 LOC)
- NEW: `src/cfg_rules.rs` (~300 LOC, per-language tables)
- MODIFY: `src/nodes.rs` — add `cfg: Option<FunctionCfg>` to `NodeData`
- MODIFY: each adapter

**DSL integration:** `fn("handler") | cfg` → dump the CFG for matched functions.

**Estimated LOC:** ~1100

---

## Phase 3: Dataflow Analysis (Priority: MEDIUM)

**What codegraph has:** Per-function parameter tracking, return value tracking,
assignment tracking, argument flow (which param flows to which call arg),
mutation detection.

**What jfc-graph has:** `taint` operator traces a named parameter through call
chains, but doesn't extract structured dataflow per-function.

### Implementation

Create `crates/jfc-graph/src/dataflow.rs`:

```rust
pub struct FunctionDataflow {
    pub params: Vec<DataflowParam>,
    pub returns: Vec<DataflowReturn>,
    pub assignments: Vec<DataflowAssignment>,
    pub arg_flows: Vec<DataflowArgFlow>,
    pub mutations: Vec<DataflowMutation>,
}

pub struct DataflowParam {
    pub name: String,
    pub position: u32,
    pub type_annotation: Option<String>,
    pub has_default: bool,
}

pub struct DataflowReturn {
    pub line: u32,
    pub expression: String, // simplified
}

pub struct DataflowAssignment {
    pub target: String,
    pub source_kind: AssignSourceKind, // literal, param, call_result, field_access
    pub line: u32,
}

pub struct DataflowArgFlow {
    pub callee: String,
    pub arg_position: u32,
    pub source_param: Option<String>, // which param flows here
    pub line: u32,
}

pub struct DataflowMutation {
    pub target: String,
    pub method: String, // e.g. "push", "sort", "clear"
    pub line: u32,
}
```

**Approach:** Walk function bodies tracking identifier flow. Per-language rules
for call expressions, assignments, member access, mutations.

**Files to create/modify:**
- NEW: `src/dataflow.rs` (~1000 LOC)
- NEW: `src/dataflow_rules.rs` (~400 LOC)
- MODIFY: `src/nodes.rs` — add `dataflow: Option<FunctionDataflow>` to `NodeData`

**DSL integration:** `fn("parse_request") | dataflow` → show param/return/mutation info.

**Estimated LOC:** ~1400

---

## Phase 4: Community Detection (Priority: MEDIUM)

**What codegraph has:** Louvain algorithm for modularity-based community
detection on the call graph. Returns node→community assignments + modularity
score.

**What jfc-graph has:** Nothing (strata gives call-depth layers but not clusters).

### Implementation

Create `crates/jfc-graph/src/communities.rs`:

```rust
pub struct CommunityResult {
    pub assignments: Vec<(NodeId, u32)>, // node → community_id
    pub modularity: f64,
    pub community_count: u32,
}

pub fn louvain(graph: &CodeGraph, resolution: f64, seed: u64) -> CommunityResult;
```

**Approach:** Classic Louvain on the undirected projection of the call graph.
Iterative modularity optimization with coarsening. Same algorithm as
codegraph's `louvain_communities`.

**Files to create/modify:**
- NEW: `src/communities.rs` (~400 LOC)
- MODIFY: `src/dsl/mod.rs` — add `communities` operator

**DSL integration:** `fn("*") | communities` → show which functions cluster together.

**Estimated LOC:** ~400

---

## Phase 5: Additional Language Adapters (Priority: MEDIUM)

**What codegraph has:** 34 languages.
**What jfc-graph has:** 4 (Rust, TypeScript, Python, Go).

### Priority order for new adapters:

1. **Java** — widely used, tree-sitter-java mature
2. **C/C++** — systems code, tree-sitter-c/cpp mature
3. **C#** — enterprise, tree-sitter-c-sharp available
4. **Ruby** — scripting, tree-sitter-ruby mature
5. **PHP** — web backends, tree-sitter-php available
6. **Kotlin** — Android/JVM, tree-sitter-kotlin-sg available
7. **Swift** — iOS, tree-sitter-swift available
8. **Scala** — JVM functional, tree-sitter-scala available

### Implementation per adapter (~200-400 LOC each):

```rust
// src/adapter/java.rs
pub struct JavaAdapter;

impl LanguageAdapter for JavaAdapter {
    fn language_id(&self) -> &str { "java" }
    fn file_extensions(&self) -> &[&str] { &["java"] }
    fn parse_file(&self, path: &Path, content: &str) -> Result<ParsedFile, AdapterError> { ... }
    fn extract_nodes(&self, parsed: &ParsedFile) -> Vec<NodeData> { ... }
    fn extract_edges(&self, parsed: &ParsedFile, nodes: &[NodeData]) -> Vec<(NodeId, NodeId, EdgeData)> { ... }
}
```

**Node mapping per language:**
- `class_declaration` → Struct
- `interface_declaration` → Trait
- `method_declaration` → Function
- `enum_declaration` → Enum
- Package/namespace → Module

**Dependencies to add (Cargo.toml):**
```toml
tree-sitter-java = "0.23"
tree-sitter-c = "0.23"
tree-sitter-cpp = "0.23"
tree-sitter-c-sharp = "0.23"
tree-sitter-ruby = "0.23"
tree-sitter-php = "0.23"
tree-sitter-kotlin-sg = "0.4"
tree-sitter-swift = "0.6"
```

**Estimated LOC:** ~2400 (8 adapters × ~300 LOC avg)

---

## Phase 6: Co-Change Analysis (Priority: LOW)

**What codegraph has:** Git history correlation — finds files/functions that
change together across commits ("temporal coupling").

**What jfc-graph has:** `history.rs` tracks `last_modified_revision` per node
and the `since N` DSL filter, but no co-change correlation.

### Implementation

Create `crates/jfc-graph/src/co_change.rs`:

```rust
pub struct CoChangeResult {
    pub pairs: Vec<CoChangePair>,
}

pub struct CoChangePair {
    pub node_a: NodeId,
    pub node_b: NodeId,
    pub times_changed_together: u32,
    pub total_changes_a: u32,
    pub total_changes_b: u32,
    pub confidence: f64, // times_together / max(total_a, total_b)
}

pub fn compute_co_changes(
    graph: &CodeGraph,
    git_log: &[CommitInfo],
    min_support: u32,
) -> CoChangeResult;
```

**Approach:** Parse `git log --name-only` output, map file paths to graph nodes,
build co-occurrence matrix, filter by minimum support threshold.

**Files to create/modify:**
- NEW: `src/co_change.rs` (~300 LOC)
- MODIFY: `src/session.rs` — add `co_changes()` method
- MODIFY: `src/dsl/mod.rs` — add `co_changes` operator

**DSL integration:** `fn("parse_request") | co_changes` → show which functions
change together with `parse_request`.

**Estimated LOC:** ~300

---

## Phase 7: Semantic Search (Priority: LOW)

**What codegraph has:** Embedding-based semantic search over symbol names and
signatures (requires external embedding model).

**What jfc-graph has:** Substring matching only (`codegraph_search` does
case-insensitive name matching).

### Implementation

Skip for now — requires an embedding model dependency (adds significant binary
size and complexity). The existing `SymbolTable` prefix/fuzzy search + the DSL's
`fn("pattern")` already covers most use cases. Revisit when a lightweight
local embedding solution (e.g. `fastembed-rs`) stabilizes.

---

## Summary

| Phase | Feature | Est. LOC | Priority | Dependencies |
|-------|---------|----------|----------|--------------|
| 1 | Complexity Metrics | ~800 | HIGH | None |
| 2 | Control Flow Graph | ~1100 | HIGH | Phase 1 (shares rules) |
| 3 | Dataflow Analysis | ~1400 | MEDIUM | Phase 2 (uses CFG) |
| 4 | Community Detection | ~400 | MEDIUM | None |
| 5 | Language Adapters (8) | ~2400 | MEDIUM | tree-sitter crates |
| 6 | Co-Change Analysis | ~300 | LOW | git2 or shell |
| 7 | Semantic Search | skip | LOW | embedding model |

**Total estimated new code:** ~6,400 LOC (excluding tests)
**Total with tests (2:1 ratio):** ~19,000 LOC

---

## Implementation Order

1. **Phase 1 (Complexity)** — foundational, no deps, immediate value for the
   `codegraph_context` MCP tool (can show hotspots).
2. **Phase 4 (Communities)** — pure graph algorithm, small, independent.
3. **Phase 2 (CFG)** — shares rule tables with Phase 1, builds on same AST walk.
4. **Phase 5 (Language Adapters)** — can be done incrementally, one at a time.
5. **Phase 3 (Dataflow)** — most complex, benefits from CFG being done first.
6. **Phase 6 (Co-Change)** — nice-to-have, low effort.

---

## Architecture Notes

### Where metrics live

Metrics attach to `NodeData` as optional fields. The graph is still in-memory
(`petgraph::StableDiGraph`), so there's no SQLite overhead. Metrics are computed
lazily on first access or eagerly during indexing (configurable).

### DSL integration pattern

Each new analysis gets a DSL operator name. The operator:
1. Filters the working set (if piped after a selector)
2. Computes/retrieves the analysis for matching nodes
3. Adds the result to `QueryResult.annotations`

The formatter then renders annotations alongside node info in the
token-budgeted output.

### Adapter trait extension

New adapters implement the existing `LanguageAdapter` trait unchanged. The
`AdapterRegistry` dispatches by file extension. No breaking changes to
existing code.

### Feature flags

Heavy analysis (CFG, dataflow) can be gated behind cargo features:
```toml
[features]
default = ["complexity", "cfg"]
complexity = []
cfg = []
dataflow = ["cfg"]
communities = []
```

This keeps the base binary lean for users who only need call-graph traversal.
