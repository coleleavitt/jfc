use serde::Serialize;

use super::{CouncilSession, KickOutcome, TranscriptEntry};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum PendingCouncilAction {
    Challenge {
        from_id: String,
        target_id: String,
        question: String,
    },
    ModelDm {
        from_id: String,
        target_id: String,
        opening: String,
    },
    KickVote {
        requested_by: String,
        target_id: String,
    },
    OperatorVote {
        requested_by: String,
        reason: String,
    },
}

impl CouncilSession {
    pub fn pending_action(&self) -> Option<&PendingCouncilAction> {
        self.pending_action.as_ref()
    }

    pub fn deny_pending_action(&mut self) -> Option<String> {
        let action = self.pending_action.take()?;
        Some(format!(
            "Pending action denied: {}.",
            self.pending_action_label(&action)
        ))
    }

    pub(super) fn stage_pending_action(
        &mut self,
        action: PendingCouncilAction,
        notes: &mut Vec<String>,
    ) -> bool {
        if self.pending_action.is_some() {
            notes.push(
                "Another council action is already awaiting operator approval; new request ignored. Use `/council approve` or `/council deny` first."
                    .to_owned(),
            );
            return false;
        }
        let label = self.pending_action_label(&action);
        self.pending_action = Some(action);
        notes.push(format!(
            "{label} awaiting operator approval. Use `/council approve` or `/council deny`."
        ));
        true
    }

    pub async fn approve_pending_action(&mut self) -> anyhow::Result<Vec<String>> {
        let Some(action) = self.pending_action.take() else {
            return Ok(vec!["No pending council action to approve.".to_owned()]);
        };
        match action {
            PendingCouncilAction::Challenge {
                from_id,
                target_id,
                question,
            } => Ok(vec![
                self.approve_challenge(&from_id, &target_id, &question),
            ]),
            PendingCouncilAction::ModelDm {
                from_id,
                target_id,
                opening,
            } => self.approve_model_dm(&from_id, &target_id, opening).await,
            PendingCouncilAction::KickVote {
                requested_by,
                target_id,
            } => self.approve_kick_vote(&requested_by, &target_id).await,
            PendingCouncilAction::OperatorVote {
                requested_by,
                reason,
            } => self.approve_operator_vote(&requested_by, &reason).await,
        }
    }

    fn approve_challenge(&mut self, from_id: &str, target_id: &str, question: &str) -> String {
        let from_name = self.seat_name(from_id);
        let target_name = self.seat_name(target_id);
        self.transcript.push(TranscriptEntry::system(
            self.round,
            format!("CHALLENGE — {from_name} to {target_name}: {question}"),
        ));
        self.call_next(target_id);
        format!("Challenge approved; {target_name} answers next with `/council continue`.")
    }

    async fn approve_model_dm(
        &mut self,
        from_id: &str,
        target_id: &str,
        opening: String,
    ) -> anyhow::Result<Vec<String>> {
        match self.seat(from_id).map(|s| s.asides_remaining) {
            Some(0) | None => Ok(vec![
                "Aside approval failed — no aside allowance remaining.".to_owned(),
            ]),
            Some(_) => {
                if let Some(seat) = self.seat_mut(from_id) {
                    seat.asides_remaining = seat.asides_remaining.saturating_sub(1);
                }
                self.run_side_conversation(from_id, target_id, Some(opening), None)
                    .await?;
                Ok(vec![
                    "Aside approved and completed (sealed from the table).".to_owned(),
                ])
            }
        }
    }

    async fn approve_kick_vote(
        &mut self,
        requested_by: &str,
        target_id: &str,
    ) -> anyhow::Result<Vec<String>> {
        let requester = self.seat_name(requested_by);
        let outcome = self.run_kick_vote(target_id).await?;
        Ok(vec![format_vote_note("Kick vote", &requester, outcome)])
    }

    async fn approve_operator_vote(
        &mut self,
        requested_by: &str,
        reason: &str,
    ) -> anyhow::Result<Vec<String>> {
        if self.round < 2 || self.operator_muted {
            return Ok(vec!["Operator vote ignored (unavailable now).".to_owned()]);
        }
        let requester = self.seat_name(requested_by);
        let outcome = self.run_operator_vote().await?;
        Ok(vec![format!(
            "{} Reason: {reason}",
            format_vote_note("Operator vote", &requester, outcome)
        )])
    }

    pub(super) fn pending_action_label(&self, action: &PendingCouncilAction) -> String {
        match action {
            PendingCouncilAction::Challenge {
                from_id, target_id, ..
            } => format!(
                "Challenge from {} to {}",
                self.seat_name(from_id),
                self.seat_name(target_id)
            ),
            PendingCouncilAction::ModelDm {
                from_id, target_id, ..
            } => format!(
                "Private aside from {} to {}",
                self.seat_name(from_id),
                self.seat_name(target_id)
            ),
            PendingCouncilAction::KickVote {
                requested_by,
                target_id,
            } => format!(
                "Kick vote requested by {} against {}",
                self.seat_name(requested_by),
                self.seat_name(target_id)
            ),
            PendingCouncilAction::OperatorVote { requested_by, .. } => {
                format!(
                    "Operator-mute vote requested by {}",
                    self.seat_name(requested_by)
                )
            }
        }
    }

    pub(super) fn seat_name(&self, seat_id: &str) -> String {
        self.seat(seat_id)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| seat_id.to_owned())
    }
}

fn format_vote_note(kind: &str, requester: &str, outcome: KickOutcome) -> String {
    let result = if outcome.passed { "passed" } else { "failed" };
    format!(
        "{kind} approved for {requester}; {result} ({} / {}).",
        outcome.kick, outcome.keep
    )
}
