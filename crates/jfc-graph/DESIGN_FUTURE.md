//! Design stubs for future large features (P11-1, P11-3, P13, P14, P15).
//!
//! These are documented designs for features that exceed the scope of
//! a single session. Each section outlines the architecture, crate
//! changes needed, and estimated effort.
//!
//! ## P11-1: GraphBLAS SpMV-formulation BFS
//!
//! ### Design
//! Model BFS as sparse matrix–vector multiplication (SpMV):
//! - Adjacency matrix `A` as `sprs::CsMat<bool>` (or `u8` for
//!   edge-kind discrimination).
//! - Frontier vector `v` as `sprs::CsVec<bool>`.
//! - One BFS step = `v' = A^T * v` (transpose for outgoing).
//! - Direction-optimised: switch between `A^T * v` (push) and
//!   `A * v` (pull) based on `nnz(v) / nnz(A)`.
//!
//! ### Dependencies
//! - `sprs = "0.11"` (sparse linear algebra, pure Rust).
//!
//! ### Estimated LoC: ~400
//!
//! ---
//!
//! ## P11-3: METIS-style Graph Partitioning
//!
//! ### Design
//! Multilevel k-way partitioning:
//! 1. **Coarsen**: heavy-edge matching → contracted graph (halve V
//!    each level until V < 1000).
//! 2. **Initial partition**: greedy bisection on the coarsest graph.
//! 3. **Refine**: Kernighan-Lin / Fiduccia-Mattheyses refinement at
//!    each uncoarsen level.
//!
//! Output: `Vec<usize>` of length V mapping each node to its
//! partition.
//!
//! ### Use case
//! Sharding a 100k-node monorepo graph so that independent analysis
//! passes can run on partitions in parallel without cross-boundary
//! edge traffic.
//!
//! ### Estimated LoC: ~800
//!
//! ---
//!
//! ## P13: Datalog Rule Engine (Soufflé-style)
//!
//! ### Design
//! Semi-naive bottom-up evaluator:
//! - **Relations**: `BTreeSet<Vec<Value>>` where `Value` is
//!   `NodeId | String | u64`.
//! - **Rules**: `Rule { head: Atom, body: Vec<Atom>, neg: Vec<Atom> }`
//! - **Evaluation**: stratified (using [`crate::strata::stratify`]),
//!   then per-stratum iterate rules until fixpoint using delta
//!   relations.
//! - **Built-in relations**: `calls(A, B)`, `uses_type(A, B)`,
//!   `implements(A, B)`, `node(Id, Kind, Name)`.
//! - **User-defined rules**: e.g.
//!   ```text
//!   reachable(X, Y) :- calls(X, Y).
//!   reachable(X, Y) :- calls(X, Z), reachable(Z, Y).
//!   unsafe_path(X) :- reachable(X, Y), sink(Y), !sanitized(X).
//!   ```
//!
//! ### Dependencies
//! - None beyond what's in the workspace.
//!
//! ### Estimated LoC: ~1500-2000
//!
//! ---
//!
//! ## P14-1: GPU Backend
//!
//! ### Design
//! `wgpu` compute shader backend for BFS at scale (>1M edges):
//! - CSR arrays uploaded as storage buffers.
//! - Frontier as a GPU-side bitset buffer.
//! - One dispatch = one BFS level (push-only; pull on GPU is
//!   memory-bandwidth-bound and rarely wins).
//! - Results downloaded as a `Vec<u32>` of visited vertex indices.
//!
//! ### When to use
//! Only justified when V > 100k AND the user has a discrete GPU.
//! Feature-gated behind `features = ["gpu"]`.
//!
//! ### Estimated LoC: ~600 (shader + host code)
//!
//! ---
//!
//! ## P14-2: Distributed Shards
//!
//! ### Design
//! Multi-process architecture:
//! - Each process owns one graph partition (from P11-3).
//! - Cross-shard edges stored as ghost vertices.
//! - Query coordinator broadcasts query to all shards; each shard
//!   executes locally and returns partial results; coordinator merges.
//! - Communication: Unix domain sockets + bincode serialisation.
//!
//! ### When to use
//! Monorepos > 500k nodes where a single process can't hold the
//! graph in L3 cache.
//!
//! ### Estimated LoC: ~1200 (coordinator + shard worker)
//!
//! ---
//!
//! ## P15: LSP Server Backed by Graph
//!
//! ### Design
//! Standalone binary `jfc-lsp` in the workspace:
//! - Implements LSP `textDocument/definition`, `references`, `hover`.
//! - On init: builds `GraphSession` from workspace root.
//! - On `didChange`: calls `session.file_changed(...)`.
//! - `definition`: resolve symbol handle → `NodeData.span`.
//! - `references`: `graph_query` `fn("X") | callers` → locations.
//! - `hover`: `KindData` projection → markdown summary.
//!
//! ### Dependencies
//! - `lsp-server`, `lsp-types` crates.
//!
//! ### Estimated LoC: ~800
