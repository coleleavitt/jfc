//! Consensus, synthesis, and the formal verdict flow for [`CouncilSession`].
//!
//! - **Consensus** is a soft poll (AGREE/DISAGREE + one-line summary).
//! - **Synthesis** asks one chair seat to distil the whole debate.
//! - **Verdict** is the formal finalization: collect final CHOICE+POSITION from
//!   each seat, fuzzy-group choices (via [`super::scoring`]); if unanimous,
//!   conclude; otherwise run a secret ballot, tally, and surface ties for the
//!   operator to break. Open flagged claims soft-block the verdict.

use serde::Serialize;

use super::scoring::group_choices;
use super::{CouncilSession, SessionPhase, TranscriptEntry};

/// One seat's consensus reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConsensusReply {
    pub seat_id: String,
    pub agree: bool,
    pub summary: String,
}

/// One seat's final verdict position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemberVerdict {
    pub seat_id: String,
    pub choice: String,
    pub position: String,
}

/// The resolved verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VerdictOutcome {
    pub unanimous: bool,
    /// Winning choice text.
    pub choice: String,
    /// Carried-by seat name (or "all participants" when unanimous).
    pub winner: String,
    /// `true` when a tie was left for the operator to break.
    pub tie: bool,
    pub positions: Vec<MemberVerdict>,
}

impl VerdictOutcome {
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        if self.unanimous {
            out.push_str(&format!("**Unanimous verdict:** {}\n", self.choice));
        } else if self.tie {
            out.push_str(&format!(
                "**Verdict (tie — operator decides):** {}\n",
                self.choice
            ));
        } else {
            out.push_str(&format!(
                "**Verdict:** {} — carried by {}\n",
                self.choice, self.winner
            ));
        }
        out
    }
}

impl CouncilSession {
    /// Soft consensus poll. Records a system summary; leaves the session open.
    pub async fn check_consensus(&mut self) -> anyhow::Result<Vec<ConsensusReply>> {
        if self.active_count() < 2 {
            return Err(anyhow::anyhow!("consensus needs at least 2 participants"));
        }
        let prompt = "Consensus check. Reply EXACTLY two lines, nothing else:\nVERDICT: AGREE or DISAGREE\nSUMMARY: <one sentence on what the council appears to have concluded>";
        let voters: Vec<String> = self.active_seats().map(|s| s.id.clone()).collect();
        let mut replies = Vec::new();
        for seat_id in &voters {
            let raw = self.poll_seat(seat_id, prompt).await.unwrap_or_default();
            replies.push(parse_consensus(seat_id, &raw));
        }
        let mut summary = String::from("CONSENSUS CHECK\n");
        for r in &replies {
            let name = self
                .seat(&r.seat_id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| r.seat_id.clone());
            let verdict = if r.agree { "AGREE" } else { "DISAGREE" };
            summary.push_str(&format!("\n{name}: {verdict} — {}", r.summary));
        }
        self.transcript
            .push(TranscriptEntry::system(self.round, summary));
        Ok(replies)
    }

    /// Chair synthesis: one seat distils the debate into a single answer.
    pub async fn synthesize(&mut self, chair_id: Option<&str>) -> anyhow::Result<String> {
        let chair = chair_id
            .map(str::to_owned)
            .or_else(|| self.active_seats().next().map(|s| s.id.clone()))
            .ok_or_else(|| anyhow::anyhow!("no chair available"))?;
        let prompt = "You are the chair. Read the discussion and write a SYNTHESIS for the operator with these headings:\n## Where the council landed\n## Key disagreements\n## Strongest case on each side\n## Bottom line\nBe even-handed; do not invent positions nobody took.";
        let synthesis = self.poll_seat(&chair, prompt).await?;
        let name = self.seat(&chair).map(|s| s.name.clone()).unwrap_or(chair);
        self.transcript.push(TranscriptEntry::system(
            self.round,
            format!("SYNTHESIS — chair {name}\n\n{synthesis}"),
        ));
        Ok(synthesis)
    }

    /// Whether the verdict is soft-blocked by unresolved flagged claims.
    pub fn verdict_blocked_by_flags(&self) -> bool {
        self.open_flag_count() > 0
    }

    /// Run the formal verdict flow: collect positions → group → (unanimous |
    /// ballot) → tally. On a tie, `tie = true` and the operator must break it
    /// with [`Self::break_tie`]. Sets [`Self::verdict`] and phase to Concluded
    /// for a clean (non-tie) result.
    pub async fn trigger_verdict(&mut self) -> anyhow::Result<VerdictOutcome> {
        if self.active_count() < 2 {
            return Err(anyhow::anyhow!("verdict needs at least 2 participants"));
        }
        self.phase = SessionPhase::Voting;
        let positions = self.collect_positions().await?;
        if positions.len() < 2 {
            self.phase = SessionPhase::Debating;
            return Err(anyhow::anyhow!("not enough seats produced a position"));
        }

        let groups = group_choices(
            positions
                .iter()
                .map(|p| (p.seat_id.as_str(), p.choice.as_str())),
        );
        if groups.len() == 1 {
            let outcome = VerdictOutcome {
                unanimous: true,
                choice: groups[0].choice.clone(),
                winner: "all participants".to_owned(),
                tie: false,
                positions,
            };
            self.finalize_verdict(outcome.clone());
            return Ok(outcome);
        }

        let ballots = self.run_ballot(&positions).await;
        let outcome = self.tally_ballot(positions, ballots);
        if outcome.tie {
            // Stash the pending tie so `break_tie` can resolve it. The session
            // stays in `Voting` (not concluded) until the operator picks.
            self.verdict = Some(outcome.clone());
        } else {
            self.finalize_verdict(outcome.clone());
        }
        Ok(outcome)
    }

    /// Resolve a tie by operator pick (the chosen seat's position carries).
    pub fn break_tie(&mut self, winning_seat_id: &str) -> Option<VerdictOutcome> {
        let pending = self.verdict.clone();
        let positions = match &pending {
            Some(v) if v.tie => v.positions.clone(),
            _ => return None,
        };
        let winner = positions.iter().find(|p| p.seat_id == winning_seat_id)?;
        let name = self
            .seat(winning_seat_id)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| winning_seat_id.to_owned());
        let outcome = VerdictOutcome {
            unanimous: false,
            choice: winner.choice.clone(),
            winner: format!("{name} (operator broke tie)"),
            tie: false,
            positions,
        };
        self.finalize_verdict(outcome.clone());
        Some(outcome)
    }

    fn finalize_verdict(&mut self, outcome: VerdictOutcome) {
        let headline = if outcome.unanimous {
            format!("UNANIMOUS VERDICT\n\n{}", outcome.choice)
        } else {
            format!(
                "VERDICT — carried by {}\n\n{}",
                outcome.winner, outcome.choice
            )
        };
        self.transcript
            .push(TranscriptEntry::system(self.round, headline));
        self.verdict = Some(outcome);
        self.phase = SessionPhase::Concluded;
        self.current_speaker = None;
    }

    async fn collect_positions(&self) -> anyhow::Result<Vec<MemberVerdict>> {
        let prompt = "The council is called to a verdict. Reply EXACTLY two lines:\nCHOICE: <your bottom-line answer in 1-6 words>\nPOSITION: <reasoning in 1-2 sentences>";
        let voters: Vec<String> = self.active_seats().map(|s| s.id.clone()).collect();
        let mut positions = Vec::new();
        for seat_id in &voters {
            if let Ok(raw) = self.poll_seat(seat_id, prompt).await {
                if let Some(v) = parse_verdict(seat_id, &raw) {
                    positions.push(v);
                }
            }
        }
        Ok(positions)
    }

    /// Each seat votes for the most compelling *other* position (1-based index).
    async fn run_ballot(&self, positions: &[MemberVerdict]) -> Vec<Option<usize>> {
        let list: String = positions
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let name = self
                    .seat(&p.seat_id)
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| p.seat_id.clone());
                format!("{}. {name} — {}\n   {}", i + 1, p.choice, p.position)
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        let mut ballots = Vec::with_capacity(positions.len());
        for (idx, p) in positions.iter().enumerate() {
            let own = idx + 1;
            let prompt = format!(
                "Final positions:\n\n{list}\n\nVote for the position you find MOST COMPELLING. You may NOT vote for your own (#{own}). Reply with ONLY the number.",
            );
            let vote = self
                .poll_seat(&p.seat_id, &prompt)
                .await
                .ok()
                .and_then(|raw| parse_ballot(&raw, positions.len(), own));
            ballots.push(vote);
        }
        ballots
    }

    fn tally_ballot(
        &self,
        positions: Vec<MemberVerdict>,
        ballots: Vec<Option<usize>>,
    ) -> VerdictOutcome {
        let mut tallies = vec![0usize; positions.len()];
        for choice in ballots.into_iter().flatten() {
            if (1..=positions.len()).contains(&choice) {
                tallies[choice - 1] += 1;
            }
        }
        let max = tallies.iter().copied().max().unwrap_or(0);
        let winners: Vec<usize> = (0..positions.len())
            .filter(|i| tallies[*i] == max)
            .collect();
        let tie = winners.len() > 1 && max > 0;
        let winning = winners.first().copied().unwrap_or(0);
        let winner_pos = &positions[winning];
        let winner_name = self
            .seat(&winner_pos.seat_id)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| winner_pos.seat_id.clone());
        VerdictOutcome {
            unanimous: false,
            choice: winner_pos.choice.clone(),
            winner: if tie {
                "tie — operator decides".to_owned()
            } else {
                winner_name
            },
            tie,
            positions,
        }
    }
}

fn parse_consensus(seat_id: &str, raw: &str) -> ConsensusReply {
    let upper = raw.to_ascii_uppercase();
    // DISAGREE contains AGREE, so check the negative first.
    let agree =
        !(upper.contains("DISAGREE") || upper.contains("DON'T AGREE")) && upper.contains("AGREE");
    let summary = raw
        .lines()
        .find_map(|l| {
            let t = l.trim();
            t.to_ascii_uppercase().strip_prefix("SUMMARY").map(|_| {
                t.split_once([':', '-'])
                    .map(|x| x.1)
                    .unwrap_or("")
                    .trim()
                    .to_owned()
            })
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| raw.trim().lines().next().unwrap_or("").to_owned());
    ConsensusReply {
        seat_id: seat_id.to_owned(),
        agree,
        summary,
    }
}

fn parse_verdict(seat_id: &str, raw: &str) -> Option<MemberVerdict> {
    let line_value = |key: &str| {
        raw.lines().find_map(|l| {
            let t = l.trim();
            if t.to_ascii_uppercase().starts_with(key) {
                t.split_once([':', '-']).map(|x| x.1.trim().to_owned())
            } else {
                None
            }
        })
    };
    let mut choice = line_value("CHOICE").unwrap_or_default();
    let mut position = line_value("POSITION").unwrap_or_default();
    if choice.is_empty() {
        choice = raw
            .trim()
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(60)
            .collect();
    }
    if position.is_empty() {
        position = raw.trim().to_owned();
    }
    let choice = choice.trim_matches(['"', '\'', '`', '.']).trim().to_owned();
    if choice.is_empty() {
        return None;
    }
    Some(MemberVerdict {
        seat_id: seat_id.to_owned(),
        choice,
        position,
    })
}

fn parse_ballot(raw: &str, count: usize, own: usize) -> Option<usize> {
    let n: usize = raw
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()?;
    (n >= 1 && n <= count && n != own).then_some(n)
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::super::*;

    #[test]
    fn parse_consensus_negative_first_robust() {
        let r = super::parse_consensus("a", "VERDICT: DISAGREE\nSUMMARY: not convinced");
        assert!(!r.agree);
        assert_eq!(r.summary, "not convinced");
        let r2 = super::parse_consensus("b", "VERDICT: AGREE\nSUMMARY: aligned");
        assert!(r2.agree);
    }

    #[test]
    fn parse_verdict_extracts_choice_normal() {
        let v = super::parse_verdict("a", "CHOICE: Option A\nPOSITION: it's cheapest").unwrap();
        assert_eq!(v.choice, "Option A");
        assert_eq!(v.position, "it's cheapest");
    }

    #[test]
    fn parse_ballot_rejects_self_and_oob_robust() {
        assert_eq!(super::parse_ballot("2", 3, 1), Some(2));
        assert_eq!(super::parse_ballot("1", 3, 1), None); // own
        assert_eq!(super::parse_ballot("9", 3, 1), None); // out of bounds
    }

    #[tokio::test]
    async fn verdict_unanimous_skips_ballot_normal() {
        let mut s = CouncilSession::new(
            "Pick?",
            CouncilSessionMode::Debate,
            vec![
                seat_seq("a", "Alpha", vec!["CHOICE: Redis\nPOSITION: best"]),
                seat_seq("b", "Beta", vec!["CHOICE: redis\nPOSITION: agree"]),
            ],
        );
        s.start();
        let outcome = s.trigger_verdict().await.unwrap();
        assert!(outcome.unanimous);
        assert!(s.is_concluded());
    }

    #[tokio::test]
    async fn verdict_divergent_runs_ballot_robust() {
        // Distinct choices; each votes for #1 (Redis) except a, who can't.
        let mut s = CouncilSession::new(
            "Pick?",
            CouncilSessionMode::Debate,
            vec![
                seat_seq("a", "Alpha", vec!["CHOICE: Redis\nPOSITION: p", "2"]),
                seat_seq("b", "Beta", vec!["CHOICE: Postgres\nPOSITION: p", "1"]),
                seat_seq("c", "Gamma", vec!["CHOICE: SQLite\nPOSITION: p", "1"]),
            ],
        );
        s.start();
        let outcome = s.trigger_verdict().await.unwrap();
        assert!(!outcome.unanimous);
        assert_eq!(outcome.choice, "Redis");
        assert!(!outcome.tie);
    }

    #[tokio::test]
    async fn verdict_tie_is_stored_and_break_tie_resolves_robust() {
        // Two seats, two distinct choices, each forced to vote for the other →
        // 1-1 tie. The pending tie must be stored so `break_tie` can resolve it.
        let mut s = CouncilSession::new(
            "Pick?",
            CouncilSessionMode::Debate,
            vec![
                seat_seq("a", "Alpha", vec!["CHOICE: Redis\nPOSITION: p", "2"]),
                seat_seq("b", "Beta", vec!["CHOICE: Postgres\nPOSITION: p", "1"]),
            ],
        );
        s.start();
        let outcome = s.trigger_verdict().await.unwrap();
        assert!(outcome.tie, "1-1 ballot is a tie");
        assert!(s.verdict.is_some(), "pending tie stored on the session");
        assert!(
            !s.is_concluded(),
            "session stays open until the tie is broken"
        );
        let resolved = s.break_tie("b").expect("break_tie resolves a pending tie");
        assert_eq!(resolved.choice, "Postgres");
        assert!(s.is_concluded());
    }

    #[test]
    fn verdict_blocked_by_open_flag_normal() {
        let mut s = CouncilSession::new(
            "Q?",
            CouncilSessionMode::Debate,
            vec![seat("a", "Alpha", "x"), seat("b", "Beta", "y")],
        );
        s.add_flag("dubious number", "a", Some("b".into()));
        assert!(s.verdict_blocked_by_flags());
    }
}
