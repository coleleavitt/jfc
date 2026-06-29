use serde::{Deserialize, Serialize};

use crate::error::{ChangeSetError, Result};

fn bool_to_u64(value: bool) -> u64 {
    u64::from(value)
}

/// Lifecycle state of an agent change proposal.
///
/// Mirrors Dolt's "isolated branch → reviewed → tested → merged" promise as an
/// explicit state machine. The ordering encodes the safety invariant: a change
/// cannot reach [`ChangeState::Applied`] (touch the main checkout) without
/// first passing [`ChangeState::Tested`] and [`ChangeState::Approved`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChangeState {
    /// The worktree/branch exists and the agent is (or was) mutating it.
    Draft,
    /// The agent finished; a diff exists and is ready for review/test.
    Ready,
    /// Attached test commands ran against the branch (results recorded).
    Tested,
    /// A human (or validator quorum) approved the tested change.
    Approved,
    /// The branch was merged into the base — production now reflects it.
    Applied,
    /// A previously-applied change was undone (git revert/reset of the merge).
    Reverted,
    /// The change was discarded without applying (worktree may be preserved).
    Abandoned,
}

impl ChangeState {
    /// Human-readable label for logs and the `jfc changes` table.
    pub fn label(self) -> &'static str {
        linkscope::record_items("changeset.state.label", 1);
        let label = match self {
            Self::Draft => "draft",
            Self::Ready => "ready",
            Self::Tested => "tested",
            Self::Approved => "approved",
            Self::Applied => "applied",
            Self::Reverted => "reverted",
            Self::Abandoned => "abandoned",
        };
        linkscope::detail_event_fields(
            "changeset.state.label.result",
            [linkscope::TraceField::text("state", label)],
        );
        label
    }

    /// A terminal state has no outgoing transitions — the change-set is done.
    pub fn is_terminal(self) -> bool {
        let terminal = matches!(self, Self::Reverted | Self::Abandoned);
        linkscope::detail_event_fields(
            "changeset.state.terminal",
            [
                linkscope::TraceField::text("state", self.label()),
                linkscope::TraceField::count("terminal", bool_to_u64(terminal)),
            ],
        );
        terminal
    }

    /// Whether `self -> next` is a legal lifecycle transition.
    ///
    /// The happy path is `Draft → Ready → Tested → Approved → Applied`. Any
    /// state before `Applied` may be `Abandoned`. Only `Applied` may be
    /// `Reverted`. Crucially, `Ready`/`Tested` cannot jump straight to
    /// `Applied`: review and a passing test run are mandatory waypoints.
    pub fn can_transition_to(self, next: ChangeState) -> bool {
        use ChangeState::*;
        let allowed = match (self, next) {
            (Draft, Ready)
            | (Ready, Tested)
            | (Tested, Approved)
            | (Approved, Applied)
            | (Applied, Reverted) => true,
            // Any non-terminal, not-yet-applied state can be abandoned.
            (Draft | Ready | Tested | Approved, Abandoned) => true,
            _ => false,
        };
        linkscope::detail_event_fields(
            "changeset.state.transition.check",
            [
                linkscope::TraceField::text("from", self.label()),
                linkscope::TraceField::text("to", next.label()),
                linkscope::TraceField::count("allowed", bool_to_u64(allowed)),
            ],
        );
        allowed
    }

    /// Validate a transition, returning a descriptive error when illegal.
    pub fn ensure_transition(self, next: ChangeState) -> Result<()> {
        let _linkscope_transition = linkscope::phase("changeset.state.transition.ensure");
        if self.can_transition_to(next) {
            linkscope::record_items("changeset.state.transition.ok", 1);
            return Ok(());
        }
        let reason = if self.is_terminal() {
            "source state is terminal"
        } else if next == ChangeState::Applied {
            "Applied requires passing through Tested then Approved"
        } else {
            "not a permitted lifecycle edge"
        };
        linkscope::event_fields(
            "changeset.state.transition.rejected",
            [
                linkscope::TraceField::text("from", self.label()),
                linkscope::TraceField::text("to", next.label()),
                linkscope::TraceField::text("reason", reason),
            ],
        );
        Err(ChangeSetError::IllegalTransition {
            from: self,
            to: next,
            reason: reason.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::ChangeState::*;
    use super::*;

    // Normal: the full happy-path chain is legal end to end.
    #[test]
    fn happy_path_chain_is_legal_normal() {
        let chain = [Draft, Ready, Tested, Approved, Applied, Reverted];
        for pair in chain.windows(2) {
            assert!(
                pair[0].can_transition_to(pair[1]),
                "{:?} -> {:?} should be legal",
                pair[0],
                pair[1]
            );
            pair[0].ensure_transition(pair[1]).expect("legal edge");
        }
    }

    // Robust — the load-bearing safety guard: a Ready (un-tested,
    // un-approved) change must NOT be appliable directly to production.
    #[test]
    fn ready_cannot_skip_to_applied_robust() {
        assert!(!Ready.can_transition_to(Applied));
        let err = Ready.ensure_transition(Applied).unwrap_err();
        assert!(matches!(
            err,
            ChangeSetError::IllegalTransition {
                from: Ready,
                to: Applied,
                ..
            }
        ));
    }

    // Robust: a Tested-but-unapproved change still cannot apply.
    #[test]
    fn tested_without_approval_cannot_apply_robust() {
        assert!(!Tested.can_transition_to(Applied));
    }

    const ALL_STATES: [ChangeState; 7] =
        [Draft, Ready, Tested, Approved, Applied, Reverted, Abandoned];

    /// Count legal outgoing edges from `s` without branching in a test body.
    fn outgoing_edge_count(s: ChangeState) -> usize {
        ALL_STATES
            .iter()
            .filter(|&&next| s.can_transition_to(next))
            .count()
    }

    // Robust: the Reverted terminal state has zero outgoing edges.
    #[test]
    fn reverted_is_a_dead_end_robust() {
        assert!(Reverted.is_terminal());
        assert_eq!(outgoing_edge_count(Reverted), 0);
    }

    // Robust: the Abandoned terminal state has zero outgoing edges.
    #[test]
    fn abandoned_is_a_dead_end_robust() {
        assert!(Abandoned.is_terminal());
        assert_eq!(outgoing_edge_count(Abandoned), 0);
    }

    // Robust: only an Applied change can be reverted (you can't revert a
    // change that never touched production).
    #[test]
    fn only_applied_can_revert_robust() {
        for s in [Draft, Ready, Tested, Approved] {
            assert!(!s.can_transition_to(Reverted), "{s:?} -> Reverted illegal");
        }
        assert!(Applied.can_transition_to(Reverted));
    }

    // Robust: every pre-apply state can be abandoned; Applied cannot (it must
    // be reverted instead).
    #[test]
    fn abandon_allowed_before_apply_only_robust() {
        for s in [Draft, Ready, Tested, Approved] {
            assert!(s.can_transition_to(Abandoned), "{s:?} -> Abandoned legal");
        }
        assert!(!Applied.can_transition_to(Abandoned));
    }
}
