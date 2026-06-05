//! Code graph intelligence: a tree-sitter-backed symbol/call/type graph over
//! the workspace, queryable through a pipe-based DSL with set algebra, path
//! patterns, taint tracing, and preconditions.
//!
//! The graph indexes 12 languages via per-language adapters, stores nodes and
//! edges in a CSR (compressed sparse row) representation for fast traversal,
//! and supports incremental re-indexing driven by a file watcher. Advanced
//! analysis modules include control-flow graphs, dataflow, interprocedural
//! points-to, complexity metrics, community detection, and coverage
//! annotation. A `GraphSession` memoizes query results and invalidates caches
//! after edits.

pub mod adapter;
pub mod analysis;
pub mod analysis_tools;
pub mod bfs_directed;
pub mod builder;
pub mod cache;
pub mod call_site;
pub mod capabilities;
pub mod cascade;
pub mod cfg;
pub mod cfg_rules;
pub mod closure;
pub mod co_change;
pub mod communities;
pub mod complexity;
pub mod complexity_rules;
pub mod content_index;
pub mod context;
pub mod coverage;
pub mod csr;
pub mod data_dir;
pub mod dataflow;
pub mod dataflow_rules;
pub mod dominators;
pub mod dsl;
pub mod edges;
pub mod enrichment;
pub mod fingerprint;
pub mod formatting;
pub mod framework_routes;
pub mod frontier;
pub mod graph;
pub mod history;
pub mod hll;
pub mod incremental;
pub(crate) mod index;
pub mod ir;
pub mod ir_map;
pub mod kind_specific;
pub mod label_reachability;
pub mod monomorphize;
pub mod nodes;
pub mod overlay;
pub mod partial;
pub mod pass;
pub mod persistence;
pub mod points_to;
pub mod polyglot;
pub mod possible_types;
pub mod predicates;
pub mod reactive;
pub mod resolver;
pub mod schema;
pub mod session;
pub mod slicing;
pub mod strata;
pub mod symbols;
pub mod taint_naming;
pub mod taint_v2;
pub mod traits_hierarchy;
pub mod traversal;
pub mod validation;
pub mod worktree;
