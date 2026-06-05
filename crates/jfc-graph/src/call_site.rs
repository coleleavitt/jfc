//! Unresolved call-site captures, fed into the cross-file resolver.
//!
//! Adapters surface call sites *during node extraction* and stash them in
//! `LanguageAdapter::extract_call_sites`. The builder then runs a single
//! [`crate::resolver::ReferenceResolver`] pass over every captured site
//! once all files are indexed — so a call to `foo()` in `src/a.rs` that
//! resolves to a `pub fn foo` in `src/b.rs` is bound correctly. The old
//! per-file path silently dropped cross-file calls because the targets
//! weren't visible in the file's local `name_to_node` map.
//!
//! Path-qualified calls (`mod::sym`, `Type::method`) keep their qualifier
//! segments so the resolver can re-rank candidates: a call to
//! `dispatch::execute_tool` strongly prefers an `execute_tool` whose
//! file path contains a `dispatch` segment.

use std::path::PathBuf;

use crate::nodes::NodeId;

/// How the call expression was written. Affects scoring weights in
/// [`crate::resolver::ReferenceResolver::resolve_one`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallSiteKind {
    /// Bare name: `foo()`.
    Bare,
    /// Qualifier-prefixed: `module::foo()`, `Type::method()`,
    /// `crate::sub::foo()`. The leading segments live in
    /// [`CallSite::path_segments`].
    Qualified,
    /// Receiver-prefixed: `obj.method()`. Receivers aren't tracked yet;
    /// the resolver falls back to bare-name scoring with a function/method
    /// kind bonus.
    MethodCall,
}

/// A single unresolved call expression captured by an adapter.
#[derive(Debug, Clone)]
pub struct CallSite {
    /// The function that contains this call (its enclosing
    /// `function_item`). Always present — the resolver needs it to weight
    /// directory proximity and to emit the Calls edge source.
    pub caller_id: NodeId,
    /// The file the call lives in. Cached so the resolver doesn't have to
    /// re-derive it via `graph.get_node(&caller_id)`.
    pub file_path: PathBuf,
    /// The leaf name of the call — what the resolver looks up. For
    /// `mod::sub::foo()` this is `foo`.
    pub name: String,
    /// Qualifier segments preceding the leaf, in source order. Empty for
    /// bare calls.
    pub path_segments: Vec<String>,
    /// 1-indexed source line of the call expression.
    pub line: u32,
    /// Byte offset of the call expression's start in the file. Carried so the
    /// resolver's `Calls` edge can record the real call-site location — which
    /// `outgoing_call_predicates` walks up from to find the enclosing
    /// `if`/`match`/`while` guard. Without it the edge span defaulted to byte
    /// 0 (file top), so no enclosing predicate was ever found.
    pub byte_offset: usize,
    pub kind: CallSiteKind,
}

impl CallSite {
    /// The name the resolver uses for candidate lookup — always the leaf.
    pub fn name_for_resolution(&self) -> &str {
        &self.name
    }

    /// True when the caller wrote a qualifier (`mod::sym`,
    /// `Type::method`). Resolver gives such sites a strong path-segment
    /// match bonus to disambiguate same-named functions.
    pub fn is_qualified(&self) -> bool {
        !self.path_segments.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::NodeKind;

    fn caller(path: &str) -> NodeId {
        NodeId::new(path, "crate::caller", NodeKind::Function)
    }

    #[test]
    fn bare_site_is_not_qualified() {
        let s = CallSite {
            caller_id: caller("src/a.rs"),
            file_path: PathBuf::from("src/a.rs"),
            name: "foo".into(),
            path_segments: Vec::new(),
            line: 42,
            byte_offset: 0,
            kind: CallSiteKind::Bare,
        };
        assert_eq!(s.name_for_resolution(), "foo");
        assert!(!s.is_qualified());
    }

    #[test]
    fn qualified_site_keeps_segments() {
        let s = CallSite {
            caller_id: caller("src/a.rs"),
            file_path: PathBuf::from("src/a.rs"),
            name: "execute_tool".into(),
            path_segments: vec!["dispatch".into(), "heavy".into()],
            line: 42,
            byte_offset: 0,
            kind: CallSiteKind::Qualified,
        };
        assert_eq!(s.name_for_resolution(), "execute_tool");
        assert!(s.is_qualified());
        assert_eq!(s.path_segments, vec!["dispatch", "heavy"]);
    }
}
