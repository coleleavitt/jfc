//! Field-sensitive, flow-insensitive points-to analysis (Andersen-style).
//!
//! Operates over the language-agnostic IR ([`crate::ir::IrFunction`]) emitted
//! by each adapter's [`crate::ir::IrLowering`] driver. For each named source
//! variable in a function body, computes the set of [`AbstractLocation`]s it
//! may point to.
//!
//! # Why field-sensitive?
//!
//! Coarse pointer analysis collapses every struct field into a single
//! "heap" abstraction, which destroys precision for downstream taint /
//! slicing. By distinguishing `obj.x` from `obj.y` we can answer "could
//! tainted data reach `db.execute(s.query)`?" without first sanitizing
//! the entire `s` struct.
//!
//! # Algorithm
//!
//! Worklist-driven constraint solver. Each [`IrOp`] is interpreted into
//! one of the four classic Andersen constraints:
//!
//! | IR shape                                | Constraint                                                 |
//! |-----------------------------------------|------------------------------------------------------------|
//! | `dst = literal` / `dst = call(...)`     | `pts(dst) ⊇ { new heap location }` (allocation site)        |
//! | `dst = src`     (`src: Var`)            | `pts(dst) ⊇ pts(src)`                                       |
//! | `dst = base.field`                      | `pts(dst) ⊇ { Field(l, field) | l ∈ pts(base) }`            |
//! | `base.field = src`                      | for each `l ∈ pts(base)`: `pts(Field(l, field)) ⊇ pts(src)` |
//!
//! Flow-insensitive: ordering of statements is ignored, so the resulting
//! pts-sets are an over-approximation. This is the standard, well-studied
//! soundness/precision tradeoff used by LLVM's basic-aa and SVF's Andersen
//! solver — good enough for "could X alias Y" questions, less precise than
//! a sparse flow-sensitive pass.
//!
//! # Interprocedural mode
//!
//! [`analyze_interprocedural`] enriches per-function results by propagating
//! caller argument pts-sets into callee parameter pts-sets (and callee
//! return values back to caller call-site destinations) across `Calls`
//! edges. Fixed-point iteration (max 8 rounds) ensures transitive
//! dataflow stabilizes.
//!
//! # Limitations
//!
//! - Branch- and loop-insensitive — all paths are merged.
//! - Recursion through fields is bounded by [`MAX_FIELD_DEPTH`] so the
//!   worklist always terminates even on pathologic cyclic field stores.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use crate::edges::EdgeKind;
use crate::graph::CodeGraph;
use crate::ir::{IrFunction, IrOp, Operand, Var};
use crate::nodes::NodeId;

/// Maximum nesting depth for `Field(Field(...))` abstract locations. Bounds
/// the worklist termination cost on cyclic field stores like
/// `node.next = node` → `Field(node, next) → Field(Field(node, next), next)`.
const MAX_FIELD_DEPTH: usize = 8;

/// An abstract memory location that a variable can point to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AbstractLocation {
    /// Concrete allocation site keyed by (function-local) instruction index.
    Heap(usize),
    /// A function parameter — initial pts-set is `{ Param(name) }` so
    /// callers can later wire interprocedural flow.
    Param(String),
    /// An anonymous literal constant (every literal gets one location).
    Literal(usize),
    /// `Field(base, field_name)` — the storage cell for `base.field`.
    Field(Box<AbstractLocation>, String),
}

impl AbstractLocation {
    pub fn heap(site: usize) -> Self {
        AbstractLocation::Heap(site)
    }

    pub fn param(name: impl Into<String>) -> Self {
        AbstractLocation::Param(name.into())
    }

    pub fn literal(idx: usize) -> Self {
        AbstractLocation::Literal(idx)
    }

    pub fn field(base: AbstractLocation, name: impl Into<String>) -> Self {
        AbstractLocation::Field(Box::new(base), name.into())
    }

    /// `Field(Field(Field(...)))` nesting depth. Used to enforce
    /// [`MAX_FIELD_DEPTH`].
    fn depth(&self) -> usize {
        match self {
            AbstractLocation::Field(inner, _) => 1 + inner.depth(),
            _ => 0,
        }
    }
}

/// Map from variable to the set of [`AbstractLocation`]s it may point to,
/// plus an auxiliary map from `Field` locations to their pts-sets (for
/// `o.f` storage cells).
#[derive(Debug, Default, Clone)]
pub struct PointsToTable {
    /// `pts(v)` for every source variable.
    pub vars: BTreeMap<Var, BTreeSet<AbstractLocation>>,
    /// `pts(Field(...))` for every observed field cell.
    pub fields: BTreeMap<AbstractLocation, BTreeSet<AbstractLocation>>,
}

impl PointsToTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Points-to set for a named variable. Returns `None` if the variable
    /// was never assigned in the analysed function.
    pub fn pts_of(&self, v: &Var) -> Option<&BTreeSet<AbstractLocation>> {
        self.vars.get(v)
    }

    /// Points-to set for an abstract `Field(...)` cell.
    pub fn field_pts(&self, loc: &AbstractLocation) -> Option<&BTreeSet<AbstractLocation>> {
        self.fields.get(loc)
    }

    /// `true` iff `a` and `b` could refer to overlapping memory (i.e. the
    /// intersection of their pts-sets is non-empty).
    pub fn may_alias(&self, a: &Var, b: &Var) -> bool {
        match (self.vars.get(a), self.vars.get(b)) {
            (Some(pa), Some(pb)) => pa.intersection(pb).next().is_some(),
            _ => false,
        }
    }

    /// Add `loc` to `pts(v)`. Returns `true` if the set grew.
    fn add_var(&mut self, v: Var, loc: AbstractLocation) -> bool {
        self.vars.entry(v).or_default().insert(loc)
    }

    /// Bulk-add `src ⊆ pts(v)`. Returns `true` if the set grew.
    fn add_var_set(&mut self, v: Var, src: &BTreeSet<AbstractLocation>) -> bool {
        let entry = self.vars.entry(v).or_default();
        let before = entry.len();
        for loc in src {
            entry.insert(loc.clone());
        }
        entry.len() > before
    }

    /// Add `loc` to `pts(Field(...))`. Returns `true` if it grew.
    fn add_field(&mut self, fld: AbstractLocation, loc: AbstractLocation) -> bool {
        if fld.depth() > MAX_FIELD_DEPTH {
            return false;
        }
        self.fields.entry(fld).or_default().insert(loc)
    }
}

/// Run Andersen-style field-sensitive points-to analysis on a single
/// function. Returns the saturated [`PointsToTable`].
pub fn analyze(ir: &IrFunction) -> PointsToTable {
    let mut pts = PointsToTable::new();
    seed_params(ir, &mut pts);
    seed_constraints(ir, &mut pts);
    saturate_worklist(ir, &mut pts);
    pts
}

/// Phase 1: each parameter starts pointing to its own abstract location so
/// callers can later wire interprocedural flow.
fn seed_params(ir: &IrFunction, pts: &mut PointsToTable) {
    for p in &ir.params {
        pts.add_var(p.clone(), AbstractLocation::param(p.as_str()));
    }
}

/// Phase 2: single-pass constraint generation. Because we're
/// flow-insensitive, the statement order within `body` doesn't change the
/// final fixed point. Each Call/Const gets a fresh allocation site.
fn seed_constraints(ir: &IrFunction, pts: &mut PointsToTable) {
    let mut literal_idx: usize = 0;
    for (op_idx, op) in ir.body.iter().enumerate() {
        match op {
            IrOp::Assign { dst, src } => {
                seed_assign(dst, src, pts, &mut literal_idx);
            }
            IrOp::Call { dst, .. } => {
                if let Some(d) = dst {
                    pts.add_var(d.clone(), AbstractLocation::heap(op_idx));
                }
            }
            IrOp::FieldRead { dst, base, field } => {
                apply_field_read(dst, base, field, pts);
            }
            IrOp::FieldWrite { base, field, src } => {
                apply_field_write(base, field, src, pts);
            }
            IrOp::BinOp { dst, .. } => {
                pts.add_var(dst.clone(), AbstractLocation::literal(literal_idx));
                literal_idx += 1;
            }
            IrOp::Branch { .. }
            | IrOp::Jump { .. }
            | IrOp::Return { .. }
            | IrOp::Label(_)
            | IrOp::Nop => {}
        }
    }
}

fn seed_assign(dst: &Var, src: &Operand, pts: &mut PointsToTable, lit: &mut usize) {
    match src {
        Operand::Var(v) => {
            if let Some(set) = pts.vars.get(v).cloned() {
                pts.add_var_set(dst.clone(), &set);
            }
        }
        Operand::Const(_) => {
            pts.add_var(dst.clone(), AbstractLocation::literal(*lit));
            *lit += 1;
        }
        Operand::Temp(t) => {
            let tv = Var::new(format!("__t{t}"));
            if let Some(set) = pts.vars.get(&tv).cloned() {
                pts.add_var_set(dst.clone(), &set);
            }
        }
    }
}

fn apply_field_read(dst: &Var, base: &Operand, field: &str, pts: &mut PointsToTable) {
    let base_set = base_pts(pts, base).into_owned();
    for base_loc in &base_set {
        let cell = AbstractLocation::field(base_loc.clone(), field.to_owned());
        if let Some(field_pts) = pts.fields.get(&cell).cloned() {
            pts.add_var_set(dst.clone(), &field_pts);
        } else {
            pts.add_var(dst.clone(), cell);
        }
    }
}

fn apply_field_write(base: &Operand, field: &str, src: &Operand, pts: &mut PointsToTable) {
    let base_set = base_pts(pts, base).into_owned();
    let src_set: BTreeSet<AbstractLocation> = operand_pts(pts, src).into_iter().collect();
    for base_loc in &base_set {
        let cell = AbstractLocation::field(base_loc.clone(), field.to_owned());
        for s in &src_set {
            pts.add_field(cell.clone(), s.clone());
        }
    }
}

/// Phase 3: worklist saturation. Re-scan until no pts-set grew.
fn saturate_worklist(ir: &IrFunction, pts: &mut PointsToTable) {
    let mut worklist: VecDeque<usize> = (0..ir.body.len()).collect();
    let mut rounds: usize = 0;
    const MAX_ROUNDS: usize = 32;
    while let Some(idx) = worklist.pop_front() {
        if rounds > MAX_ROUNDS * ir.body.len() {
            break;
        }
        rounds += 1;

        let changed = match &ir.body[idx] {
            IrOp::Assign {
                dst,
                src: Operand::Var(v),
            } => match pts.vars.get(v).cloned() {
                Some(set) => pts.add_var_set(dst.clone(), &set),
                None => false,
            },
            IrOp::FieldRead { dst, base, field } => worklist_field_read(dst, base, field, pts),
            IrOp::FieldWrite { base, field, src } => worklist_field_write(base, field, src, pts),
            _ => false,
        };

        if changed {
            for i in 0..ir.body.len() {
                worklist.push_back(i);
            }
        }
    }
}

fn worklist_field_read(dst: &Var, base: &Operand, field: &str, pts: &mut PointsToTable) -> bool {
    let mut changed = false;
    let base_set = base_pts(pts, base).into_owned();
    for base_loc in &base_set {
        let cell = AbstractLocation::field(base_loc.clone(), field.to_owned());
        if let Some(fs) = pts.fields.get(&cell).cloned() {
            if pts.add_var_set(dst.clone(), &fs) {
                changed = true;
            }
        }
    }
    changed
}

fn worklist_field_write(
    base: &Operand,
    field: &str,
    src: &Operand,
    pts: &mut PointsToTable,
) -> bool {
    let mut changed = false;
    let base_set = base_pts(pts, base).into_owned();
    let src_set: BTreeSet<AbstractLocation> = operand_pts(pts, src).into_iter().collect();
    for base_loc in &base_set {
        let cell = AbstractLocation::field(base_loc.clone(), field.to_owned());
        for s in &src_set {
            if pts.add_field(cell.clone(), s.clone()) {
                changed = true;
            }
        }
    }
    changed
}

/// Resolve an [`Operand`] to its current pts-set (cloning).
fn base_pts<'a>(
    pts: &'a PointsToTable,
    operand: &Operand,
) -> std::borrow::Cow<'a, BTreeSet<AbstractLocation>> {
    match operand {
        Operand::Var(v) => match pts.vars.get(v) {
            Some(s) => std::borrow::Cow::Borrowed(s),
            None => std::borrow::Cow::Owned(BTreeSet::new()),
        },
        Operand::Temp(t) => {
            let tv = Var::new(format!("__t{t}"));
            match pts.vars.get(&tv) {
                Some(s) => std::borrow::Cow::Borrowed(s),
                None => std::borrow::Cow::Owned(BTreeSet::new()),
            }
        }
        Operand::Const(_) => std::borrow::Cow::Owned(BTreeSet::new()),
    }
}

/// Resolve an [`Operand`] to a concrete location set used for FieldWrite
/// source values. Literals get a fresh anonymous location so the
/// destination field cell records *something*.
fn operand_pts(pts: &PointsToTable, operand: &Operand) -> Vec<AbstractLocation> {
    match operand {
        Operand::Var(v) => pts.vars.get(v).into_iter().flatten().cloned().collect(),
        Operand::Temp(t) => {
            let tv = Var::new(format!("__t{t}"));
            pts.vars.get(&tv).into_iter().flatten().cloned().collect()
        }
        Operand::Const(_) => vec![AbstractLocation::literal(usize::MAX)],
    }
}

// ─── Interprocedural analysis ────────────────────────────────────────────────

/// Maximum fixed-point iteration rounds for interprocedural propagation.
const MAX_INTERPROC_ROUNDS: usize = 8;

/// Run interprocedural points-to analysis across the call graph.
///
/// 1. Compute intraprocedural results for each function.
/// 2. Propagate argument pts-sets into callee parameters and callee return
///    pts-sets back into caller call-site destinations.
/// 3. Re-analyze callees with enriched seeds until stable (max 8 rounds).
pub fn analyze_interprocedural(
    graph: &CodeGraph,
    ir_map: &HashMap<NodeId, IrFunction>,
) -> HashMap<NodeId, PointsToTable> {
    let mut tables: HashMap<NodeId, PointsToTable> = ir_map
        .iter()
        .map(|(id, ir)| (id.clone(), analyze(ir)))
        .collect();

    for _round in 0..MAX_INTERPROC_ROUNDS {
        if !propagate_one_round(graph, ir_map, &mut tables) {
            break;
        }
    }
    tables
}

/// Single propagation round. Returns `true` if any table grew.
fn propagate_one_round(
    graph: &CodeGraph,
    ir_map: &HashMap<NodeId, IrFunction>,
    tables: &mut HashMap<NodeId, PointsToTable>,
) -> bool {
    let mut changed = false;
    let caller_ids: Vec<NodeId> = tables.keys().cloned().collect();

    for caller_id in &caller_ids {
        let edges = graph.get_edges_from(caller_id);
        let call_edges: Vec<NodeId> = edges
            .iter()
            .filter(|(_, ed)| matches!(ed.kind, EdgeKind::Calls))
            .map(|(target, _)| (*target).clone())
            .collect();

        for callee_id in &call_edges {
            if propagate_caller_to_callee(caller_id, callee_id, ir_map, tables) {
                changed = true;
            }
        }
    }
    changed
}

/// Propagate argument and return pts-sets between a caller-callee pair.
fn propagate_caller_to_callee(
    caller_id: &NodeId,
    callee_id: &NodeId,
    ir_map: &HashMap<NodeId, IrFunction>,
    tables: &mut HashMap<NodeId, PointsToTable>,
) -> bool {
    let (Some(caller_ir), Some(callee_ir)) = (ir_map.get(caller_id), ir_map.get(callee_id)) else {
        return false;
    };

    let arg_seeds = collect_arg_seeds(caller_ir, callee_ir, tables.get(caller_id));
    let ret_seeds = collect_ret_seeds(callee_ir, tables.get(callee_id));
    let call_dsts = collect_call_dsts(caller_ir, &callee_ir.name);

    let mut changed = false;
    if let Some(callee_pts) = tables.get_mut(callee_id) {
        for (param, locs) in &arg_seeds {
            if callee_pts.add_var_set(param.clone(), locs) {
                changed = true;
            }
        }
    }
    if let Some(caller_pts) = tables.get_mut(caller_id) {
        for dst in &call_dsts {
            if caller_pts.add_var_set(dst.clone(), &ret_seeds) {
                changed = true;
            }
        }
    }
    changed
}

/// Map caller argument pts-sets to callee parameter variables.
fn collect_arg_seeds(
    caller_ir: &IrFunction,
    callee_ir: &IrFunction,
    caller_pts: Option<&PointsToTable>,
) -> Vec<(Var, BTreeSet<AbstractLocation>)> {
    let Some(pts) = caller_pts else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for op in &caller_ir.body {
        let IrOp::Call { callee, args, .. } = op else {
            continue;
        };
        if callee != &callee_ir.name {
            continue;
        }
        for (i, arg) in args.iter().enumerate() {
            let Some(param) = callee_ir.params.get(i) else {
                continue;
            };
            let arg_set = operand_pts_set(pts, arg);
            if !arg_set.is_empty() {
                result.push((param.clone(), arg_set));
            }
        }
    }
    result
}

/// Collect the pts-set of all return values in a callee.
fn collect_ret_seeds(
    callee_ir: &IrFunction,
    callee_pts: Option<&PointsToTable>,
) -> BTreeSet<AbstractLocation> {
    let Some(pts) = callee_pts else {
        return BTreeSet::new();
    };
    let mut ret_set = BTreeSet::new();
    for op in &callee_ir.body {
        let IrOp::Return { value: Some(val) } = op else {
            continue;
        };
        for loc in operand_pts_set(pts, val) {
            ret_set.insert(loc);
        }
    }
    ret_set
}

/// Find all destination variables in the caller for calls to `callee_name`.
fn collect_call_dsts(caller_ir: &IrFunction, callee_name: &str) -> Vec<Var> {
    caller_ir
        .body
        .iter()
        .filter_map(|op| match op {
            IrOp::Call {
                dst: Some(d),
                callee,
                ..
            } if callee == callee_name => Some(d.clone()),
            _ => None,
        })
        .collect()
}

/// Resolve an operand's pts-set for interprocedural seed building.
fn operand_pts_set(pts: &PointsToTable, operand: &Operand) -> BTreeSet<AbstractLocation> {
    match operand {
        Operand::Var(v) => pts.vars.get(v).cloned().unwrap_or_default(),
        Operand::Temp(t) => {
            let tv = Var::new(format!("__t{t}"));
            pts.vars.get(&tv).cloned().unwrap_or_default()
        }
        Operand::Const(_) => BTreeSet::new(),
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{BinOpKind, IrFunction, IrOp, Operand, Var};

    #[test]
    fn param_seeds_with_self_location() {
        let mut f = IrFunction::new("f");
        f.params.push(Var::new("x"));
        let pts = analyze(&f);
        let set = pts.pts_of(&Var::new("x")).expect("param tracked");
        assert!(set.contains(&AbstractLocation::param("x")));
    }

    #[test]
    fn copy_propagates_pts_set() {
        // y = x; → pts(y) ⊇ pts(x)
        let mut f = IrFunction::new("f");
        f.params.push(Var::new("x"));
        f.push(IrOp::Assign {
            dst: Var::new("y"),
            src: Operand::Var(Var::new("x")),
        });
        let pts = analyze(&f);
        let x_set = pts.pts_of(&Var::new("x")).unwrap().clone();
        let y_set = pts.pts_of(&Var::new("y")).unwrap();
        assert!(x_set.is_subset(y_set), "pts(y) should ⊇ pts(x)");
    }

    #[test]
    fn aliasing_through_copy() {
        // y = x;  z = x;  → may_alias(y, z) == true
        let mut f = IrFunction::new("f");
        f.params.push(Var::new("x"));
        f.push(IrOp::Assign {
            dst: Var::new("y"),
            src: Operand::Var(Var::new("x")),
        });
        f.push(IrOp::Assign {
            dst: Var::new("z"),
            src: Operand::Var(Var::new("x")),
        });
        let pts = analyze(&f);
        assert!(pts.may_alias(&Var::new("y"), &Var::new("z")));
    }

    #[test]
    fn distinct_calls_get_distinct_heap_sites() {
        // a = call(); b = call();  → !may_alias(a, b)
        let mut f = IrFunction::new("f");
        f.push(IrOp::Call {
            dst: Some(Var::new("a")),
            callee: "new".into(),
            args: vec![],
        });
        f.push(IrOp::Call {
            dst: Some(Var::new("b")),
            callee: "new".into(),
            args: vec![],
        });
        let pts = analyze(&f);
        assert!(!pts.may_alias(&Var::new("a"), &Var::new("b")));
    }

    #[test]
    fn field_write_then_read_preserves_value() {
        // o = call();
        // o.f = x;
        // y = o.f;
        // → pts(y) should overlap pts(x)
        let mut f = IrFunction::new("f");
        f.params.push(Var::new("x"));
        f.push(IrOp::Call {
            dst: Some(Var::new("o")),
            callee: "new".into(),
            args: vec![],
        });
        f.push(IrOp::FieldWrite {
            base: Operand::Var(Var::new("o")),
            field: "f".into(),
            src: Operand::Var(Var::new("x")),
        });
        f.push(IrOp::FieldRead {
            dst: Var::new("y"),
            base: Operand::Var(Var::new("o")),
            field: "f".into(),
        });
        let pts = analyze(&f);
        let x_set = pts.pts_of(&Var::new("x")).unwrap().clone();
        let y_set = pts.pts_of(&Var::new("y")).unwrap();
        assert!(
            x_set.iter().any(|loc| y_set.contains(loc)),
            "pts(y) should contain at least one location from pts(x); got y={:?} x={:?}",
            y_set,
            x_set,
        );
    }

    #[test]
    fn distinct_fields_dont_alias() {
        // o = call();
        // o.f = x;
        // o.g = y;     (different field)
        // a = o.f;
        // b = o.g;
        // → !may_alias(a, b)
        let mut f = IrFunction::new("f");
        f.params.push(Var::new("x"));
        f.params.push(Var::new("y"));
        f.push(IrOp::Call {
            dst: Some(Var::new("o")),
            callee: "new".into(),
            args: vec![],
        });
        f.push(IrOp::FieldWrite {
            base: Operand::Var(Var::new("o")),
            field: "f".into(),
            src: Operand::Var(Var::new("x")),
        });
        f.push(IrOp::FieldWrite {
            base: Operand::Var(Var::new("o")),
            field: "g".into(),
            src: Operand::Var(Var::new("y")),
        });
        f.push(IrOp::FieldRead {
            dst: Var::new("a"),
            base: Operand::Var(Var::new("o")),
            field: "f".into(),
        });
        f.push(IrOp::FieldRead {
            dst: Var::new("b"),
            base: Operand::Var(Var::new("o")),
            field: "g".into(),
        });
        let pts = analyze(&f);
        assert!(
            !pts.may_alias(&Var::new("a"), &Var::new("b")),
            "field-sensitive analysis should not alias a (from o.f) with b (from o.g)"
        );
    }

    #[test]
    fn binop_result_is_anonymous_literal() {
        let mut f = IrFunction::new("f");
        f.params.push(Var::new("x"));
        f.push(IrOp::BinOp {
            dst: Var::new("y"),
            lhs: Operand::Var(Var::new("x")),
            op: BinOpKind::Add,
            rhs: Operand::Const("1".into()),
        });
        let pts = analyze(&f);
        let y_set = pts.pts_of(&Var::new("y")).unwrap();
        assert!(
            y_set
                .iter()
                .any(|l| matches!(l, AbstractLocation::Literal(_))),
            "binop result should yield a Literal location"
        );
    }

    #[test]
    fn cyclic_field_store_terminates() {
        // node.next = node;  (self-referential — bounded by MAX_FIELD_DEPTH)
        let mut f = IrFunction::new("f");
        f.push(IrOp::Call {
            dst: Some(Var::new("node")),
            callee: "new".into(),
            args: vec![],
        });
        f.push(IrOp::FieldWrite {
            base: Operand::Var(Var::new("node")),
            field: "next".into(),
            src: Operand::Var(Var::new("node")),
        });
        // If this loops forever the test runner will time out;
        // termination is the assertion.
        analyze(&f);
    }

    // ─── Interprocedural tests ───────────────────────────────────────────

    use crate::edges::EdgeData;
    use crate::graph::CodeGraph;
    use crate::nodes::{NodeData, NodeId, NodeKind, Span, Visibility};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn mk_span() -> Span {
        Span {
            file: PathBuf::from("test.rs"),
            start_line: 1,
            start_col: 0,
            end_line: 1,
            end_col: 0,
            byte_range: 0..0,
        }
    }

    fn mk_node_data(name: &str) -> NodeData {
        NodeData {
            id: NodeId::new("test.rs", name, NodeKind::Function),
            kind: NodeKind::Function,
            name: name.to_string(),
            qualified_name: name.to_string(),
            file_path: PathBuf::from("test.rs"),
            span: mk_span(),
            visibility: Visibility::Private,
            metadata: HashMap::new(),
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
        }
    }

    fn calls_edge() -> EdgeData {
        EdgeData {
            kind: EdgeKind::Calls,
            source_span: mk_span(),
            weight: 1.0,
        }
    }

    #[test]
    fn interprocedural_return_flows_to_caller() {
        // source() returns a param-location; main calls source() and
        // assigns to `x`. x's pts should contain source's return location.
        let mut source_ir = IrFunction::new("source");
        source_ir.params.push(Var::new("seed"));
        source_ir.push(IrOp::Return {
            value: Some(Operand::Var(Var::new("seed"))),
        });

        let mut main_ir = IrFunction::new("main");
        main_ir.push(IrOp::Call {
            dst: Some(Var::new("x")),
            callee: "source".into(),
            args: vec![],
        });

        let source_nd = mk_node_data("source");
        let main_nd = mk_node_data("main");
        let source_id = source_nd.id.clone();
        let main_id = main_nd.id.clone();

        let mut graph = CodeGraph::new();
        graph.add_node(source_nd);
        graph.add_node(main_nd);
        graph.add_edge(&main_id, &source_id, calls_edge()).unwrap();

        let mut ir_map: HashMap<NodeId, IrFunction> = HashMap::new();
        ir_map.insert(source_id.clone(), source_ir);
        ir_map.insert(main_id.clone(), main_ir);

        let tables = analyze_interprocedural(&graph, &ir_map);
        let main_pts = tables.get(&main_id).expect("main table");
        let x_pts = main_pts.pts_of(&Var::new("x")).expect("x tracked");

        // source's return is `seed` → Param("seed"); it should flow to x.
        assert!(
            x_pts.contains(&AbstractLocation::param("seed")),
            "expected Param(seed) in pts(x), got {:?}",
            x_pts,
        );
    }

    #[test]
    fn interprocedural_arg_flows_to_callee_param() {
        // main has param `data`, calls sink(data). sink's param `x` pts
        // should include main's Param("data") location.
        let mut main_ir = IrFunction::new("main");
        main_ir.params.push(Var::new("data"));
        main_ir.push(IrOp::Call {
            dst: None,
            callee: "sink".into(),
            args: vec![Operand::Var(Var::new("data"))],
        });

        let mut sink_ir = IrFunction::new("sink");
        sink_ir.params.push(Var::new("x"));

        let main_nd = mk_node_data("main");
        let sink_nd = mk_node_data("sink");
        let main_id = main_nd.id.clone();
        let sink_id = sink_nd.id.clone();

        let mut graph = CodeGraph::new();
        graph.add_node(main_nd);
        graph.add_node(sink_nd);
        graph.add_edge(&main_id, &sink_id, calls_edge()).unwrap();

        let mut ir_map: HashMap<NodeId, IrFunction> = HashMap::new();
        ir_map.insert(main_id.clone(), main_ir);
        ir_map.insert(sink_id.clone(), sink_ir);

        let tables = analyze_interprocedural(&graph, &ir_map);
        let sink_pts = tables.get(&sink_id).expect("sink table");
        let x_pts = sink_pts.pts_of(&Var::new("x")).expect("x tracked");

        assert!(
            x_pts.contains(&AbstractLocation::param("data")),
            "expected Param(data) in pts(x), got {:?}",
            x_pts,
        );
    }

    #[test]
    fn interprocedural_transitive_source_to_sink() {
        // source() → returns Param("taint")
        // sink(x) — just receives
        // main: sink(source())
        //
        // After propagation: sink's param should contain Param("taint").
        let mut source_ir = IrFunction::new("source");
        source_ir.params.push(Var::new("taint"));
        source_ir.push(IrOp::Return {
            value: Some(Operand::Var(Var::new("taint"))),
        });

        let mut sink_ir = IrFunction::new("sink");
        sink_ir.params.push(Var::new("x"));

        // main: tmp = source(); sink(tmp);
        let mut main_ir = IrFunction::new("main");
        main_ir.push(IrOp::Call {
            dst: Some(Var::new("tmp")),
            callee: "source".into(),
            args: vec![],
        });
        main_ir.push(IrOp::Call {
            dst: None,
            callee: "sink".into(),
            args: vec![Operand::Var(Var::new("tmp"))],
        });

        let source_nd = mk_node_data("source");
        let sink_nd = mk_node_data("sink");
        let main_nd = mk_node_data("main");
        let source_id = source_nd.id.clone();
        let sink_id = sink_nd.id.clone();
        let main_id = main_nd.id.clone();

        let mut graph = CodeGraph::new();
        graph.add_node(source_nd);
        graph.add_node(sink_nd);
        graph.add_node(main_nd);
        graph.add_edge(&main_id, &source_id, calls_edge()).unwrap();
        graph.add_edge(&main_id, &sink_id, calls_edge()).unwrap();

        let mut ir_map: HashMap<NodeId, IrFunction> = HashMap::new();
        ir_map.insert(source_id.clone(), source_ir);
        ir_map.insert(sink_id.clone(), sink_ir);
        ir_map.insert(main_id.clone(), main_ir);

        let tables = analyze_interprocedural(&graph, &ir_map);
        let sink_pts = tables.get(&sink_id).expect("sink table");
        let x_pts = sink_pts.pts_of(&Var::new("x")).expect("x tracked");

        assert!(
            x_pts.contains(&AbstractLocation::param("taint")),
            "transitive: expected Param(taint) in sink's pts(x), got {:?}",
            x_pts,
        );
    }
}
