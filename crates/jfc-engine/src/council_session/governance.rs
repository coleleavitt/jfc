//! Participant governance + private asides for [`CouncilSession`].
//!
//! Covers the RoundTable mechanics that aren't the main turn loop:
//! sealed model↔model side-conversations (and operator↔model asides), kick
//! votes, operator-mute votes, voluntary departure, and the flagged-claim
//! lifecycle. Asides are *sealed*: only the two participants ever see them in
//! future context (enforced in [`super::turns::CouncilSession::build_context_for_seat`]
//! via [`CouncilSession::aside_recall_for`]).

use jfc_provider::{ProviderContent, ProviderMessage, ProviderRole, StreamOptions};
use serde::Serialize;

use super::{CouncilSession, PendingCouncilAction, TranscriptEntry};
use crate::council_directives::Directive;

/// Hard ceiling on messages in one sealed aside (matches the web client).
pub const SIDE_CONVO_HARD_CAP: usize = 12;
/// Default per-aside message cap (clamped to [`SIDE_CONVO_HARD_CAP`]).
pub const DEFAULT_SIDE_CONVO_MAX_LEN: usize = 6;

/// One line in a sealed side-conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SideConvoTurn {
    /// Seat id, or `"operator"` for an operator↔model aside.
    pub from: String,
    pub content: String,
}

/// A sealed one-to-one aside. Visible only to its two participants (and the
/// operator) — never woven into any other seat's context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SideConvo {
    pub id: String,
    pub a: String,
    pub b: String,
    pub round: u32,
    pub turns: Vec<SideConvoTurn>,
    pub closed: bool,
    /// Operator directed the two seats to confer (bypasses per-seat allowance).
    pub operator_directed: bool,
}

impl SideConvo {
    pub fn involves(&self, seat_id: &str) -> bool {
        self.a == seat_id || self.b == seat_id
    }
}

/// Lifecycle status of a flagged factual claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FlagStatus {
    Open,
    Verified,
    False,
    Corrected,
    Dismissed,
}

impl FlagStatus {
    pub fn is_open(self) -> bool {
        matches!(self, Self::Open)
    }
}

/// A load-bearing factual claim flagged for verification. Open flags soft-block
/// the verdict until the operator resolves them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FlaggedClaim {
    pub id: String,
    pub claim: String,
    /// Seat id, or `"operator"`.
    pub flagged_by: String,
    /// Seat whose claim it is (the target), if known.
    pub target: Option<String>,
    pub round: u32,
    pub status: FlagStatus,
    pub correction: Option<String>,
}

/// Outcome of a kick or operator-mute vote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KickOutcome {
    pub passed: bool,
    pub kick: usize,
    pub keep: usize,
}

impl CouncilSession {
    // ── Flagged claims ──────────────────────────────────────────────────────

    /// Flag a claim. Dedupes identical (target, flagger, claim) flags.
    pub fn add_flag(
        &mut self,
        claim: impl Into<String>,
        flagged_by: impl Into<String>,
        target: Option<String>,
    ) -> String {
        let claim = claim.into();
        let flagged_by = flagged_by.into();
        if let Some(existing) = self.flagged_claims.iter().find(|f| {
            f.flagged_by == flagged_by && f.claim.eq_ignore_ascii_case(&claim) && f.target == target
        }) {
            return existing.id.clone();
        }
        let id = format!("flag-{}", self.flagged_claims.len() + 1);
        self.flagged_claims.push(FlaggedClaim {
            id: id.clone(),
            claim,
            flagged_by,
            target,
            round: self.round,
            status: FlagStatus::Open,
            correction: None,
        });
        id
    }

    pub fn resolve_flag(&mut self, flag_id: &str, status: FlagStatus, correction: Option<String>) {
        if let Some(flag) = self.flagged_claims.iter_mut().find(|f| f.id == flag_id) {
            flag.status = status;
            flag.correction = correction.filter(|_| status == FlagStatus::Corrected);
        }
    }

    pub fn open_flags(&self) -> impl Iterator<Item = &FlaggedClaim> {
        self.flagged_claims.iter().filter(|f| f.status.is_open())
    }

    pub fn open_flag_count(&self) -> usize {
        self.open_flags().count()
    }

    // ── Sealed asides ─────────────────────────────────────────────────────────

    /// The sealed-aside recall block for `seat_id`: every aside this seat was a
    /// participant in, rendered for its private context. `None` when it has none.
    pub fn aside_recall_for(&self, seat_id: &str) -> Option<String> {
        let mine: Vec<&SideConvo> = self
            .side_convos
            .iter()
            .filter(|c| c.involves(seat_id))
            .collect();
        if mine.is_empty() {
            return None;
        }
        let mut out = String::from(
            "\n--- Your private side-conversations so far (sealed; the rest of the table did not see these) ---\n",
        );
        for convo in mine {
            let other_id = if convo.a == seat_id {
                &convo.b
            } else {
                &convo.a
            };
            let other = if other_id == "operator" {
                "the operator".to_owned()
            } else {
                self.seat(other_id)
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| other_id.clone())
            };
            out.push_str(&format!("(Aside with {other}:)\n"));
            for turn in &convo.turns {
                let who = if turn.from == seat_id {
                    "You"
                } else if turn.from == "operator" {
                    "Operator"
                } else {
                    other.as_str()
                };
                out.push_str(&format!("{who}: {}\n", turn.content));
            }
        }
        out.push_str("--- end private asides ---\n");
        Some(out)
    }

    /// Run a sealed model↔model aside until both have settled (END DM) or the
    /// length cap. Returns the new convo id. `operator_brief` (if any) seeds an
    /// operator-directed aside and bypasses the per-seat allowance.
    pub async fn run_side_conversation(
        &mut self,
        from_id: &str,
        to_id: &str,
        opening: Option<String>,
        operator_brief: Option<String>,
    ) -> anyhow::Result<String> {
        let cap = self.side_convo_cap();
        let convo_id = format!("dm-{}", self.side_convos.len() + 1);
        let mut convo = SideConvo {
            id: convo_id.clone(),
            a: from_id.to_owned(),
            b: to_id.to_owned(),
            round: self.round,
            turns: Vec::new(),
            closed: false,
            operator_directed: operator_brief.is_some(),
        };
        if let Some(open) = opening.filter(|_| operator_brief.is_none()) {
            convo.turns.push(SideConvoTurn {
                from: from_id.to_owned(),
                content: open,
            });
        }
        // Operator-directed: A speaks first; otherwise B replies to A's opener.
        let (mut current, mut partner) = if operator_brief.is_some() {
            (from_id.to_owned(), to_id.to_owned())
        } else {
            (to_id.to_owned(), from_id.to_owned())
        };

        // Hard ceiling on provider calls: even a model that only ever replies
        // "END DM" (so `clean` is empty and `turns.len()` never grows) must
        // terminate. Each side gets at most `cap` attempts.
        let max_replies = cap.saturating_mul(2).max(2);
        let mut replies = 0usize;
        loop {
            let content = self
                .aside_reply(&current, &partner, &convo, operator_brief.as_deref())
                .await?;
            replies += 1;
            let closing = content.contains("END DM");
            let clean = content.replace("END DM", "").trim().to_owned();
            if !clean.is_empty() {
                convo.turns.push(SideConvoTurn {
                    from: current.clone(),
                    content: clean,
                });
            }
            let enough = convo.turns.len() >= 3;
            if closing && enough {
                break;
            }
            if convo.turns.len() >= cap || replies >= max_replies {
                break;
            }
            std::mem::swap(&mut current, &mut partner);
        }
        convo.closed = true;
        self.side_convos.push(convo);
        Ok(convo_id)
    }

    fn side_convo_cap(&self) -> usize {
        DEFAULT_SIDE_CONVO_MAX_LEN.clamp(4, SIDE_CONVO_HARD_CAP)
    }

    /// Orchestrate the governance directives a seat emitted on its turn
    /// (challenge, model→model DM, kick/operator votes). Self-contained
    /// directives (FLAG CLAIM / ASSIGN / LEAVE TABLE) are already applied in the
    /// turn path. Returns operator-facing notes describing what happened. Awaits
    /// model calls for asides and votes, so this is async.
    ///
    /// Aside allowance (Bug fix): a model-initiated DM to another seat consumes
    /// one of the speaker's `asides_remaining`; when none remain the request is
    /// declined with a note rather than silently running.
    pub async fn process_turn_directives(
        &mut self,
        speaker_id: &str,
        directives: Vec<Directive>,
    ) -> Vec<String> {
        let mut notes = Vec::new();
        for directive in directives {
            match directive {
                Directive::Challenge { target, question } => {
                    self.stage_challenge(speaker_id, &target, question, &mut notes);
                }
                Directive::Dm { target, opening } => {
                    self.stage_model_dm(speaker_id, &target, opening, &mut notes);
                }
                Directive::KickVote { target } => {
                    self.stage_kick_directive(speaker_id, &target, &mut notes);
                }
                Directive::OperatorVote { reason } => {
                    if self.round >= 2 && !self.operator_muted {
                        self.stage_pending_action(
                            PendingCouncilAction::OperatorVote {
                                requested_by: speaker_id.to_owned(),
                                reason,
                            },
                            &mut notes,
                        );
                    } else {
                        notes.push("Operator vote ignored (unlocks after round 2).".to_owned());
                    }
                }
                Directive::GenerateImage {
                    prompt,
                    aspect_ratio,
                } => {
                    let ratio = aspect_ratio.map(|r| format!(" ({r})")).unwrap_or_default();
                    notes.push(format!(
                        "Image requested but terminal council cannot generate images yet: {prompt}{ratio}."
                    ));
                    self.transcript.push(TranscriptEntry::system(
                        self.round,
                        format!("IMAGE REQUEST — {prompt}{ratio}\n\nTerminal council parsed the request; image generation requires an image-capable provider in this surface."),
                    ));
                }
                // FLAG CLAIM / ASSIGN / LEAVE TABLE handled in the turn path;
                // STANCE / PASS / GENERATE IMAGE are non-orchestrated here.
                _ => {}
            }
        }
        notes
    }

    fn stage_challenge(
        &mut self,
        from_id: &str,
        target: &str,
        question: String,
        notes: &mut Vec<String>,
    ) {
        if self.seat(from_id).is_some_and(|s| s.challenge_used) {
            notes.push("Challenge ignored — one per debate already used.".to_owned());
            return;
        }
        let Some(target_id) = self
            .resolve_seat(target, Some(from_id))
            .map(|s| s.id.clone())
        else {
            notes.push(format!("Challenge target '{target}' not found."));
            return;
        };
        if let Some(seat) = self.seat_mut(from_id) {
            seat.challenge_used = true;
        }
        self.stage_pending_action(
            PendingCouncilAction::Challenge {
                from_id: from_id.to_owned(),
                target_id,
                question,
            },
            notes,
        );
    }

    fn stage_model_dm(
        &mut self,
        from_id: &str,
        target: &str,
        opening: String,
        notes: &mut Vec<String>,
    ) {
        if Directive::is_operator_dm(target) {
            // A private word to the operator surfaces as a note rather than a
            // sealed model↔model aside.
            let from_name = self
                .seat(from_id)
                .map(|s| s.name.clone())
                .unwrap_or_default();
            notes.push(format!("{from_name} → operator (private): {opening}"));
            return;
        }
        let Some(target_id) = self
            .resolve_seat(target, Some(from_id))
            .map(|s| s.id.clone())
        else {
            notes.push(format!("Aside target '{target}' not found."));
            return;
        };
        match self.seat(from_id).map(|s| s.asides_remaining) {
            Some(0) | None => {
                notes.push("Aside request declined — no aside allowance remaining.".to_owned());
                return;
            }
            Some(_) => {}
        }
        self.stage_pending_action(
            PendingCouncilAction::ModelDm {
                from_id: from_id.to_owned(),
                target_id,
                opening,
            },
            notes,
        );
    }

    fn stage_kick_directive(&mut self, speaker_id: &str, target: &str, notes: &mut Vec<String>) {
        let Some(target_id) = self.resolve_seat(target, None).map(|s| s.id.clone()) else {
            notes.push(format!("Kick target '{target}' not found."));
            return;
        };
        self.stage_pending_action(
            PendingCouncilAction::KickVote {
                requested_by: speaker_id.to_owned(),
                target_id,
            },
            notes,
        );
    }

    async fn aside_reply(
        &self,
        speaker_id: &str,
        partner_id: &str,
        convo: &SideConvo,
        brief: Option<&str>,
    ) -> anyhow::Result<String> {
        let speaker = self
            .seat(speaker_id)
            .ok_or_else(|| anyhow::anyhow!("aside speaker {speaker_id} unknown"))?;
        let partner_name = if partner_id == "operator" {
            "the operator".to_owned()
        } else {
            self.seat(partner_id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| partner_id.to_owned())
        };
        let system = format!(
            "You are {me}, in a sealed one-to-one aside with {partner} during a council on: {topic}. The rest of the table cannot see this. Coordinate honestly and briefly. When the two of you have genuinely reached a conclusion, end your message with a line: END DM. Do not end on a question.{brief}",
            me = speaker.name,
            partner = partner_name,
            topic = self.topic,
            brief = brief
                .map(|b| format!(" The operator asked you to discuss: {b}"))
                .unwrap_or_default(),
        );
        let mut messages = Vec::new();
        for turn in &convo.turns {
            let role = if turn.from == speaker_id {
                ProviderRole::Assistant
            } else {
                ProviderRole::User
            };
            messages.push(ProviderMessage {
                role,
                content: vec![ProviderContent::Text(turn.content.clone())],
            });
        }
        if messages.is_empty()
            || matches!(messages.last(), Some(m) if m.role == ProviderRole::Assistant)
        {
            messages.push(ProviderMessage {
                role: ProviderRole::User,
                content: vec![ProviderContent::Text(
                    brief
                        .map(str::to_owned)
                        .unwrap_or_else(|| "Begin the aside.".to_owned()),
                )],
            });
        }
        let opts = StreamOptions::new(speaker.model.clone())
            .system(system)
            .max_tokens(512);
        let resp =
            crate::prompt_executor::complete_once(speaker.provider.as_ref(), messages, &opts)
                .await?;
        Ok(resp.content)
    }

    // ── Departure / mute ────────────────────────────────────────────────────

    /// A seat voluntarily leaves the table (round 2+). Records a system note.
    pub fn leave_table(&mut self, seat_id: &str, reason: &str) {
        if let Some(seat) = self.seat_mut(seat_id) {
            seat.has_left = true;
        }
        let name = self
            .seat(seat_id)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| seat_id.to_owned());
        let note = if reason.trim().is_empty() {
            format!("{name} left the table.")
        } else {
            format!("{name} left the table — {reason}")
        };
        self.transcript
            .push(TranscriptEntry::system(self.round, note));
        self.turn_queue.retain(|id| id != seat_id);
    }

    /// Lift a council-imposed operator mute.
    pub fn unmute_operator(&mut self) {
        self.operator_muted = false;
    }

    // ── Votes ─────────────────────────────────────────────────────────────────

    /// Poll active seats (excluding `target_id`) for KICK/KEEP. Majority kicks.
    pub async fn run_kick_vote(&mut self, target_id: &str) -> anyhow::Result<KickOutcome> {
        let voters: Vec<String> = self
            .active_seats()
            .filter(|s| s.id != target_id)
            .map(|s| s.id.clone())
            .collect();
        if voters.len() < 2 {
            return Err(anyhow::anyhow!("need at least 2 voters for a kick vote"));
        }
        let target_name = self
            .seat(target_id)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| target_id.to_owned());
        let prompt = format!(
            "A kick vote has been called: should {target_name} be removed from the council? Reply EXACTLY two lines:\nVOTE: KICK or KEEP\nREASON: <one sentence>"
        );
        let mut kick = 0usize;
        for voter in &voters {
            let reply = self.poll_seat(voter, &prompt).await.unwrap_or_default();
            if vote_says(&reply, "KICK", &["kick", "remove", "eject"]) {
                kick += 1;
            }
        }
        let keep = voters.len() - kick;
        let passed = kick > keep;
        if passed {
            if let Some(seat) = self.seat_mut(target_id) {
                seat.kicked = true;
            }
            self.turn_queue.retain(|id| id != target_id);
        }
        let headline = if passed {
            format!("KICK VOTE PASSED — {target_name} removed · {kick} kick / {keep} keep")
        } else {
            format!("KICK VOTE FAILED — {target_name} stays · {kick} kick / {keep} keep")
        };
        self.transcript
            .push(TranscriptEntry::system(self.round, headline));
        Ok(KickOutcome { passed, kick, keep })
    }

    /// Poll active seats on muting the operator. Majority mutes.
    pub async fn run_operator_vote(&mut self) -> anyhow::Result<KickOutcome> {
        let voters: Vec<String> = self.active_seats().map(|s| s.id.clone()).collect();
        if voters.len() < 2 {
            return Err(anyhow::anyhow!(
                "need at least 2 participants for an operator vote"
            ));
        }
        let prompt = "A motion to MUTE the operator has been raised. Reply EXACTLY two lines:\nVOTE: MUTE or KEEP\nREASON: <one sentence>";
        let mut mute = 0usize;
        for voter in &voters {
            let reply = self.poll_seat(voter, prompt).await.unwrap_or_default();
            if vote_says(&reply, "MUTE", &["mute"]) {
                mute += 1;
            }
        }
        let keep = voters.len() - mute;
        let passed = mute > keep;
        if passed {
            self.operator_muted = true;
        }
        let headline = if passed {
            format!("OPERATOR VOTE PASSED — operator muted · {mute} mute / {keep} keep")
        } else {
            format!("OPERATOR VOTE FAILED — operator retained · {mute} mute / {keep} keep")
        };
        self.transcript
            .push(TranscriptEntry::system(self.round, headline));
        Ok(KickOutcome {
            passed,
            kick: mute,
            keep,
        })
    }

    /// One private poll completion for `seat_id` with `prompt` appended to its
    /// full context. Shared by consensus, verdict, and votes.
    pub(super) async fn poll_seat(&self, seat_id: &str, prompt: &str) -> anyhow::Result<String> {
        let seat = self
            .seat(seat_id)
            .ok_or_else(|| anyhow::anyhow!("unknown seat {seat_id}"))?;
        let (system, mut messages, _blind) = self.build_context_for_seat(seat_id);
        messages.push(ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(prompt.to_owned())],
        });
        let opts = StreamOptions::new(seat.model.clone())
            .system(system)
            .max_tokens(512);
        let resp =
            crate::prompt_executor::complete_once(seat.provider.as_ref(), messages, &opts).await?;
        Ok(resp.content)
    }
}

/// True when a ballot reply chooses `_canonical` per any of `keywords`. The
/// negative ("KEEP") is the implicit default when no keyword matches.
fn vote_says(reply: &str, _canonical: &str, keywords: &[&str]) -> bool {
    let upper = reply.to_ascii_uppercase();
    if let Some(line) = upper.lines().find(|l| l.trim_start().starts_with("VOTE")) {
        return keywords
            .iter()
            .any(|k| line.contains(&k.to_ascii_uppercase()));
    }
    keywords
        .iter()
        .any(|k| upper.contains(&k.to_ascii_uppercase()))
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::super::*;
    use super::*;

    #[test]
    fn flag_dedup_and_open_count_normal() {
        let mut s = CouncilSession::new(
            "Q?",
            CouncilSessionMode::Debate,
            vec![seat("a", "Alpha", "x"), seat("b", "Beta", "y")],
        );
        let id1 = s.add_flag("benchmark was 9s", "a", Some("b".into()));
        let id2 = s.add_flag("benchmark was 9s", "a", Some("b".into()));
        assert_eq!(id1, id2, "identical flag deduped");
        assert_eq!(s.open_flag_count(), 1);
        s.resolve_flag(&id1, FlagStatus::Verified, None);
        assert_eq!(s.open_flag_count(), 0);
    }

    #[test]
    fn aside_recall_sealed_to_participants_robust() {
        let mut s = CouncilSession::new(
            "Q?",
            CouncilSessionMode::Debate,
            vec![
                seat("a", "Alpha", "x"),
                seat("b", "Beta", "y"),
                seat("c", "Gamma", "z"),
            ],
        );
        s.side_convos.push(SideConvo {
            id: "dm-1".into(),
            a: "a".into(),
            b: "b".into(),
            round: 1,
            turns: vec![SideConvoTurn {
                from: "a".into(),
                content: "let's align".into(),
            }],
            closed: true,
            operator_directed: false,
        });
        assert!(s.aside_recall_for("a").is_some());
        assert!(s.aside_recall_for("b").is_some());
        assert!(
            s.aside_recall_for("c").is_none(),
            "non-participant gets no recall"
        );
    }

    #[tokio::test]
    async fn kick_vote_majority_robust() {
        let mut s = CouncilSession::new(
            "Q?",
            CouncilSessionMode::Debate,
            vec![
                seat_seq("a", "Alpha", vec!["VOTE: KICK\nREASON: disruptive"]),
                seat_seq("b", "Beta", vec!["VOTE: KICK\nREASON: agreed"]),
                seat("c", "Gamma", "ignored"),
            ],
        );
        s.start();
        let outcome = s.run_kick_vote("c").await.unwrap();
        assert!(outcome.passed);
        assert_eq!(outcome.kick, 2);
        assert!(s.seat("c").unwrap().kicked);
    }

    #[tokio::test]
    async fn aside_terminates_on_end_dm_only_robust() {
        // A model that ONLY ever replies "END DM" (empty after strip) must not
        // loop forever — the reply-count guard terminates it.
        let mut s = CouncilSession::new(
            "Q?",
            CouncilSessionMode::Debate,
            vec![
                seat_seq("a", "Alpha", vec!["END DM"]),
                seat_seq("b", "Beta", vec!["END DM"]),
            ],
        );
        s.start();
        let id = s
            .run_side_conversation("a", "b", Some("hi".into()), None)
            .await
            .unwrap();
        assert!(s.side_convos.iter().any(|c| c.id == id && c.closed));
    }

    #[tokio::test]
    async fn model_dm_spends_allowance_only_after_approval_robust() {
        let mut s = CouncilSession::new(
            "Q?",
            CouncilSessionMode::Debate,
            vec![
                seat_seq("a", "Alpha", vec!["END DM"]),
                seat_seq("b", "Beta", vec!["END DM"]),
            ],
        );
        s.start();
        assert_eq!(s.seat("a").unwrap().asides_remaining, 1);
        let notes = s
            .process_turn_directives(
                "a",
                vec![Directive::Dm {
                    target: "Beta".into(),
                    opening: "let's align".into(),
                }],
            )
            .await;
        assert!(notes.iter().any(|n| n.contains("approval")));
        assert!(s.pending_action().is_some());
        assert_eq!(s.seat("a").unwrap().asides_remaining, 1);

        s.approve_pending_action().await.unwrap();
        assert_eq!(s.seat("a").unwrap().asides_remaining, 0);
        let before = s.side_convos.len();
        let notes = s
            .process_turn_directives(
                "a",
                vec![Directive::Dm {
                    target: "Beta".into(),
                    opening: "again".into(),
                }],
            )
            .await;
        assert_eq!(
            s.side_convos.len(),
            before,
            "no aside opened without allowance"
        );
        assert!(notes.iter().any(|n| n.contains("allowance")));
    }

    #[tokio::test]
    async fn leave_table_removes_from_queue_normal() {
        let mut s = CouncilSession::new(
            "Q?",
            CouncilSessionMode::Debate,
            vec![seat("a", "Alpha", "x"), seat("b", "Beta", "y")],
        );
        s.start();
        s.leave_table("b", "futile");
        assert!(s.seat("b").unwrap().has_left);
        assert!(!s.turn_queue.contains(&"b".to_owned()));
    }
}
