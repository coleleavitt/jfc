//! Stratified negation for rule-based queries (Phase 10-3).
//!
//! ## Rationale
//!
//! Set difference (`A \ B`) exists in the DSL today. Datalog goes one
//! step further with **stratified negation**: rules can use negation
//! (`!P`) provided the program's predicate-dependency graph has no
//! cycle through a negated edge. Stratification computes a layering
//! such that all negated atoms in a rule for predicate `p` refer to
//! predicates of strictly lower stratum — guaranteeing semantics
//! match the perfect model.
//!
//! ## Surface
//!
//! - [`Rule`] — head + body (positive + negative dependencies).
//! - [`stratify`] — given a rule set, return either a per-predicate
//!   stratum number (0-indexed) or [`StrataError::NegationCycle`] if
//!   stratification is impossible.
//!
//! ## Why this is independent of the executor
//!
//! Stratification is a static analysis over the rule graph; it
//! doesn't itself evaluate anything. Once a rule set is stratified,
//! semi-naive evaluation processes strata bottom-up. The full
//! evaluator lives in [`crate::datalog`] (Phase 13); this module is
//! the static-analysis prerequisite.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use thiserror::Error;

/// A predicate name. `String` rather than a typed handle so the rule
/// language can grow new predicates without recompilation.
pub type PredicateName = String;

/// One Datalog rule. `head` is the predicate produced; `pos` are
/// predicates that must succeed; `neg` are predicates whose negation
/// must succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub head: PredicateName,
    pub pos: Vec<PredicateName>,
    pub neg: Vec<PredicateName>,
}

impl Rule {
    pub fn new(head: impl Into<String>) -> Self {
        Self {
            head: head.into(),
            pos: Vec::new(),
            neg: Vec::new(),
        }
    }
    pub fn pos(mut self, name: impl Into<String>) -> Self {
        self.pos.push(name.into());
        self
    }
    pub fn neg(mut self, name: impl Into<String>) -> Self {
        self.neg.push(name.into());
        self
    }
}

/// Errors from [`stratify`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum StrataError {
    #[error("negation cycle: predicate {predicate} depends on itself through negation")]
    NegationCycle { predicate: PredicateName },
}

/// Compute the per-predicate stratum number. Returns
/// `BTreeMap<PredicateName, usize>` keyed by predicate; the lowest
/// stratum is 0. The stratification rule:
///
/// - If `head` has a positive dependency on `body`, then
///   `stratum(head) >= stratum(body)`.
/// - If `head` has a *negative* dependency on `body`, then
///   `stratum(head) > stratum(body)` strictly.
///
/// A cycle that includes any negative edge → no valid stratification
/// exists → [`StrataError::NegationCycle`].
pub fn stratify(rules: &[Rule]) -> Result<BTreeMap<PredicateName, usize>, StrataError> {
    // Step 1: collect every predicate seen in heads or bodies.
    let mut all_preds: BTreeSet<PredicateName> = BTreeSet::new();
    for r in rules {
        all_preds.insert(r.head.clone());
        for p in r.pos.iter().chain(r.neg.iter()) {
            all_preds.insert(p.clone());
        }
    }

    // Step 2: build the dependency graph with edge labels {pos, neg}.
    // adjacency[head] = Vec<(body, is_negated)>
    let mut adj: HashMap<PredicateName, Vec<(PredicateName, bool)>> = HashMap::new();
    for r in rules {
        let entry = adj.entry(r.head.clone()).or_default();
        for p in &r.pos {
            entry.push((p.clone(), false));
        }
        for p in &r.neg {
            entry.push((p.clone(), true));
        }
    }

    // Step 3: detect SCCs (Tarjan). Inside an SCC, no edge may be
    // negated — a negation inside an SCC is by definition a
    // cycle-through-negation.
    let sccs = compute_sccs(&all_preds, &adj);

    for scc in &sccs {
        if scc.len() < 2 {
            // Singleton SCC: still a problem if it has a self-loop
            // through a negated edge.
            let only = scc.iter().next().unwrap();
            if let Some(edges) = adj.get(only) {
                for (target, neg) in edges {
                    if *neg && target == only {
                        return Err(StrataError::NegationCycle {
                            predicate: only.clone(),
                        });
                    }
                }
            }
            continue;
        }
        // Multi-node SCC: any internal negated edge is a violation.
        let scc_set: HashSet<&PredicateName> = scc.iter().collect();
        for member in scc {
            if let Some(edges) = adj.get(member) {
                for (target, neg) in edges {
                    if *neg && scc_set.contains(target) {
                        return Err(StrataError::NegationCycle {
                            predicate: member.clone(),
                        });
                    }
                }
            }
        }
    }

    // Step 4: Each SCC is a single stratum. Topologically order the
    // SCCs by their dependency edges (DAG of SCCs). The stratum
    // assignment is: stratum(p) = topo_order_of_its_scc, with one
    // increment for each negated cross-SCC edge into p's SCC.

    // Build SCC index map and condensation.
    let mut scc_of: HashMap<PredicateName, usize> = HashMap::new();
    for (i, scc) in sccs.iter().enumerate() {
        for member in scc {
            scc_of.insert(member.clone(), i);
        }
    }

    // SCC condensation edges.
    let mut scc_edges: Vec<HashSet<(usize, bool)>> = vec![HashSet::new(); sccs.len()];
    for (head, edges) in &adj {
        let from_scc = scc_of[head];
        for (body, neg) in edges {
            let to_scc = scc_of[body];
            if from_scc != to_scc {
                scc_edges[to_scc].insert((from_scc, *neg));
            }
        }
    }

    // Compute the stratum of each SCC: max over all incoming edges
    // of stratum(source_scc) + (negated ? 1 : 0). Process SCCs in
    // reverse-topological order (leaves first); since Tarjan
    // returns SCCs in reverse-topo order already, we process in
    // ascending index.
    //
    // Wait — `compute_sccs` returns in reverse-topo order. To
    // process leaves first we should iterate in given order (the
    // first SCC is a leaf in the condensation).
    //
    // Actually the dependency direction is: head depends on body, so
    // if A depends on B then we need stratum(A) >= stratum(B). We
    // compute strata starting from the SCCs with no outgoing
    // dependencies (the "leaves" — bodies with no further
    // dependencies). Tarjan's reverse-topo order on the original
    // graph gives us those first.
    let mut strata_of_scc: Vec<usize> = vec![0; sccs.len()];
    for i in 0..sccs.len() {
        // For each SCC i, look at the SCCs i depends on (via
        // outgoing edges from any member of i). Their stratum
        // determines i's.
        let mut max_dep_stratum: i64 = -1;
        for member in &sccs[i] {
            if let Some(edges) = adj.get(member) {
                for (body, neg) in edges {
                    let body_scc = scc_of[body];
                    if body_scc == i {
                        continue;
                    }
                    let dep_stratum = strata_of_scc[body_scc] as i64;
                    let bumped = dep_stratum + if *neg { 1 } else { 0 };
                    if bumped > max_dep_stratum {
                        max_dep_stratum = bumped;
                    }
                }
            }
        }
        strata_of_scc[i] = if max_dep_stratum < 0 { 0 } else { max_dep_stratum as usize };
    }

    let mut out = BTreeMap::new();
    for p in &all_preds {
        out.insert(p.clone(), strata_of_scc[scc_of[p]]);
    }
    Ok(out)
}

/// Tarjan SCC over the predicate-dependency graph.
fn compute_sccs(
    preds: &BTreeSet<PredicateName>,
    adj: &HashMap<PredicateName, Vec<(PredicateName, bool)>>,
) -> Vec<Vec<PredicateName>> {
    let mut index_of: HashMap<PredicateName, usize> = HashMap::new();
    let mut lowlink: HashMap<PredicateName, usize> = HashMap::new();
    let mut on_stack: HashSet<PredicateName> = HashSet::new();
    let mut stack: Vec<PredicateName> = Vec::new();
    let mut index: usize = 0;
    let mut sccs: Vec<Vec<PredicateName>> = Vec::new();

    fn strong(
        v: &PredicateName,
        adj: &HashMap<PredicateName, Vec<(PredicateName, bool)>>,
        index_of: &mut HashMap<PredicateName, usize>,
        lowlink: &mut HashMap<PredicateName, usize>,
        on_stack: &mut HashSet<PredicateName>,
        stack: &mut Vec<PredicateName>,
        index: &mut usize,
        sccs: &mut Vec<Vec<PredicateName>>,
    ) {
        index_of.insert(v.clone(), *index);
        lowlink.insert(v.clone(), *index);
        *index += 1;
        stack.push(v.clone());
        on_stack.insert(v.clone());

        if let Some(edges) = adj.get(v) {
            for (w, _neg) in edges {
                if !index_of.contains_key(w) {
                    strong(w, adj, index_of, lowlink, on_stack, stack, index, sccs);
                    let w_low = lowlink[w];
                    let v_low = lowlink[v];
                    lowlink.insert(v.clone(), v_low.min(w_low));
                } else if on_stack.contains(w) {
                    let w_idx = index_of[w];
                    let v_low = lowlink[v];
                    lowlink.insert(v.clone(), v_low.min(w_idx));
                }
            }
        }

        if lowlink[v] == index_of[v] {
            let mut comp = Vec::new();
            loop {
                let popped = stack.pop().unwrap();
                on_stack.remove(&popped);
                let done = popped == *v;
                comp.push(popped);
                if done {
                    break;
                }
            }
            sccs.push(comp);
        }
    }

    for p in preds {
        if !index_of.contains_key(p) {
            strong(
                p,
                adj,
                &mut index_of,
                &mut lowlink,
                &mut on_stack,
                &mut stack,
                &mut index,
                &mut sccs,
            );
        }
    }
    sccs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_two_strata() {
        // a :- b
        // c :- !a
        // → strata: a=0, b=0, c=1
        let rules = vec![Rule::new("a").pos("b"), Rule::new("c").neg("a")];
        let strata = stratify(&rules).unwrap();
        assert_eq!(strata["b"], 0);
        assert_eq!(strata["a"], 0);
        assert_eq!(strata["c"], 1);
    }

    #[test]
    fn negation_cycle_rejected() {
        // a :- !b
        // b :- !a    ← cycle through negation
        let rules = vec![Rule::new("a").neg("b"), Rule::new("b").neg("a")];
        let err = stratify(&rules).unwrap_err();
        assert!(matches!(err, StrataError::NegationCycle { .. }));
    }

    #[test]
    fn positive_cycle_ok() {
        // Mutual recursion via positive edges is fine.
        let rules = vec![Rule::new("a").pos("b"), Rule::new("b").pos("a")];
        let strata = stratify(&rules).unwrap();
        assert_eq!(strata["a"], strata["b"]);
    }

    #[test]
    fn self_loop_through_negation_rejected() {
        let rules = vec![Rule::new("a").neg("a")];
        let err = stratify(&rules).unwrap_err();
        assert!(matches!(err, StrataError::NegationCycle { .. }));
    }

    #[test]
    fn three_strata_chain() {
        // a :- b
        // c :- !a
        // d :- !c
        let rules = vec![
            Rule::new("a").pos("b"),
            Rule::new("c").neg("a"),
            Rule::new("d").neg("c"),
        ];
        let strata = stratify(&rules).unwrap();
        assert_eq!(strata["b"], 0);
        assert_eq!(strata["a"], 0);
        assert_eq!(strata["c"], 1);
        assert_eq!(strata["d"], 2);
    }

    #[test]
    fn empty_rule_set_yields_empty_map() {
        let strata = stratify(&[]).unwrap();
        assert!(strata.is_empty());
    }

    #[test]
    fn multiple_rules_for_one_predicate() {
        // a :- b
        // a :- !c
        // → a's stratum is max(stratum(b), stratum(c) + 1)
        let rules = vec![Rule::new("a").pos("b"), Rule::new("a").neg("c")];
        let strata = stratify(&rules).unwrap();
        assert!(strata["a"] > strata["c"]);
    }

    #[test]
    fn unrelated_predicates_get_lowest_stratum() {
        let rules = vec![Rule::new("x").pos("y"), Rule::new("p").pos("q")];
        let strata = stratify(&rules).unwrap();
        assert_eq!(strata["x"], 0);
        assert_eq!(strata["y"], 0);
        assert_eq!(strata["p"], 0);
        assert_eq!(strata["q"], 0);
    }

    #[test]
    fn diamond_dependency_strata() {
        // a :- b, !c
        // b :- d
        // c :- d
        // → strata: d=0, b=0, c=0, a=1
        let rules = vec![
            Rule::new("a").pos("b").neg("c"),
            Rule::new("b").pos("d"),
            Rule::new("c").pos("d"),
        ];
        let strata = stratify(&rules).unwrap();
        assert_eq!(strata["d"], 0);
        assert_eq!(strata["b"], 0);
        assert_eq!(strata["c"], 0);
        assert_eq!(strata["a"], 1);
    }
}
