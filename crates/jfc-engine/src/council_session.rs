//! Persistent, turn-based **Council Session** — the RoundTable state machine.
//!
//! Where [`crate::council`] fans a question out once and synthesises a single
//! report, a [`CouncilSession`] is a *resumable* deliberation: a frozen roster
//! of [`CouncilSeat`]s takes turns, the operator steers between turns, models
//! emit [control directives](crate::council_directives), and the session ends
//! on an accepted verdict or an explicit `/end`.
//!
//! Ownership boundary: this module owns the **state machine and orchestration**
//! (roster, transcript, turn queue, consensus/verdict/governance). It reuses
//! [`crate::prompt_executor::complete_once`] for transport and
//! [`crate::council_directives`] for the directive grammar. It is provider-
//! agnostic and free of `EngineState`/registry coupling so it stays unit-
//! testable with mock providers — the slash-command layer
//! ([`crate::commands::context`]) adapts it to the live engine.

mod blind_map;
mod governance;
mod pending;
mod personas;
mod scoring;
mod turns;
mod verdict;

pub use blind_map::BlindMapReport;
pub use governance::{FlagStatus, FlaggedClaim, KickOutcome, SideConvo, SideConvoTurn};
pub use pending::PendingCouncilAction;
pub use personas::{Persona, Profession, Role};
pub use scoring::{ChoiceGroup, choices_match, group_choices};
pub use verdict::{ConsensusReply, MemberVerdict, VerdictOutcome};

use std::{collections::VecDeque, sync::Arc};

use jfc_provider::{ModelId, Provider};
use serde::Serialize;

use crate::council_directives::Stance;

/// Hard ceiling on seats per table, bounding cost/latency like the web client.
pub const MAX_SEATS: usize = 16;
/// Default per-seat aside allowance (sealed model↔model DMs the seat may open).
pub const DEFAULT_ASIDE_ALLOWANCE: u32 = 1;
/// Default suggested rounds before the session nudges toward a verdict.
pub const DEFAULT_MAX_ROUNDS: u32 = 4;

/// Deliberation style for a whole session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CouncilSessionMode {
    /// Adversarial: seats argue, challenge, and converge only when convinced.
    #[default]
    Debate,
    /// Cooperative: seats hold work roles and build a shared deliverable.
    Collaborate,
    /// Opening statements are committed independently (blind), then revealed.
    BlindReveal,
    /// One-shot pipeline: blind answers → red-team → synthesis.
    BlindMapReduce,
}

impl CouncilSessionMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value
            .trim()
            .to_ascii_lowercase()
            .replace(['-', '_'], "")
            .as_str()
        {
            "debate" => Some(Self::Debate),
            "collaborate" | "collab" => Some(Self::Collaborate),
            "blindreveal" | "blind" => Some(Self::BlindReveal),
            "blindmapreduce" | "blindmap" | "mapreduce" => Some(Self::BlindMapReduce),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Debate => "debate",
            Self::Collaborate => "collaborate",
            Self::BlindReveal => "blind-reveal",
            Self::BlindMapReduce => "blind-map-reduce",
        }
    }

    /// Whether opening turns must be written blind to the other seats.
    pub fn blind_first_round(self) -> bool {
        matches!(self, Self::BlindReveal | Self::BlindMapReduce)
    }
}

/// Lifecycle phase of the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionPhase {
    Idle,
    Debating,
    Voting,
    Concluded,
}

/// One participant. Carries identity, the provider binding, persona/role, and
/// the mutable per-seat state (kicked/left, spend, aside allowance, last stance).
#[derive(Clone)]
pub struct CouncilSeat {
    pub id: String,
    pub name: String,
    pub provider: Arc<dyn Provider>,
    pub model: ModelId,
    pub persona: Persona,
    pub profession: Profession,
    pub role: Role,
    /// Operator's private system-prompt injection, never shown to other seats.
    pub custom_system: Option<String>,
    pub kicked: bool,
    pub has_left: bool,
    pub tokens_used: u64,
    /// Remaining sealed asides this seat may open (0 = none / operator-only).
    pub asides_remaining: u32,
    /// One direct challenge per debate.
    pub challenge_used: bool,
    pub last_stance: Option<(Stance, Option<u8>)>,
}

impl CouncilSeat {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        provider: Arc<dyn Provider>,
        model: impl Into<ModelId>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            provider,
            model: model.into(),
            persona: Persona::Default,
            profession: Profession::None,
            role: Role::Worker,
            custom_system: None,
            kicked: false,
            has_left: false,
            tokens_used: 0,
            asides_remaining: DEFAULT_ASIDE_ALLOWANCE,
            challenge_used: false,
            last_stance: None,
        }
    }

    pub fn with_persona(mut self, persona: Persona) -> Self {
        self.persona = persona;
        self
    }

    pub fn with_profession(mut self, profession: Profession) -> Self {
        self.profession = profession;
        self
    }

    pub fn with_role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// At the table right now (not kicked, not departed).
    pub fn active(&self) -> bool {
        !self.kicked && !self.has_left
    }
}

/// Who authored a transcript entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "id")]
pub enum Speaker {
    Operator,
    System,
    Seat(String),
}

/// One entry in the public transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TranscriptEntry {
    pub speaker: Speaker,
    pub round: u32,
    pub content: String,
    /// Written blind (the seat couldn't see the others when composing it).
    #[serde(default)]
    pub blind: bool,
    /// Stance declared on this turn, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stance: Option<(Stance, Option<u8>)>,
}

impl TranscriptEntry {
    pub fn operator(round: u32, content: impl Into<String>) -> Self {
        Self {
            speaker: Speaker::Operator,
            round,
            content: content.into(),
            blind: false,
            stance: None,
        }
    }

    pub fn system(round: u32, content: impl Into<String>) -> Self {
        Self {
            speaker: Speaker::System,
            round,
            content: content.into(),
            blind: false,
            stance: None,
        }
    }

    pub fn seat(id: impl Into<String>, round: u32, content: impl Into<String>) -> Self {
        Self {
            speaker: Speaker::Seat(id.into()),
            round,
            content: content.into(),
            blind: false,
            stance: None,
        }
    }
}

/// The full persistent session state.
pub struct CouncilSession {
    pub topic: String,
    pub mode: CouncilSessionMode,
    pub phase: SessionPhase,
    pub seats: Vec<CouncilSeat>,
    pub transcript: Vec<TranscriptEntry>,
    pub round: u32,
    pub max_rounds: u32,
    pub turn_queue: VecDeque<String>,
    pub current_speaker: Option<String>,
    pub side_convos: Vec<SideConvo>,
    pub flagged_claims: Vec<FlaggedClaim>,
    pub operator_muted: bool,
    /// Pending verdict, populated by [`Self`] verdict helpers.
    pub verdict: Option<VerdictOutcome>,
    /// Per-member completion budget (max output tokens) for a turn.
    pub max_tokens: u32,
    /// Optional per-member timeout for a turn.
    pub member_timeout: Option<std::time::Duration>,
    pub pending_action: Option<PendingCouncilAction>,
}

impl CouncilSession {
    /// Start a session. `seats` is the frozen roster; solo mode (one active
    /// seat) disables council mechanics in prompt construction.
    pub fn new(
        topic: impl Into<String>,
        mode: CouncilSessionMode,
        seats: Vec<CouncilSeat>,
    ) -> Self {
        let mut session = Self {
            topic: topic.into(),
            mode,
            phase: SessionPhase::Idle,
            seats,
            transcript: Vec::new(),
            round: 1,
            max_rounds: DEFAULT_MAX_ROUNDS,
            turn_queue: VecDeque::new(),
            current_speaker: None,
            side_convos: Vec::new(),
            flagged_claims: Vec::new(),
            operator_muted: false,
            verdict: None,
            max_tokens: crate::council_session::turns::DEFAULT_TURN_MAX_TOKENS,
            member_timeout: Some(std::time::Duration::from_secs(120)),
            pending_action: None,
        };
        session.build_turn_queue();
        session
    }

    pub fn with_max_rounds(mut self, rounds: u32) -> Self {
        self.max_rounds = rounds.max(1);
        self
    }

    /// Begin the session: push the operator topic, set phase, seed the queue.
    pub fn start(&mut self) {
        self.phase = SessionPhase::Debating;
        let topic = self.topic.clone();
        self.transcript.push(TranscriptEntry::operator(0, topic));
        self.build_turn_queue();
    }

    /// Active seats (not kicked, not departed).
    pub fn active_seats(&self) -> impl Iterator<Item = &CouncilSeat> {
        self.seats.iter().filter(|s| s.active())
    }

    pub fn active_count(&self) -> usize {
        self.active_seats().count()
    }

    /// One active seat → solo chat; council mechanics are disabled.
    pub fn is_solo(&self) -> bool {
        self.active_count() == 1
    }

    pub fn seat(&self, id: &str) -> Option<&CouncilSeat> {
        self.seats.iter().find(|s| s.id == id)
    }

    pub fn seat_mut(&mut self, id: &str) -> Option<&mut CouncilSeat> {
        self.seats.iter_mut().find(|s| s.id == id)
    }

    pub fn is_concluded(&self) -> bool {
        self.phase == SessionPhase::Concluded
    }

    /// Resolve a participant by loose name/id match, excluding `exclude_id`.
    /// Returns `None` when the name is ambiguous (matches >1 seat) or unknown,
    /// mirroring the web client's `resolveCalloutTarget` conservatism.
    pub fn resolve_seat(&self, name: &str, exclude_id: Option<&str>) -> Option<&CouncilSeat> {
        let needle = name.trim().to_ascii_lowercase();
        if needle.is_empty() {
            return None;
        }
        let pool: Vec<&CouncilSeat> = self
            .active_seats()
            .filter(|s| exclude_id != Some(s.id.as_str()))
            .collect();
        if let Some(hit) = pool.iter().find(|s| s.name.to_ascii_lowercase() == needle) {
            return Some(*hit);
        }
        if let Some(hit) = pool.iter().find(|s| s.id.to_ascii_lowercase() == needle) {
            return Some(*hit);
        }
        let loose: Vec<&&CouncilSeat> = pool
            .iter()
            .filter(|s| {
                let n = s.name.to_ascii_lowercase();
                let id = s.id.to_ascii_lowercase();
                needle.contains(&n)
                    || needle.contains(&id)
                    || (n.contains(&needle) && needle.len() >= 3)
            })
            .collect();
        match loose.as_slice() {
            [only] => Some(**only),
            _ => None,
        }
    }

    /// Conclude the session.
    pub fn conclude(&mut self) {
        self.phase = SessionPhase::Concluded;
        self.current_speaker = None;
    }

    /// Total tokens spent across all seats this session.
    pub fn total_tokens(&self) -> u64 {
        self.seats.iter().map(|s| s.tokens_used).sum()
    }

    /// Render the session transcript as a Markdown block for a chat surface.
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("## Council — {}\n\n", self.mode.as_str()));
        out.push_str(&format!("_Topic: {}_\n\n", self.topic));
        let mut last_round = u32::MAX;
        for entry in &self.transcript {
            if entry.round != last_round && entry.round > 0 {
                out.push_str(&format!("\n**Round {:02}**\n\n", entry.round));
                last_round = entry.round;
            }
            let who = match &entry.speaker {
                Speaker::Operator => "Operator".to_owned(),
                Speaker::System => "Council Record".to_owned(),
                Speaker::Seat(id) => self
                    .seat(id)
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| id.clone()),
            };
            let blind = if entry.blind { " · blind" } else { "" };
            out.push_str(&format!("**{who}**{blind}: {}\n\n", entry.content));
        }
        if let Some(verdict) = &self.verdict {
            out.push_str("\n---\n");
            out.push_str(&verdict.to_markdown());
        }
        out
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use anyhow::{Result, anyhow};
    use async_trait::async_trait;
    use jfc_provider::{
        CompletionResponse, EventStream, ModelInfo, ProviderMessage, StreamConvention,
        StreamOptions, TokenUsage,
    };
    use std::sync::Mutex;

    /// A provider that pops scripted replies in order, repeating the last.
    pub struct ScriptedProvider {
        pub name: &'static str,
        pub replies: Mutex<Vec<Result<String>>>,
    }

    impl ScriptedProvider {
        pub fn answering(name: &'static str, reply: &str) -> Arc<Self> {
            Arc::new(Self {
                name,
                replies: Mutex::new(vec![Ok(reply.to_owned())]),
            })
        }

        pub fn sequence(name: &'static str, replies: Vec<&str>) -> Arc<Self> {
            Arc::new(Self {
                name,
                replies: Mutex::new(replies.into_iter().map(|s| Ok(s.to_owned())).collect()),
            })
        }
    }

    #[async_trait]
    impl Provider for ScriptedProvider {
        fn name(&self) -> &str {
            self.name
        }
        fn available_models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }
        fn stream_convention(&self) -> StreamConvention {
            StreamConvention::AnthropicNative
        }
        async fn stream(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> Result<EventStream> {
            Err(anyhow!("stream not used in council-session tests"))
        }
        async fn complete(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> Result<CompletionResponse> {
            let mut guard = self.replies.lock().unwrap();
            let next = if guard.len() > 1 {
                guard.remove(0)
            } else {
                match guard.first() {
                    Some(Ok(s)) => Ok(s.clone()),
                    Some(Err(e)) => Err(anyhow!("{e}")),
                    None => Err(anyhow!("no scripted reply")),
                }
            };
            next.map(|content| CompletionResponse {
                content,
                usage: TokenUsage {
                    input_tokens: 40,
                    output_tokens: 20,
                    thinking_tokens: None,
                    cache_read_tokens: 0,
                    cache_creation_tokens: 0,
                },
                context_signals: None,
            })
        }
    }
    impl jfc_provider::seal::Sealed for ScriptedProvider {}

    /// Build a seat backed by a single-answer scripted provider.
    pub fn seat(id: &'static str, name: &str, reply: &str) -> CouncilSeat {
        CouncilSeat::new(
            id,
            name,
            ScriptedProvider::answering(id, reply),
            format!("{id}-model"),
        )
    }

    /// Build a seat backed by a multi-reply scripted provider.
    pub fn seat_seq(id: &'static str, name: &str, replies: Vec<&str>) -> CouncilSeat {
        CouncilSeat::new(
            id,
            name,
            ScriptedProvider::sequence(id, replies),
            format!("{id}-model"),
        )
    }
}

#[cfg(test)]
mod roundtable_tests;

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    #[test]
    fn solo_detection_normal() {
        let session = CouncilSession::new(
            "Q?",
            CouncilSessionMode::Debate,
            vec![seat("a", "Alpha", "hi")],
        );
        assert!(session.is_solo());
        assert_eq!(session.active_count(), 1);
    }

    #[test]
    fn resolve_seat_exact_and_loose_robust() {
        let session = CouncilSession::new(
            "Q?",
            CouncilSessionMode::Debate,
            vec![seat("claude", "Claude", "x"), seat("gpt", "GPT", "y")],
        );
        assert_eq!(
            session.resolve_seat("Claude", None).map(|s| s.id.as_str()),
            Some("claude")
        );
        assert_eq!(
            session.resolve_seat("gpt", None).map(|s| s.id.as_str()),
            Some("gpt")
        );
        // Excluding self → none for a self-addressed name.
        assert!(session.resolve_seat("Claude", Some("claude")).is_none());
    }

    #[test]
    fn start_pushes_topic_and_builds_queue_normal() {
        let mut session = CouncilSession::new(
            "Why blue?",
            CouncilSessionMode::Debate,
            vec![seat("a", "Alpha", "x"), seat("b", "Beta", "y")],
        );
        session.start();
        assert_eq!(session.phase, SessionPhase::Debating);
        assert_eq!(session.transcript.len(), 1);
        assert_eq!(session.turn_queue.len(), 2);
        assert_eq!(session.current_speaker.as_deref(), Some("a"));
    }

    #[test]
    fn mode_round_trip_normal() {
        for m in [
            CouncilSessionMode::Debate,
            CouncilSessionMode::Collaborate,
            CouncilSessionMode::BlindReveal,
            CouncilSessionMode::BlindMapReduce,
        ] {
            assert_eq!(CouncilSessionMode::parse(m.as_str()), Some(m));
        }
        assert!(CouncilSessionMode::BlindReveal.blind_first_round());
        assert!(!CouncilSessionMode::Debate.blind_first_round());
    }

    /// Full lifecycle: two turns each, then a unanimous verdict concludes the
    /// session. Exercises the queue, turn runner, and verdict path end to end.
    #[tokio::test]
    async fn full_debate_to_verdict_normal() {
        let mut session = CouncilSession::new(
            "Cache: Redis or Postgres?",
            CouncilSessionMode::Debate,
            vec![
                seat_seq(
                    "a",
                    "Alpha",
                    vec![
                        "Redis is faster for this.\nSTANCE: FOR | 70",
                        "Still Redis.\nSTANCE: FOR | 80",
                        "CHOICE: Redis\nPOSITION: lowest latency",
                    ],
                ),
                seat_seq(
                    "b",
                    "Beta",
                    vec![
                        "I lean Redis too.\nSTANCE: FOR | 60",
                        "Convinced — Redis.\nSTANCE: FOR | 75",
                        "CHOICE: redis\nPOSITION: agree, latency wins",
                    ],
                ),
            ],
        );
        session.start();
        // Round 1: a (opening already? no — start doesn't auto-run here).
        let first = session.current_speaker.clone().unwrap();
        session.run_seat_turn(&first).await.unwrap();
        for _ in 0..3 {
            if let Some(next) = session.advance_queue() {
                session.run_seat_turn(&next).await.unwrap();
            }
        }
        // Both seats declared a FOR stance.
        assert!(session.seat("a").unwrap().last_stance.is_some());
        assert!(session.seat("b").unwrap().last_stance.is_some());

        let outcome = session.trigger_verdict().await.unwrap();
        assert!(outcome.unanimous, "Redis ~= redis clusters to one choice");
        assert!(session.is_concluded());
        assert!(session.total_tokens() > 0);
        let md = session.to_markdown();
        assert!(md.contains("Unanimous verdict"));
    }
}
