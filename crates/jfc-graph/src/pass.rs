//! Pass framework for jfc-graph analyses.
//!
//! Passes declare REQUIRED preconditions (flags that must be true before
//! they run) and POSTCONDITIONS (flags that change after they run).
//! The pass manager honors a partial order, not a total phase ordering.
//!
//! Idiom from t-compiler/mir-opts (Dylan MacKenzie): "Passes can declare
//! dependencies on sets of flags instead of specifying what phase they
//! run in, since that makes it hard to tell exactly what they depend on."
//!
//! # Migration plan
//!
//! This module is FORWARD INFRASTRUCTURE. Existing analyses
//! (`analysis::callers_of`, `enrichment::analyze_unresolved_calls`,
//! `symbols::ResolutionJob`, etc.) will gradually become [`Pass`]
//! implementors. The [`builder::GraphBuilder`](crate::builder::GraphBuilder)
//! will register them and call [`PassManager::run`] instead of invoking
//! each analysis in a fixed phase ordering. The intermediate state will
//! be a hybrid: some passes go through the manager while legacy call
//! sites remain in `enrichment.rs`/`analysis.rs`/`builder.rs`.
//!
//! # Example
//!
//! ```ignore
//! use jfc_graph::pass::{PassManager, StubResolveSymbolsPass, StubInferTypesPass, GraphFlag};
//!
//! let mut pm = PassManager::new();
//! pm.seed(GraphFlag::TreeParsed);
//! pm.register(Box::new(StubInferTypesPass));      // out of order on purpose —
//! pm.register(Box::new(StubResolveSymbolsPass));  // topo-sort fixes it
//! pm.run(&mut graph)?;
//! assert!(pm.flags().contains(&GraphFlag::TypesInferred));
//! ```

use std::collections::{HashMap, HashSet, VecDeque};

use crate::graph::CodeGraph;

/// Graph-state flags. Each represents an invariant that may or may not
/// hold for the graph at a given moment.
///
/// Marked `#[non_exhaustive]` so we can keep adding variants as new
/// passes land without breaking external consumers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GraphFlag {
    /// All source files have been parsed into the graph.
    TreeParsed,
    /// All in-file symbol references have been resolved.
    SymbolsResolved,
    /// All cross-file symbol references have been resolved.
    CrossFileResolved,
    /// All `UnresolvedCall` edges have been replaced with `Calls` or
    /// `ExternalCall` (or marked permanently unresolved).
    CallsResolved,
    /// Type information has been enriched into nodes (when LSP available).
    TypesInferred,
    /// CFG-style call graph is reachable from at least one entry point.
    CallGraphReachable,
}

/// Set of flags currently true on the graph.
pub type FlagSet = HashSet<GraphFlag>;

/// Trait every pass implements.
pub trait Pass {
    /// Stable, human-readable identifier — used in error messages and
    /// in cycle reports. Must be unique within a [`PassManager`].
    fn name(&self) -> &'static str;

    /// Flags that must be true before this pass runs. The manager
    /// returns `Err(PassError::MissingPrecondition)` if not all
    /// preconditions are met at execution time.
    fn requires(&self) -> &'static [GraphFlag];

    /// Flags newly set after this pass runs successfully.
    fn establishes(&self) -> &'static [GraphFlag];

    /// Flags that may be invalidated by this pass running. Other
    /// passes that required them must re-run.
    ///
    /// Default: nothing invalidated.
    fn invalidates(&self) -> &'static [GraphFlag] {
        &[]
    }

    /// Run the pass against the graph. Should return early on
    /// hard failures.
    fn run(&self, graph: &mut CodeGraph) -> Result<(), PassError>;
}

/// Errors raised by the pass manager.
#[derive(Debug, thiserror::Error)]
pub enum PassError {
    /// A pass's [`Pass::requires`] was not satisfied at execution time.
    #[error("pass '{name}' missing precondition: {flag:?}")]
    MissingPrecondition {
        name: &'static str,
        flag: GraphFlag,
    },
    /// A pass returned a hard failure from its `run`.
    #[error("pass '{name}' failed: {reason}")]
    Failed {
        name: &'static str,
        reason: String,
    },
    /// The dependency DAG contains a cycle. The vector lists the
    /// pass names involved (in any order).
    #[error("pass dependency cycle detected: {0:?}")]
    Cycle(Vec<&'static str>),
}

/// Drives a set of registered [`Pass`]es in dependency order.
pub struct PassManager {
    flags: FlagSet,
    passes: Vec<Box<dyn Pass>>,
}

impl PassManager {
    pub fn new() -> Self {
        Self {
            flags: FlagSet::new(),
            passes: Vec::new(),
        }
    }

    /// Register a pass with the manager. Order of registration does
    /// not matter — the manager will topo-sort them at [`Self::run`].
    pub fn register(&mut self, p: Box<dyn Pass>) {
        self.passes.push(p);
    }

    /// Mark a flag as already true on the graph. Useful for the
    /// initial state (e.g. the [`builder::GraphBuilder`](crate::builder)
    /// has already parsed all trees, so `TreeParsed` is seeded before
    /// any analysis pass runs).
    pub fn seed(&mut self, flag: GraphFlag) {
        self.flags.insert(flag);
    }

    /// Inspect the currently-true flag set.
    pub fn flags(&self) -> &FlagSet {
        &self.flags
    }

    /// Run all registered passes in dependency order.
    ///
    /// Algorithm: Kahn's topo-sort over the requires/establishes DAG.
    /// Edges go from a pass that establishes flag `F` to every pass
    /// that requires `F`. Cycles are reported as
    /// [`PassError::Cycle`].
    ///
    /// Cycle detection: any pass whose in-degree never reaches zero
    /// after the queue drains is part of a cycle (Kahn's classical
    /// post-condition).
    pub fn run(&mut self, graph: &mut CodeGraph) -> Result<(), PassError> {
        let order = self.topo_sort()?;

        for idx in order {
            let pass = &self.passes[idx];

            // Verify preconditions still hold (a previous pass may
            // have invalidated something we rely on; the topo-sort
            // doesn't itself guarantee this).
            for flag in pass.requires() {
                if !self.flags.contains(flag) {
                    return Err(PassError::MissingPrecondition {
                        name: pass.name(),
                        flag: *flag,
                    });
                }
            }

            pass.run(graph)?;

            // Apply postconditions. Invalidations first, in case a
            // pass both invalidates and re-establishes the same flag.
            for flag in pass.invalidates() {
                self.flags.remove(flag);
            }
            for flag in pass.establishes() {
                self.flags.insert(*flag);
            }
        }
        Ok(())
    }

    /// Kahn's algorithm: returns indices into `self.passes` in
    /// dependency order, or `Err(PassError::Cycle)`.
    fn topo_sort(&self) -> Result<Vec<usize>, PassError> {
        // Producer map: flag -> index of the pass that establishes it.
        // We assume each flag is established by at most one pass for
        // dependency-graph purposes; if more than one establishes the
        // same flag, the last registration wins as the "producer" for
        // dependency edges (the others still execute, they just don't
        // create edges).
        let mut producer: HashMap<GraphFlag, usize> = HashMap::new();
        for (i, p) in self.passes.iter().enumerate() {
            for f in p.establishes() {
                producer.insert(*f, i);
            }
        }

        // Build adjacency + in-degree. Edge u -> v means "u must run
        // before v". For each pass v, for each flag v requires, if
        // some pass u establishes it, add edge u -> v. Flags already
        // seeded into `self.flags` create no edge — they're already
        // satisfied by the initial state.
        let n = self.passes.len();
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut in_degree: Vec<usize> = vec![0; n];

        for (v, p) in self.passes.iter().enumerate() {
            for flag in p.requires() {
                if self.flags.contains(flag) {
                    continue; // Pre-satisfied by seed.
                }
                if let Some(&u) = producer.get(flag) {
                    if u == v {
                        // Self-edge: a pass requiring something it
                        // establishes is itself a cycle of length 1.
                        adj[u].push(v);
                        in_degree[v] += 1;
                    } else {
                        adj[u].push(v);
                        in_degree[v] += 1;
                    }
                }
                // If no producer and not seeded, the precondition is
                // unsatisfiable. We let `run` catch it as
                // MissingPrecondition rather than failing here, so
                // callers can introspect which pass tripped it.
            }
        }

        // Invalidation creates a back-edge: if pass v invalidates
        // flag F and pass u requires F, then u must run before v.
        // (Otherwise v would invalidate F and u would then fail its
        // precondition.) This is where real cycles emerge:
        //   A requires X, invalidates Y
        //   B requires Y, invalidates X
        // produces edges A->B (B requires Y, A establishes Y... wait,
        // A merely invalidates Y, doesn't establish it). The cycle
        // actually comes from the requires-vs-invalidates pairing:
        //   B requires Y, A invalidates Y => B must run before A.
        //   A requires X, B invalidates X => A must run before B.
        for (v, p) in self.passes.iter().enumerate() {
            for inv in p.invalidates() {
                for (u, q) in self.passes.iter().enumerate() {
                    if u == v {
                        continue;
                    }
                    if q.requires().contains(inv) {
                        // u requires what v invalidates — u must run
                        // before v.
                        adj[u].push(v);
                        in_degree[v] += 1;
                    }
                }
            }
        }

        // Kahn: seed queue with zero in-degree nodes.
        let mut queue: VecDeque<usize> =
            (0..n).filter(|&i| in_degree[i] == 0).collect();
        let mut order = Vec::with_capacity(n);

        while let Some(u) = queue.pop_front() {
            order.push(u);
            for &v in &adj[u] {
                in_degree[v] -= 1;
                if in_degree[v] == 0 {
                    queue.push_back(v);
                }
            }
        }

        if order.len() != n {
            // Anything still with in_degree > 0 is in (or downstream
            // of) a cycle. Report all of them.
            let stuck: Vec<&'static str> = (0..n)
                .filter(|&i| !order.contains(&i))
                .map(|i| self.passes[i].name())
                .collect();
            return Err(PassError::Cycle(stuck));
        }

        Ok(order)
    }
}

impl Default for PassManager {
    fn default() -> Self {
        Self::new()
    }
}

// -- Stub passes -------------------------------------------------------------
//
// These exist to (a) demonstrate the API on real flags and (b) drive
// the unit tests. They do nothing to the graph; the real passes will
// land as the migration progresses.

/// Stub: pretends to resolve in-file symbol references.
pub struct StubResolveSymbolsPass;

impl Pass for StubResolveSymbolsPass {
    fn name(&self) -> &'static str {
        "stub-resolve-symbols"
    }
    fn requires(&self) -> &'static [GraphFlag] {
        &[GraphFlag::TreeParsed]
    }
    fn establishes(&self) -> &'static [GraphFlag] {
        &[GraphFlag::SymbolsResolved]
    }
    fn run(&self, _graph: &mut CodeGraph) -> Result<(), PassError> {
        Ok(())
    }
}

/// Stub: pretends to enrich nodes with type info from an LSP.
pub struct StubInferTypesPass;

impl Pass for StubInferTypesPass {
    fn name(&self) -> &'static str {
        "stub-infer-types"
    }
    fn requires(&self) -> &'static [GraphFlag] {
        &[GraphFlag::SymbolsResolved]
    }
    fn establishes(&self) -> &'static [GraphFlag] {
        &[GraphFlag::TypesInferred]
    }
    fn run(&self, _graph: &mut CodeGraph) -> Result<(), PassError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A pass that records its execution into a shared trace.
    struct TracePass {
        name: &'static str,
        requires: &'static [GraphFlag],
        establishes: &'static [GraphFlag],
        invalidates: &'static [GraphFlag],
        trace: Rc<RefCell<Vec<&'static str>>>,
    }

    impl Pass for TracePass {
        fn name(&self) -> &'static str {
            self.name
        }
        fn requires(&self) -> &'static [GraphFlag] {
            self.requires
        }
        fn establishes(&self) -> &'static [GraphFlag] {
            self.establishes
        }
        fn invalidates(&self) -> &'static [GraphFlag] {
            self.invalidates
        }
        fn run(&self, _graph: &mut CodeGraph) -> Result<(), PassError> {
            self.trace.borrow_mut().push(self.name);
            Ok(())
        }
    }

    fn trace() -> Rc<RefCell<Vec<&'static str>>> {
        Rc::new(RefCell::new(Vec::new()))
    }

    #[test]
    fn pass_manager_runs_in_dependency_order_normal() {
        let mut pm = PassManager::new();
        pm.seed(GraphFlag::TreeParsed);
        // Register out of order — topo-sort should still run resolve
        // before infer.
        pm.register(Box::new(StubInferTypesPass));
        pm.register(Box::new(StubResolveSymbolsPass));
        let mut g = CodeGraph::new();
        pm.run(&mut g).expect("clean run");
        assert!(pm.flags().contains(&GraphFlag::SymbolsResolved));
        assert!(pm.flags().contains(&GraphFlag::TypesInferred));
    }

    #[test]
    fn pass_manager_runs_passes_in_observed_order_normal() {
        let t = trace();
        let mut pm = PassManager::new();
        pm.seed(GraphFlag::TreeParsed);
        pm.register(Box::new(TracePass {
            name: "infer",
            requires: &[GraphFlag::SymbolsResolved],
            establishes: &[GraphFlag::TypesInferred],
            invalidates: &[],
            trace: t.clone(),
        }));
        pm.register(Box::new(TracePass {
            name: "resolve",
            requires: &[GraphFlag::TreeParsed],
            establishes: &[GraphFlag::SymbolsResolved],
            invalidates: &[],
            trace: t.clone(),
        }));
        let mut g = CodeGraph::new();
        pm.run(&mut g).unwrap();
        assert_eq!(*t.borrow(), vec!["resolve", "infer"]);
    }

    #[test]
    fn pass_manager_detects_missing_precondition_robust() {
        // No producer for SymbolsResolved AND nothing seeded → missing
        // precondition surfaces at execution time.
        let mut pm = PassManager::new();
        pm.register(Box::new(StubInferTypesPass));
        let mut g = CodeGraph::new();
        let err = pm.run(&mut g).expect_err("should fail");
        match err {
            PassError::MissingPrecondition { name, flag } => {
                assert_eq!(name, "stub-infer-types");
                assert_eq!(flag, GraphFlag::SymbolsResolved);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn pass_manager_detects_cycle_robust() {
        // A requires X, invalidates Y; B requires Y, invalidates X.
        // requires-vs-invalidates: A requires X, B invalidates X => A
        // before B. B requires Y, A invalidates Y => B before A. Cycle.
        struct A;
        impl Pass for A {
            fn name(&self) -> &'static str {
                "A"
            }
            fn requires(&self) -> &'static [GraphFlag] {
                &[GraphFlag::CrossFileResolved]
            }
            fn establishes(&self) -> &'static [GraphFlag] {
                &[]
            }
            fn invalidates(&self) -> &'static [GraphFlag] {
                &[GraphFlag::CallsResolved]
            }
            fn run(&self, _g: &mut CodeGraph) -> Result<(), PassError> {
                Ok(())
            }
        }
        struct B;
        impl Pass for B {
            fn name(&self) -> &'static str {
                "B"
            }
            fn requires(&self) -> &'static [GraphFlag] {
                &[GraphFlag::CallsResolved]
            }
            fn establishes(&self) -> &'static [GraphFlag] {
                &[]
            }
            fn invalidates(&self) -> &'static [GraphFlag] {
                &[GraphFlag::CrossFileResolved]
            }
            fn run(&self, _g: &mut CodeGraph) -> Result<(), PassError> {
                Ok(())
            }
        }

        let mut pm = PassManager::new();
        pm.seed(GraphFlag::CrossFileResolved);
        pm.seed(GraphFlag::CallsResolved);
        pm.register(Box::new(A));
        pm.register(Box::new(B));
        let mut g = CodeGraph::new();
        let err = pm.run(&mut g).expect_err("should detect cycle");
        match err {
            PassError::Cycle(names) => {
                assert!(names.contains(&"A"));
                assert!(names.contains(&"B"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn pass_manager_topo_sort_handles_diamond_normal() {
        // A establishes X. B requires X, establishes Y1. C requires X,
        // establishes Y2. D requires Y1 and Y2.
        let t = trace();
        let mut pm = PassManager::new();

        pm.register(Box::new(TracePass {
            name: "D",
            requires: &[GraphFlag::SymbolsResolved, GraphFlag::CrossFileResolved],
            establishes: &[GraphFlag::CallsResolved],
            invalidates: &[],
            trace: t.clone(),
        }));
        pm.register(Box::new(TracePass {
            name: "C",
            requires: &[GraphFlag::TreeParsed],
            establishes: &[GraphFlag::CrossFileResolved],
            invalidates: &[],
            trace: t.clone(),
        }));
        pm.register(Box::new(TracePass {
            name: "B",
            requires: &[GraphFlag::TreeParsed],
            establishes: &[GraphFlag::SymbolsResolved],
            invalidates: &[],
            trace: t.clone(),
        }));
        pm.register(Box::new(TracePass {
            name: "A",
            requires: &[],
            establishes: &[GraphFlag::TreeParsed],
            invalidates: &[],
            trace: t.clone(),
        }));

        let mut g = CodeGraph::new();
        pm.run(&mut g).unwrap();

        // Verify diamond ordering: A first, D last; B and C between
        // (in either order).
        let trace = t.borrow();
        let pos = |n: &str| trace.iter().position(|x| *x == n).unwrap();
        assert!(pos("A") < pos("B"));
        assert!(pos("A") < pos("C"));
        assert!(pos("B") < pos("D"));
        assert!(pos("C") < pos("D"));

        // All four flags should now be set.
        assert!(pm.flags().contains(&GraphFlag::TreeParsed));
        assert!(pm.flags().contains(&GraphFlag::SymbolsResolved));
        assert!(pm.flags().contains(&GraphFlag::CrossFileResolved));
        assert!(pm.flags().contains(&GraphFlag::CallsResolved));
    }
}
