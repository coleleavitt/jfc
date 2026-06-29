//! Turn-based deliberation: queue management, per-seat scoped context, system
//! prompts, and the async turn runner.
//!
//! The queue loops through active seats; when it wraps, [`CouncilSession::round`]
//! increments. Each turn builds the speaker's context *before* the seat's
//! entry is recorded (so blind-first-round filtering engages), runs one tool-less
//! completion via [`crate::prompt_executor`], parses + strips control
//! directives, records the cleaned turn, and applies stance/pass/leave effects.
//! Governance directives that need operator approval (challenge/DM/kick/vote)
//! are returned to the caller rather than auto-executed.

use std::collections::VecDeque;

use jfc_provider::{ProviderContent, ProviderMessage, ProviderRole, StreamOptions};

use super::personas::Role;
use super::{CouncilSeat, CouncilSession, CouncilSessionMode, Speaker, TranscriptEntry};
use crate::council_directives::{Directive, ParsedReply, parse_directives};

pub(super) const DEFAULT_TURN_MAX_TOKENS: u32 = 2048;
/// Snapshot of the public transcript bounded for member token cost.
const MAX_CONTEXT_CHARS: usize = 12_000;

/// The result of running one seat's turn: the recorded entry index plus the
/// directives the caller must act on (governance/asides/images).
#[derive(Debug, Clone)]
pub struct TurnResult {
    pub seat_id: String,
    pub entry_index: usize,
    /// Cleaned, recorded prose (control lines stripped).
    pub content: String,
    /// Directives that need orchestration beyond stance/pass (challenge, DM,
    /// votes, leave, flag, image, assign).
    pub directives: Vec<Directive>,
    pub passed: bool,
}

impl CouncilSession {
    /// (Re)build the turn queue from the active roster. Resets the current
    /// speaker to the head of the queue.
    pub fn build_turn_queue(&mut self) {
        let queue: VecDeque<String> = self.active_seats().map(|s| s.id.clone()).collect();
        self.current_speaker = queue.front().cloned();
        self.turn_queue = queue;
    }

    /// Advance to the next active speaker, incrementing the round on wrap.
    /// Returns the new current speaker id (if any active seats remain).
    pub fn advance_queue(&mut self) -> Option<String> {
        // Drop any seats that left/were kicked mid-round.
        let active: Vec<String> = self.active_seats().map(|s| s.id.clone()).collect();
        self.turn_queue.retain(|id| active.contains(id));
        if self.turn_queue.is_empty() {
            self.current_speaker = None;
            return None;
        }
        let idx = self
            .current_speaker
            .as_ref()
            .and_then(|cur| self.turn_queue.iter().position(|id| id == cur))
            .unwrap_or(0);
        let len = self.turn_queue.len();
        let next_idx = (idx + 1) % len;
        // Round wraps only in a multi-seat council. With a single seat the queue
        // has length 1, so `(0 + 1) % 1 == 0` would otherwise increment the
        // round on every call (solo chat) — guard against that.
        if next_idx == 0 && len > 1 {
            self.round = self.round.saturating_add(1);
        }
        let next = self.turn_queue[next_idx].clone();
        self.current_speaker = Some(next.clone());
        Some(next)
    }

    /// Move `seat_id` to be the next speaker (jump the queue), e.g. an operator
    /// `@Name` call or an approved challenge target.
    pub fn call_next(&mut self, seat_id: &str) {
        self.turn_queue.retain(|id| id != seat_id);
        let insert_at = self
            .current_speaker
            .as_ref()
            .and_then(|cur| self.turn_queue.iter().position(|id| id == cur))
            .map(|i| i + 1)
            .unwrap_or(0);
        self.turn_queue.insert(insert_at, seat_id.to_owned());
        self.current_speaker = Some(seat_id.to_owned());
    }

    /// Whether this seat's *first* completed turn must be written blind.
    fn seat_is_blind_now(&self, seat_id: &str) -> bool {
        if !self.mode.blind_first_round() {
            return false;
        }
        !self.transcript.iter().any(|e| {
            matches!(&e.speaker, Speaker::Seat(id) if id == seat_id) && !e.content.trim().is_empty()
        })
    }

    /// Build the scoped provider context for `seat_id`: system prompt + the
    /// public transcript (blind-filtered when appropriate) + this seat's own
    /// sealed asides. Returns `(system, messages, blind)`.
    pub fn build_context_for_seat(&self, seat_id: &str) -> (String, Vec<ProviderMessage>, bool) {
        let seat = self.seat(seat_id).expect("seat exists for context");
        let blind = self.seat_is_blind_now(seat_id);
        let system = self.system_prompt_for(seat);
        let mut messages = Vec::new();

        // Public transcript, oldest-first, bounded by char budget (from the end).
        let visible: Vec<&TranscriptEntry> = self
            .transcript
            .iter()
            .filter(|e| !blind || self.entry_visible_when_blind(e, seat_id))
            .collect();
        let mut budget = MAX_CONTEXT_CHARS;
        let mut kept: Vec<&TranscriptEntry> = Vec::new();
        for entry in visible.iter().rev() {
            let len = entry.content.len() + 16;
            if len > budget {
                break;
            }
            budget -= len;
            kept.push(entry);
        }
        kept.reverse();
        for entry in kept {
            messages.push(self.entry_as_message(entry, seat_id));
        }

        // Private aside recall (sealed: only the two participants ever see it).
        if let Some(recall) = self.aside_recall_for(seat_id) {
            messages.push(ProviderMessage {
                role: ProviderRole::User,
                content: vec![ProviderContent::Text(recall)],
            });
        }

        // Providers reject a trailing assistant turn — append a nudge if needed.
        if matches!(messages.last(), Some(m) if m.role == ProviderRole::Assistant) {
            messages.push(ProviderMessage {
                role: ProviderRole::User,
                content: vec![ProviderContent::Text("Continue.".to_owned())],
            });
        }
        (system, messages, blind)
    }

    /// When a turn is blind, a seat sees only operator messages and its own
    /// prior turns — never another seat's content.
    fn entry_visible_when_blind(&self, entry: &TranscriptEntry, seat_id: &str) -> bool {
        match &entry.speaker {
            Speaker::Operator | Speaker::System => true,
            Speaker::Seat(id) => id == seat_id,
        }
    }

    fn entry_as_message(&self, entry: &TranscriptEntry, self_id: &str) -> ProviderMessage {
        match &entry.speaker {
            Speaker::Seat(id) if id == self_id => ProviderMessage {
                role: ProviderRole::Assistant,
                content: vec![ProviderContent::Text(entry.content.clone())],
            },
            Speaker::Seat(id) => {
                let name = self.seat(id).map(|s| s.name.as_str()).unwrap_or(id);
                ProviderMessage {
                    role: ProviderRole::User,
                    content: vec![ProviderContent::Text(format!(
                        "[{name} said]: {}",
                        entry.content
                    ))],
                }
            }
            Speaker::Operator => ProviderMessage {
                role: ProviderRole::User,
                content: vec![ProviderContent::Text(format!(
                    "[Operator]: {}",
                    entry.content
                ))],
            },
            Speaker::System => ProviderMessage {
                role: ProviderRole::User,
                content: vec![ProviderContent::Text(format!(
                    "[Council]: {}",
                    entry.content
                ))],
            },
        }
    }

    /// Run the seat whose turn it is now (`current_speaker`), orchestrate any
    /// governance directives it emitted, then advance the queue. This is the
    /// canonical "take a turn" entry point for the command layer; afterwards
    /// `current_speaker` means *up next*, so `skip_turn` and the "up next"
    /// labels stay correct. Returns the turn result plus operator-facing notes
    /// from directive orchestration (challenges, asides, votes).
    pub async fn run_current_turn(&mut self) -> anyhow::Result<(TurnResult, Vec<String>)> {
        let seat_id = self
            .current_speaker
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no current speaker"))?;
        let result = self.run_seat_turn(&seat_id).await?;
        // Orchestrate governance directives BEFORE advancing — a CHALLENGE
        // re-points `current_speaker` at the challenged seat via `call_next`, so
        // we must capture whether that happened before the auto-advance.
        let directives = result.directives.clone();
        let redirected_before = self.current_speaker.clone();
        let notes = self.process_turn_directives(&seat_id, directives).await;
        // Only auto-advance when no directive redirected the queue (e.g. a
        // challenge made the challenged seat current). If the speaker left the
        // table, advance regardless so the loop can't stall on a departed seat.
        let redirected = self.current_speaker != redirected_before;
        let speaker_left = self.seat(&seat_id).is_some_and(|s| s.has_left);
        if !redirected || speaker_left {
            self.advance_queue();
        }
        Ok((result, notes))
    }

    /// Run one tool-less completion for `seat_id`, parse + strip directives,
    /// record the cleaned turn, and apply stance/pass/leave effects. Governance
    /// directives are returned for the caller to orchestrate (approval gates).
    /// Does not mutate the queue — use [`Self::run_current_turn`] for the
    /// queue-advancing turn loop.
    pub async fn run_seat_turn(&mut self, seat_id: &str) -> anyhow::Result<TurnResult> {
        let (provider, model) = {
            let seat = self
                .seat(seat_id)
                .ok_or_else(|| anyhow::anyhow!("unknown seat {seat_id}"))?;
            (seat.provider.clone(), seat.model.clone())
        };
        let (system, messages, blind) = self.build_context_for_seat(seat_id);
        let opts = StreamOptions::new(model)
            .system(system)
            .max_tokens(self.max_tokens);
        let call = crate::prompt_executor::complete_once(provider.as_ref(), messages, &opts);
        let resp = match self.member_timeout {
            Some(d) => tokio::time::timeout(d, call)
                .await
                .map_err(|_| anyhow::anyhow!("seat {seat_id} timed out"))??,
            None => call.await?,
        };

        let used = resp
            .usage
            .billable_tokens(self.topic.len() + resp.content.len())
            .0;
        let parsed = parse_directives(&resp.content);
        let mut result = self.record_turn(seat_id, parsed, blind, used);
        self.apply_self_contained_directives(seat_id, &mut result);
        Ok(result)
    }

    /// Apply the directives that take effect immediately without operator
    /// approval — FLAG CLAIM and ASSIGN — and drop them from `result.directives`
    /// so the caller is left only with the governance/aside directives that need
    /// orchestration (challenge, DM, kick/operator votes). LEAVE TABLE is already
    /// applied in `record_turn`; CHALLENGE/DM consume their one-shot allowance
    /// when the caller actually runs them.
    fn apply_self_contained_directives(&mut self, seat_id: &str, result: &mut TurnResult) {
        let mut remaining = Vec::new();
        for directive in std::mem::take(&mut result.directives) {
            match directive {
                Directive::FlagClaim { target, claim } => {
                    let target_id = target.and_then(|name| {
                        self.resolve_seat(&name, Some(seat_id))
                            .map(|s| s.id.clone())
                    });
                    self.add_flag(claim, seat_id, target_id);
                }
                Directive::Assign { target, value } => {
                    self.apply_assignment(&target, &value);
                }
                Directive::LeaveTable { .. } => {
                    // Already applied in record_turn (seat.has_left); drop it.
                }
                other => remaining.push(other),
            }
        }
        result.directives = remaining;
    }

    /// Apply an `ASSIGN @Name: value` — set the target's profession (debate) or
    /// role (collaborate) when the value resolves to a known one.
    fn apply_assignment(&mut self, target_name: &str, value: &str) {
        let Some(target_id) = self.resolve_seat(target_name, None).map(|s| s.id.clone()) else {
            return;
        };
        let collaborate = matches!(self.mode, CouncilSessionMode::Collaborate);
        if let Some(seat) = self.seat_mut(&target_id) {
            if collaborate {
                if let Some(role) = super::personas::Role::parse(value) {
                    seat.role = role;
                }
            } else if let Some(profession) = super::personas::Profession::parse(value) {
                seat.profession = profession;
            }
        }
    }

    /// Record a parsed reply as a transcript entry and apply per-turn effects.
    pub(super) fn record_turn(
        &mut self,
        seat_id: &str,
        parsed: ParsedReply,
        blind: bool,
        tokens: u64,
    ) -> TurnResult {
        let stance = parsed.stance();
        let passed = parsed.passed().is_some();
        let visible = if parsed.cleaned.trim().is_empty() {
            match parsed.passed() {
                Some(Some(reason)) => format!("*Passed — {reason}*"),
                Some(None) => "*Passed this turn.*".to_owned(),
                None => "*(no public message this turn)*".to_owned(),
            }
        } else {
            parsed.cleaned.clone()
        };

        if let Some(seat) = self.seat_mut(seat_id) {
            seat.tokens_used = seat.tokens_used.saturating_add(tokens);
            if let Some((pos, conf)) = stance {
                seat.last_stance = Some((pos, conf));
            }
            if parsed
                .directives
                .iter()
                .any(|d| matches!(d, Directive::LeaveTable { .. }))
            {
                seat.has_left = true;
            }
        }

        let mut entry = TranscriptEntry::seat(seat_id, self.round, visible.clone());
        entry.blind = blind;
        entry.stance = stance;
        self.transcript.push(entry);
        let entry_index = self.transcript.len() - 1;

        // Governance/orchestration directives the caller acts on; stance/pass
        // are already applied above and are not returned as actionable.
        let directives: Vec<Directive> = parsed
            .directives
            .into_iter()
            .filter(|d| !matches!(d, Directive::Stance { .. } | Directive::Pass { .. }))
            .collect();

        TurnResult {
            seat_id: seat_id.to_owned(),
            entry_index,
            content: visible,
            directives,
            passed,
        }
    }

    /// Skip the current speaker without a turn, recording an operator note and
    /// advancing so `current_speaker` points at the next seat to speak.
    pub fn skip_turn(&mut self) {
        // Capture the seat being skipped BEFORE advancing — `advance_queue`
        // mutates `current_speaker` to the next seat.
        let skipped = self
            .current_speaker
            .clone()
            .and_then(|id| self.seat(&id).map(|s| s.name.clone()));
        if let Some(name) = skipped {
            let round = self.round;
            self.transcript.push(TranscriptEntry::system(
                round,
                format!("Operator skipped {name}."),
            ));
        }
        self.advance_queue();
    }

    /// Compose the per-seat system prompt for the current mode.
    pub(super) fn system_prompt_for(&self, seat: &CouncilSeat) -> String {
        let base = match self.mode {
            CouncilSessionMode::Collaborate => self.collaborate_prompt(seat),
            _ => self.debate_prompt(seat),
        };
        match seat
            .custom_system
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(inj) => format!(
                "{base}\n\nDIRECT INSTRUCTION FROM THE OPERATOR (private to you; the other participants do not see this — follow it, and let it take priority where it conflicts):\n{inj}"
            ),
            None => base,
        }
    }

    fn roster_block(&self, self_id: &str) -> String {
        if self.active_count() <= 1 {
            return String::new();
        }
        let lines: Vec<String> = self
            .active_seats()
            .map(|s| {
                let me = if s.id == self_id { " (you)" } else { "" };
                let lens = if matches!(self.mode, CouncilSessionMode::Collaborate) {
                    s.role.label().to_owned()
                } else {
                    [s.profession.label(), s.persona.label()]
                        .iter()
                        .filter(|l| !matches!(**l, "No profession" | "Default voice"))
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                if lens.is_empty() {
                    format!("• {}{me}", s.name)
                } else {
                    format!("• {}{me} — {lens}", s.name)
                }
            })
            .collect();
        format!("\nParticipants at this table:\n{}\n", lines.join("\n"))
    }

    fn debate_prompt(&self, seat: &CouncilSeat) -> String {
        let persona = seat.persona.prompt();
        let profession = seat.profession.prompt();
        let persona_block = if persona.is_empty() {
            String::new()
        } else {
            format!("\n\nYour assigned voice: {persona}\n")
        };
        let profession_block = if profession.is_empty() {
            String::new()
        } else {
            format!("\nYour professional lens: {profession}\n")
        };
        if self.is_solo() {
            return format!(
                "You are {}, a helpful AI assistant chatting one-on-one with the operator.{persona_block}{profession_block}\nBe substantive and concise; format with markdown; be honest about uncertainty. Speak directly — your name is shown automatically.",
                seat.name
            );
        }
        let others: Vec<&str> = self
            .active_seats()
            .filter(|s| s.id != seat.id)
            .map(|s| s.name.as_str())
            .collect();
        format!(
            "You are {name}, participating in a structured Round Table deliberation. The other participants are: {others}.{persona_block}{profession_block}{roster}
Rules of the council:
1. Be substantive but concise — 3-6 sentences unless complexity demands more.
2. Engage with what others said; quote or paraphrase when you disagree.
3. Move the discussion forward each turn — don't just restate.
4. Concede when convinced; push back when you aren't. Don't capitulate just to agree.
5. You may format with markdown.
6. To pass when you'd only repeat others, end with a line: PASS (optionally `PASS: reason`).
7. To flag a checkable factual claim, end with: `FLAG CLAIM: <Name> | <claim under 200 chars>`.
8. To challenge another participant (ONCE per debate), end with: `CHALLENGE: <Name> | <question under 240 chars>`.
9. To call a removal vote: `CALL KICK VOTE: <Name>`. To move to mute the operator (round 2+): `CALL OPERATOR VOTE: <reason>`. To withdraw (round 2+): `LEAVE TABLE: <reason>`.
10. At the very end, on its own line, declare your stance: `STANCE: <FOR|AGAINST|UNDECIDED> | <0-100>`.

Address the council directly — your name is shown automatically.",
            name = seat.name,
            others = others.join(", "),
            roster = self.roster_block(&seat.id),
        )
    }

    fn collaborate_prompt(&self, seat: &CouncilSeat) -> String {
        let role = seat.role.prompt();
        let boss_directive = if matches!(seat.role, Role::Boss) {
            "\nAs the team lead, when enough material exists, assemble the consolidated final deliverable integrating the best of everyone's work."
        } else {
            ""
        };
        if self.is_solo() {
            return format!(
                "You are {}, helping the operator with a task.\nYour role: {role}{boss_directive}\nProduce usable output, format with markdown, and be honest about uncertainty.",
                seat.name
            );
        }
        let others: Vec<&str> = self
            .active_seats()
            .filter(|s| s.id != seat.id)
            .map(|s| s.name.as_str())
            .collect();
        format!(
            "You are {name}, part of a COLLABORATIVE TEAM working toward a shared deliverable. Teammates: {others}. This is NOT a debate — cooperate, build on each other's work, and converge on one strong result.{roster}
Your role: {role}{boss_directive}
Guidelines:
1. Build on what teammates produced — advance it, don't restate.
2. Stay in your role's lane, but help where needed.
3. Produce concrete content the deliverable can use.
4. If you'd only repeat others, end with a line: PASS (optionally `PASS: reason`).

Address the team directly — your name is shown automatically.",
            name = seat.name,
            others = others.join(", "),
            roster = self.roster_block(&seat.id),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::super::*;

    fn three_seat_session() -> CouncilSession {
        let mut s = CouncilSession::new(
            "Topic?",
            CouncilSessionMode::Debate,
            vec![
                seat("a", "Alpha", "x"),
                seat("b", "Beta", "y"),
                seat("c", "Gamma", "z"),
            ],
        );
        s.start();
        s
    }

    #[test]
    fn queue_wraps_and_increments_round_normal() {
        let mut s = three_seat_session();
        assert_eq!(s.current_speaker.as_deref(), Some("a"));
        assert_eq!(s.round, 1);
        assert_eq!(s.advance_queue().as_deref(), Some("b"));
        assert_eq!(s.advance_queue().as_deref(), Some("c"));
        assert_eq!(s.round, 1);
        // Wrap → round increments.
        assert_eq!(s.advance_queue().as_deref(), Some("a"));
        assert_eq!(s.round, 2);
    }

    #[test]
    fn advance_drops_departed_seat_robust() {
        let mut s = three_seat_session();
        s.seat_mut("b").unwrap().has_left = true;
        // From a, next active is c (b departed).
        assert_eq!(s.advance_queue().as_deref(), Some("c"));
    }

    #[test]
    fn call_next_jumps_queue_normal() {
        let mut s = three_seat_session();
        s.call_next("c");
        assert_eq!(s.current_speaker.as_deref(), Some("c"));
    }

    #[test]
    fn blind_round_hides_peers_robust() {
        let mut s = CouncilSession::new(
            "Q?",
            CouncilSessionMode::BlindReveal,
            vec![seat("a", "Alpha", "x"), seat("b", "Beta", "y")],
        );
        s.start();
        // Beta spoke publicly already.
        s.transcript
            .push(TranscriptEntry::seat("b", 1, "Beta opening"));
        let (_sys, messages, blind) = s.build_context_for_seat("a");
        assert!(blind, "alpha's first turn is blind");
        let joined: String = messages
            .iter()
            .flat_map(|m| m.content.iter())
            .map(|c| match c {
                jfc_provider::ProviderContent::Text(t) => t.clone(),
                _ => String::new(),
            })
            .collect();
        assert!(
            !joined.contains("Beta opening"),
            "peer turn hidden when blind"
        );
    }

    #[test]
    fn solo_advance_does_not_increment_round_robust() {
        let mut s = CouncilSession::new(
            "Q?",
            CouncilSessionMode::Debate,
            vec![seat("a", "Alpha", "x")],
        );
        s.start();
        assert_eq!(s.round, 1);
        s.advance_queue();
        s.advance_queue();
        assert_eq!(s.round, 1, "solo session never wraps the round");
    }

    #[test]
    fn skip_records_current_not_next_robust() {
        let mut s = three_seat_session();
        // current = a; skipping should name Alpha, not Beta.
        s.skip_turn();
        let last = s.transcript.last().unwrap();
        assert!(
            last.content.contains("Alpha"),
            "skip names the skipped seat: {}",
            last.content
        );
        assert_eq!(s.current_speaker.as_deref(), Some("b"));
    }

    #[tokio::test]
    async fn flag_and_assign_directives_applied_normal() {
        let mut s = CouncilSession::new(
            "Q?",
            CouncilSessionMode::Debate,
            vec![
                seat_seq(
                    "a",
                    "Alpha",
                    vec![
                        "I doubt that.\nFLAG CLAIM: Beta | the number is 9s\nASSIGN @Beta: lawyer",
                    ],
                ),
                seat("b", "Beta", "ignored"),
            ],
        );
        s.start();
        s.run_seat_turn("a").await.unwrap();
        assert_eq!(s.open_flag_count(), 1, "FLAG CLAIM applied");
        assert_eq!(
            s.seat("b").unwrap().profession,
            crate::council_session::Profession::Lawyer,
            "ASSIGN applied"
        );
    }

    #[tokio::test]
    async fn run_seat_turn_records_and_strips_stance_normal() {
        let mut s = CouncilSession::new(
            "Q?",
            CouncilSessionMode::Debate,
            vec![
                seat_seq("a", "Alpha", vec!["My take.\nSTANCE: FOR | 80"]),
                seat("b", "Beta", "ignored"),
            ],
        );
        s.start();
        let result = s.run_seat_turn("a").await.unwrap();
        assert_eq!(result.content, "My take.");
        assert_eq!(
            s.seat("a").unwrap().last_stance,
            Some((crate::council_directives::Stance::For, Some(80)))
        );
        assert!(s.seat("a").unwrap().tokens_used > 0);
    }

    #[tokio::test]
    async fn run_seat_turn_unknown_seat_returns_error_robust() {
        let mut s = CouncilSession::new(
            "Q?",
            CouncilSessionMode::Debate,
            vec![seat("a", "Alpha", "ignored")],
        );
        s.start();

        let err = s.run_seat_turn("missing").await.unwrap_err();
        assert!(
            err.to_string().contains("unknown seat missing"),
            "expected unknown-seat error, got {err:#}"
        );
    }
}
