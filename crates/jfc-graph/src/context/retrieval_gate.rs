//! When-to-retrieve gating (Repoformer).
//!
//! From *Repoformer: Selective Retrieval for Repository-Level Code Completion*
//! (arXiv:2403.10059): cross-file/graph retrieval is not free, and for many
//! queries the *local* context already suffices — retrieving then only adds
//! latency and distracting tokens. Repoformer trains the model to **self-assess**
//! whether retrieval will help and abstain when it won't, cutting retrieval
//! latency by up to ~70% at equal or better accuracy.
//!
//! This module is the cheap, deterministic predicate that sits in *front* of
//! [`crate::context::expansion`] / graph expansion: given a handful of signals
//! about the query site, decide whether a graph fetch is worth it. The signals
//! come from the caller (the resolver/IR already knows how many cross-module
//! references and unresolved types are at the cursor); the *decision rule* is
//! here and fully tested. It directly attacks jfc's per-turn-latency budget.

/// Signals about a query/cursor site that predict whether graph retrieval will
/// help. All are cheap to compute from the local IR + symbol table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RetrievalSignal {
    /// Number of references to symbols defined in *other* modules/files.
    pub cross_module_refs: u32,
    /// Number of types at the site the local scope can't resolve.
    pub unresolved_types: u32,
    /// Whether the site references a symbol known to be external (another crate
    /// / a graph node outside the current file).
    pub references_external_symbol: bool,
    /// Whether the local context is self-contained — everything the site needs
    /// is defined in the same file/scope. When set, retrieval is most likely
    /// wasted.
    pub local_self_contained: bool,
}

impl RetrievalSignal {
    /// A self-contained site with no outward references — the canonical
    /// "don't retrieve" case.
    pub fn self_contained() -> Self {
        Self {
            cross_module_refs: 0,
            unresolved_types: 0,
            references_external_symbol: false,
            local_self_contained: true,
        }
    }
}

/// Decide whether to perform graph retrieval for a site with these signals.
///
/// Rule (precision-favouring, matching Repoformer's abstain-by-default spirit
/// when local context is enough):
/// 1. If anything is *unresolved* (`unresolved_types > 0`) → **retrieve**;
///    the model demonstrably lacks a definition it needs.
/// 2. Else if the site reaches outside its file (`cross_module_refs > 0` or
///    `references_external_symbol`) → **retrieve**.
/// 3. Else → **abstain**. In particular an explicitly `local_self_contained`
///    site with no outward signals never retrieves.
///
/// Unresolved types win even if `local_self_contained` was optimistically set,
/// because an unresolved type is hard evidence the local view lacks a needed
/// definition.
pub fn should_retrieve(signal: &RetrievalSignal) -> bool {
    if signal.unresolved_types > 0 {
        return true;
    }
    if signal.cross_module_refs > 0 || signal.references_external_symbol {
        return true;
    }
    false
}

/// Inverse of [`should_retrieve`], named for call sites that read better as
/// "can we skip the graph fetch?".
pub fn can_skip_retrieval(signal: &RetrievalSignal) -> bool {
    !should_retrieve(signal)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Normal: a fully self-contained site skips retrieval.
    #[test]
    fn self_contained_skips_retrieval_normal() {
        let s = RetrievalSignal::self_contained();
        assert!(!should_retrieve(&s));
        assert!(can_skip_retrieval(&s));
    }

    // Normal: a cross-module reference triggers retrieval.
    #[test]
    fn cross_module_triggers_retrieval_normal() {
        let s = RetrievalSignal {
            cross_module_refs: 2,
            ..Default::default()
        };
        assert!(should_retrieve(&s));
    }

    // Normal: an unresolved type triggers retrieval.
    #[test]
    fn unresolved_type_triggers_retrieval_normal() {
        let s = RetrievalSignal {
            unresolved_types: 1,
            ..Default::default()
        };
        assert!(should_retrieve(&s));
    }

    // Robust: unresolved types override an optimistic self-contained flag.
    #[test]
    fn unresolved_overrides_self_contained_robust() {
        let s = RetrievalSignal {
            unresolved_types: 1,
            local_self_contained: true,
            ..Default::default()
        };
        assert!(should_retrieve(&s));
    }

    // Robust: an external-symbol reference alone triggers retrieval.
    #[test]
    fn external_symbol_triggers_retrieval_robust() {
        let s = RetrievalSignal {
            references_external_symbol: true,
            ..Default::default()
        };
        assert!(should_retrieve(&s));
    }

    // Robust: the all-zero default abstains (no signal to justify the cost).
    #[test]
    fn default_signal_abstains_robust() {
        assert!(!should_retrieve(&RetrievalSignal::default()));
    }
}
