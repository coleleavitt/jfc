use crate::{ContextSkeletonError, trace};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContributorId(String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextContributor {
    id: ContributorId,
    label: String,
    /// Tokens this contributor occupies in the assembled context. Defaults to
    /// `0` so a contributor can be declared before its contribution is measured
    /// (and so older serialized records without the field still load).
    #[serde(default)]
    tokens: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contributor_trace_records_shape_without_payload_normal() {
        linkscope::trace_detail_enable();
        let id = ContributorId::new("private.contributor.id").expect("valid id");
        let contributor =
            ContextContributor::try_new(id, "Private Contributor Label").expect("valid label");
        let account = ContextAccount::new(vec![contributor.with_tokens(42)]);

        assert_eq!(account.total_tokens(), 42);
        let rendered = format!("{:?}", linkscope::snapshot());
        assert!(rendered.contains("context.contributor_id.new"));
        assert!(rendered.contains("context.contributor.try_new"));
        assert!(rendered.contains("context.account.new"));
        assert!(rendered.contains("label_bytes"));
        assert!(rendered.contains("total_tokens"));
        assert!(!rendered.contains("private.contributor.id"));
        assert!(!rendered.contains("Private Contributor Label"));
    }
}

impl ContributorId {
    pub fn new(id: impl Into<String>) -> Result<Self, ContextSkeletonError> {
        let id = id.into();
        if id.trim().is_empty() {
            trace::record_status("context.contributor_id.new", "empty");
            return Err(ContextSkeletonError::EmptyContributorId);
        }

        trace::record_text_shape(trace::TextShape {
            label: "context.contributor_id.new",
            field: "id_bytes",
            bytes: id.len(),
        });
        Ok(Self(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl ContextContributor {
    pub fn new(id: ContributorId, label: impl Into<String>) -> Self {
        let contributor = Self {
            id,
            label: label.into(),
            tokens: 0,
        };
        trace::record_contributor("context.contributor.new", &contributor);
        contributor
    }

    pub fn try_new(
        id: ContributorId,
        label: impl Into<String>,
    ) -> Result<Self, ContextSkeletonError> {
        let label = label.into();
        if label.trim().is_empty() {
            trace::record_status("context.contributor.try_new", "empty_label");
            return Err(ContextSkeletonError::EmptyContributorLabel);
        }

        let contributor = Self {
            id,
            label,
            tokens: 0,
        };
        trace::record_contributor("context.contributor.try_new", &contributor);
        Ok(contributor)
    }

    /// Attach a measured token contribution (builder style).
    #[must_use]
    pub fn with_tokens(mut self, tokens: u64) -> Self {
        self.tokens = tokens;
        trace::record_contributor("context.contributor.with_tokens", &self);
        self
    }

    pub fn id(&self) -> &ContributorId {
        &self.id
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn tokens(&self) -> u64 {
        self.tokens
    }
}

/// An ordered, token-attributed breakdown of everything occupying the assembled
/// context window. This is the owned source of truth for the context-composition
/// view (System / Docs / Memories / Compartments / Conversation / Tool Calls /
/// Tool Defs ...) — replacing the render layer recomputing estimates inline.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextAccount {
    contributors: Vec<ContextContributor>,
}

impl ContextAccount {
    pub fn new(contributors: Vec<ContextContributor>) -> Self {
        let account = Self { contributors };
        trace::record_account("context.account.new", &account);
        account
    }

    pub fn contributors(&self) -> &[ContextContributor] {
        &self.contributors
    }

    /// Sum of every contributor's token attribution.
    pub fn total_tokens(&self) -> u64 {
        let total = self
            .contributors
            .iter()
            .map(ContextContributor::tokens)
            .sum();
        if linkscope::trace_detail_enabled() {
            linkscope::detail_event_fields(
                "context.account.total_tokens",
                [linkscope::TraceField::count("total_tokens", total)],
            );
        }
        total
    }

    /// `true` when no contributor carries any tokens (nothing measured yet).
    pub fn is_empty(&self) -> bool {
        self.contributors
            .iter()
            .all(|contributor| contributor.tokens() == 0)
    }

    /// Look up a contributor's tokens by id, if present.
    pub fn tokens_for(&self, id: &str) -> Option<u64> {
        self.contributors
            .iter()
            .find(|contributor| contributor.id().as_str() == id)
            .map(ContextContributor::tokens)
    }
}
