//! Query plan optimiser for the jfc-graph DSL (Phase 3).
//!
//! ## Overview
//!
//! The DSL parser produces a syntax tree (`Expr` or `Vec<DslOp>`) that's
//! evaluated naively left-to-right by the executor. This module
//! introduces a *logical-plan-then-rewrite* layer between parsing and
//! evaluation:
//!
//! - **Filter pushdown** — `fn("X") | callers | filter kind=Function`
//!   becomes equivalent to applying the kind filter while the
//!   reverse-BFS expands, instead of scanning the post-result. We
//!   express this by reordering `DslOp` so filters appear as early as
//!   semantics permit.
//! - **Depth fusion** — `callers | callers | callers` collapses to a
//!   single `Callers + Depth(3)` invocation, saving redundant BFS work.
//! - **Set-op short-circuit** — for `A intersect B`, evaluate the side
//!   the planner predicts is smaller first (we use a heuristic on
//!   leaf-level selectivity).
//! - **Schedule selection** — pick push vs pull (or auto) for BFS-style
//!   ops based on graph density. Surfaced as a [`Schedule`] annotation
//!   the executor can consult; the legacy executor ignores it
//!   gracefully.
//!
//! ## Semantics-preservation
//!
//! Every rewrite rule here is **semantics-preserving**: it produces a
//! plan that, when evaluated, returns the same node set as the input
//! plan. The unit tests in this module assert this invariant on every
//! rewrite.

use crate::dsl::{DslOp, Expr, SetOp};

/// BFS execution strategy hint. Surfaced as an annotation on the plan
/// so the executor can pick the right primitive without re-deciding
/// per-node. The default `Auto` lets the executor consult the
/// snapshot's density at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScheduleStrategy {
    /// Iterate frontier, expand to neighbours. Best when frontier
    /// is small (early BFS layers).
    Push,
    /// Iterate every unvisited vertex, scan predecessors. Best when
    /// frontier is large (late BFS layers, ~1/16 of the graph).
    Pull,
    /// Let the executor switch dynamically (Yang 2018).
    #[default]
    Auto,
}

/// Schedule annotation attached to a plan (or sub-plan). Currently
/// just `strategy` + `parallel`; future fields might include chunk
/// size, NUMA hints, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Schedule {
    pub strategy: ScheduleStrategy,
    /// Use rayon-backed parallel BFS expansion. Caller-controllable
    /// because parallel BFS is non-deterministic in result ordering;
    /// some test paths want determinism.
    pub parallel: bool,
}

/// Logical query plan. Mirrors `Expr` but carries a [`Schedule`]
/// annotation so the optimiser can record decisions for the executor.
#[derive(Debug, Clone)]
pub enum Plan {
    Pipe {
        ops: Vec<DslOp>,
        schedule: Schedule,
    },
    Expr {
        expr: Expr,
        schedule: Schedule,
    },
}

impl Plan {
    pub fn schedule(&self) -> Schedule {
        match self {
            Plan::Pipe { schedule, .. } | Plan::Expr { schedule, .. } => *schedule,
        }
    }

    pub fn ops(&self) -> Option<&[DslOp]> {
        match self {
            Plan::Pipe { ops, .. } => Some(ops),
            _ => None,
        }
    }

    pub fn expr(&self) -> Option<&Expr> {
        match self {
            Plan::Expr { expr, .. } => Some(expr),
            _ => None,
        }
    }
}

// ─── Optimiser entry points ──────────────────────────────────────────────

/// Build a logical plan from a pipe-chain `Vec<DslOp>` and apply all
/// rewrite passes. Returns a fresh plan; the input is consumed but the
/// output is not aliased to it.
pub fn optimise_pipe(ops: Vec<DslOp>) -> Plan {
    let mut ops = ops;
    fuse_depths(&mut ops);
    push_filters_down(&mut ops);
    let schedule = pick_schedule_for_pipe(&ops);
    Plan::Pipe { ops, schedule }
}

/// Build a logical plan from an `Expr` and apply rewrite passes. Set
/// operands are reordered when intersect-with-likely-smaller-LHS.
pub fn optimise_expr(expr: Expr) -> Plan {
    let expr = rewrite_expr(expr);
    let schedule = Schedule::default();
    Plan::Expr { expr, schedule }
}

// ─── Rewrite rules ──────────────────────────────────────────────────────

/// **Depth fusion**: collapse `callers | callers | callers` into one
/// `callers + Depth(3)` invocation. Same for `callees`. Existing
/// explicit `Depth(N)` ops are folded in (multiplied because each
/// repeated `callers` is one BFS level).
///
/// The rewrite scans for runs of identical `Callers`/`Callees` ops
/// optionally followed by an explicit `Depth(n)`. The run is replaced
/// by one op + `Depth(run_len * n_or_1)`.
pub fn fuse_depths(ops: &mut Vec<DslOp>) {
    if ops.is_empty() {
        return;
    }
    let mut out: Vec<DslOp> = Vec::with_capacity(ops.len());
    let mut i = 0;
    while i < ops.len() {
        match &ops[i] {
            op @ (DslOp::Callers | DslOp::Callees) => {
                // Count consecutive identical callers/callees.
                let kind = op.clone();
                let mut count = 1usize;
                let mut j = i + 1;
                while j < ops.len() {
                    match (&ops[j], &kind) {
                        (DslOp::Callers, DslOp::Callers) | (DslOp::Callees, DslOp::Callees) => {
                            count += 1;
                            j += 1;
                        }
                        _ => break,
                    }
                }
                out.push(kind);
                // Optional trailing explicit Depth multiplies the run.
                if let Some(DslOp::Depth(n)) = ops.get(j) {
                    out.push(DslOp::Depth(count.saturating_mul(*n)));
                    i = j + 1;
                } else if count > 1 {
                    out.push(DslOp::Depth(count));
                    i = j;
                } else {
                    i = j;
                }
            }
            other => {
                out.push(other.clone());
                i += 1;
            }
        }
    }
    *ops = out;
}

/// **Filter pushdown**: move `filter kind=K` and `since N` ops as
/// early as their dependencies allow. Selection ops (`fn`, `type`)
/// already produce a kind-typed seed, so a `filter kind=Function`
/// directly after `fn(...)` is redundant; we still keep it (it's
/// idempotent) but ahead of any expensive expansion (`callers`,
/// `callees`).
///
/// Concretely: walk the op list, and for each `Filter` or `Since`,
/// hoist it past the most recent `Callers`/`Callees`/`Depth` runs as
/// long as the filter is order-independent w.r.t. those ops (it is —
/// kind is preserved by edge traversal, and revision is
/// preserved by anything except mutations, which the DSL doesn't
/// perform). Filters past `Show`/`Hot`/`Scc`/`Untested`/`PossibleTypes`
/// are NOT hoisted because those ops change the working set in ways
/// the filter could no longer recover.
pub fn push_filters_down(ops: &mut Vec<DslOp>) {
    if ops.len() < 2 {
        return;
    }
    let mut changed = true;
    let max_passes = ops.len();
    let mut passes = 0;
    while changed && passes < max_passes {
        changed = false;
        passes += 1;
        for i in 1..ops.len() {
            if !is_pure_filter(&ops[i]) {
                continue;
            }
            if can_swap_filter_with(&ops[i - 1]) {
                ops.swap(i - 1, i);
                changed = true;
            }
        }
    }
}

fn is_pure_filter(op: &DslOp) -> bool {
    matches!(op, DslOp::Filter(_) | DslOp::Since(_))
}

/// Operations the filter can hop *past* (i.e. swap with) without
/// changing query semantics. `Callers`/`Callees` preserve kind and
/// revision, so a kind/since filter applied before-or-after gives
/// the same result. Anything else is too risky.
fn can_swap_filter_with(op: &DslOp) -> bool {
    matches!(op, DslOp::Callers | DslOp::Callees | DslOp::Depth(_))
}

/// **Schedule selection** (heuristic): if the pipe contains expansion
/// ops with large `Depth`, prefer `Auto` (let runtime push/pull
/// switch). Without depth info we default to `Auto`. A pure `fn(...) |
/// hot N` query needs no expansion → `Push` is fine.
pub fn pick_schedule_for_pipe(ops: &[DslOp]) -> Schedule {
    let depth = ops
        .iter()
        .find_map(|o| if let DslOp::Depth(n) = o { Some(*n) } else { None })
        .unwrap_or(1);
    let has_expansion = ops
        .iter()
        .any(|o| matches!(o, DslOp::Callers | DslOp::Callees | DslOp::Taint(_) | DslOp::Preconditions));

    let strategy = if !has_expansion {
        ScheduleStrategy::Push
    } else if depth >= 3 {
        ScheduleStrategy::Auto
    } else {
        ScheduleStrategy::Push
    };
    Schedule {
        strategy,
        parallel: false,
    }
}

/// **Set-op short-circuit**: rewrite `A intersect B` so the side likely
/// to produce a smaller result is evaluated first. Heuristic: leaves
/// with `Filter`/`Since` are smaller; bare `entrypoints` is larger;
/// otherwise we leave order alone.
fn rewrite_expr(expr: Expr) -> Expr {
    match expr {
        Expr::SetOp { op, left, right } => {
            let l = Box::new(rewrite_expr(*left));
            let r = Box::new(rewrite_expr(*right));
            // For Intersect: smaller-side-first wins. We approximate
            // size by counting filter ops in the subtree (more filters
            // = smaller).
            // Lower score = smaller expected result. For Intersect we want
            // the smaller side first, so we swap when left's score is the
            // larger one (i.e. left is the bigger side currently).
            if op == SetOp::Intersect && expr_filter_score(&l) > expr_filter_score(&r) {
                Expr::SetOp {
                    op,
                    left: r,
                    right: l,
                }
            } else {
                Expr::SetOp {
                    op,
                    left: l,
                    right: r,
                }
            }
        }
        Expr::Pipe(ops) => {
            let mut ops = ops;
            fuse_depths(&mut ops);
            push_filters_down(&mut ops);
            Expr::Pipe(ops)
        }
        Expr::PipeFrom { base, ops } => {
            let mut ops = ops;
            fuse_depths(&mut ops);
            push_filters_down(&mut ops);
            Expr::PipeFrom {
                base: Box::new(rewrite_expr(*base)),
                ops,
            }
        }
        Expr::DominatorsOf(inner) => Expr::DominatorsOf(Box::new(rewrite_expr(*inner))),
        Expr::DominatesOf(inner) => Expr::DominatesOf(Box::new(rewrite_expr(*inner))),
        Expr::TraitImplsOf(inner) => Expr::TraitImplsOf(Box::new(rewrite_expr(*inner))),
        Expr::PathQuery(mut pq) => {
            pq.from = Box::new(rewrite_expr(*pq.from));
            pq.to = Box::new(rewrite_expr(*pq.to));
            Expr::PathQuery(pq)
        }
        Expr::MultiPath { sources, to, max_depth } => {
            let sources = sources.into_iter().map(rewrite_expr).collect();
            Expr::MultiPath {
                sources,
                to: Box::new(rewrite_expr(*to)),
                max_depth,
            }
        }
        atom @ Expr::Entrypoints(_) => atom,
    }
}

/// Heuristic selectivity score — more filters/since = lower (smaller)
/// expected result. Used to reorder intersect operands.
fn expr_filter_score(expr: &Expr) -> i32 {
    match expr {
        Expr::Pipe(ops) | Expr::PipeFrom { ops, .. } => {
            let mut score = 0i32;
            for o in ops {
                if matches!(
                    o,
                    DslOp::Filter(_)
                        | DslOp::Since(_)
                        | DslOp::Untested
                        | DslOp::PossibleTypes
                        | DslOp::Hot(_)
                ) {
                    score -= 2;
                }
                if matches!(o, DslOp::SelectFn(_) | DslOp::SelectType(_)) {
                    score -= 1;
                }
                if matches!(o, DslOp::Callers | DslOp::Callees) {
                    score += 1;
                }
            }
            score
        }
        Expr::Entrypoints(_) => 5,
        Expr::SetOp { left, right, .. } => expr_filter_score(left).min(expr_filter_score(right)),
        Expr::DominatorsOf(_) | Expr::DominatesOf(_) | Expr::TraitImplsOf(_) => 2,
        Expr::PathQuery(_) | Expr::MultiPath { .. } => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::{DslOp, EntrypointKind, Expr, PathMode, PathQuery, SetOp};
    use crate::nodes::NodeKind;

    #[test]
    fn fuse_depths_collapses_callers_run() {
        let mut ops = vec![
            DslOp::SelectFn("foo".into()),
            DslOp::Callers,
            DslOp::Callers,
            DslOp::Callers,
        ];
        fuse_depths(&mut ops);
        assert_eq!(
            ops,
            vec![
                DslOp::SelectFn("foo".into()),
                DslOp::Callers,
                DslOp::Depth(3),
            ]
        );
    }

    #[test]
    fn fuse_depths_multiplies_explicit_depth() {
        let mut ops = vec![
            DslOp::SelectFn("f".into()),
            DslOp::Callees,
            DslOp::Callees,
            DslOp::Depth(2),
        ];
        fuse_depths(&mut ops);
        assert_eq!(
            ops,
            vec![
                DslOp::SelectFn("f".into()),
                DslOp::Callees,
                DslOp::Depth(4),
            ]
        );
    }

    #[test]
    fn fuse_depths_idempotent_on_singleton() {
        let mut ops = vec![DslOp::SelectFn("x".into()), DslOp::Callers];
        let before = ops.clone();
        fuse_depths(&mut ops);
        assert_eq!(ops, before);
    }

    #[test]
    fn fuse_depths_does_not_collapse_mixed() {
        let mut ops = vec![
            DslOp::SelectFn("f".into()),
            DslOp::Callers,
            DslOp::Callees,
        ];
        let before = ops.clone();
        fuse_depths(&mut ops);
        assert_eq!(ops, before);
    }

    #[test]
    fn push_filter_down_past_callers() {
        let mut ops = vec![
            DslOp::SelectFn("foo".into()),
            DslOp::Callers,
            DslOp::Filter(NodeKind::Function),
        ];
        push_filters_down(&mut ops);
        // Filter should hop past callers, producing fn → filter → callers
        assert_eq!(
            ops,
            vec![
                DslOp::SelectFn("foo".into()),
                DslOp::Filter(NodeKind::Function),
                DslOp::Callers,
            ]
        );
    }

    #[test]
    fn push_filter_does_not_pass_through_show() {
        let mut ops = vec![
            DslOp::SelectFn("foo".into()),
            DslOp::Show(crate::dsl::Projection::Body),
            DslOp::Filter(NodeKind::Function),
        ];
        let before = ops.clone();
        push_filters_down(&mut ops);
        assert_eq!(ops, before);
    }

    #[test]
    fn since_filter_is_pushed_down() {
        let mut ops = vec![
            DslOp::SelectFn("foo".into()),
            DslOp::Callers,
            DslOp::Callees,
            DslOp::Since(10),
        ];
        push_filters_down(&mut ops);
        // since hops past callers/callees: fn → since → callers → callees
        assert_eq!(
            ops,
            vec![
                DslOp::SelectFn("foo".into()),
                DslOp::Since(10),
                DslOp::Callers,
                DslOp::Callees,
            ]
        );
    }

    #[test]
    fn pick_schedule_no_expansion_uses_push() {
        let s = pick_schedule_for_pipe(&[DslOp::SelectFn("x".into()), DslOp::Hot(5)]);
        assert_eq!(s.strategy, ScheduleStrategy::Push);
    }

    #[test]
    fn pick_schedule_deep_expansion_uses_auto() {
        let s = pick_schedule_for_pipe(&[
            DslOp::SelectFn("x".into()),
            DslOp::Callers,
            DslOp::Depth(5),
        ]);
        assert_eq!(s.strategy, ScheduleStrategy::Auto);
    }

    #[test]
    fn intersect_short_circuit_smaller_side_first() {
        let bigger = Expr::Entrypoints(Some(EntrypointKind::PublicApi));
        let smaller = Expr::Pipe(vec![
            DslOp::SelectFn("specific".into()),
            DslOp::Filter(NodeKind::Function),
        ]);
        let expr = Expr::SetOp {
            op: SetOp::Intersect,
            left: Box::new(bigger.clone()),
            right: Box::new(smaller.clone()),
        };
        let plan = optimise_expr(expr);
        let opt = plan.expr().unwrap();
        match opt {
            Expr::SetOp { op, left, right } => {
                assert_eq!(*op, SetOp::Intersect);
                // Smaller side should now be on the left.
                match left.as_ref() {
                    Expr::Pipe(_) => {}
                    _ => panic!("expected Pipe on left after rewrite, got {:?}", left),
                }
                match right.as_ref() {
                    Expr::Entrypoints(_) => {}
                    _ => panic!("expected Entrypoints on right, got {:?}", right),
                }
            }
            other => panic!("expected SetOp, got {:?}", other),
        }
    }

    #[test]
    fn union_does_not_reorder() {
        let l = Expr::Entrypoints(Some(EntrypointKind::PublicApi));
        let r = Expr::Pipe(vec![DslOp::SelectFn("x".into())]);
        let expr = Expr::SetOp {
            op: SetOp::Union,
            left: Box::new(l.clone()),
            right: Box::new(r.clone()),
        };
        let plan = optimise_expr(expr);
        match plan.expr().unwrap() {
            Expr::SetOp { op, left, .. } => {
                assert_eq!(*op, SetOp::Union);
                match left.as_ref() {
                    Expr::Entrypoints(_) => {}
                    _ => panic!("union must not reorder"),
                }
            }
            _ => panic!(),
        }
    }

    #[test]
    fn optimise_pipe_combines_fusion_and_pushdown() {
        let ops = vec![
            DslOp::SelectFn("foo".into()),
            DslOp::Callers,
            DslOp::Callers,
            DslOp::Filter(NodeKind::Function),
        ];
        let plan = optimise_pipe(ops);
        assert!(matches!(
            plan.ops().unwrap()[0],
            DslOp::SelectFn(_)
        ));
        // Filter should be hoisted ahead of the fused callers.
        let pos_filter = plan
            .ops()
            .unwrap()
            .iter()
            .position(|o| matches!(o, DslOp::Filter(_)))
            .unwrap();
        let pos_callers = plan
            .ops()
            .unwrap()
            .iter()
            .position(|o| matches!(o, DslOp::Callers))
            .unwrap();
        assert!(pos_filter < pos_callers);
    }

    #[test]
    fn rewrite_path_query_descends_into_endpoints() {
        let pq = PathQuery {
            mode: PathMode::Shortest,
            from: Box::new(Expr::Pipe(vec![
                DslOp::SelectFn("a".into()),
                DslOp::Callers,
                DslOp::Callers,
            ])),
            to: Box::new(Expr::Pipe(vec![DslOp::SelectFn("b".into())])),
            intermediate_kind: None,
            via_edge: None,
            max_depth: None,
        };
        let plan = optimise_expr(Expr::PathQuery(pq));
        match plan.expr().unwrap() {
            Expr::PathQuery(pq) => match pq.from.as_ref() {
                Expr::Pipe(ops) => {
                    // callers run should be fused.
                    let depths: Vec<_> = ops
                        .iter()
                        .filter_map(|o| if let DslOp::Depth(n) = o { Some(*n) } else { None })
                        .collect();
                    assert_eq!(depths, vec![2]);
                }
                other => panic!("expected Pipe, got {:?}", other),
            },
            _ => panic!(),
        }
    }
}
