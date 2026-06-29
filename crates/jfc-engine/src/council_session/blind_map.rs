use jfc_provider::{ProviderContent, ProviderMessage, ProviderRole, StreamOptions};
use serde::Serialize;

use super::{CouncilSession, SessionPhase, TranscriptEntry};

const CLASSIFIER_PROMPT: &str = "You classify whether this task warrants multi-model blind map-reduce. Reply exactly:\nVERDICT: HIGH or LOW\nWHY: <one short sentence>";
const BLIND_MAP_PROMPT: &str = "You are answering independently. You cannot see any other model's answer. Structure exactly:\n## Answer\n<direct answer>\n\n## Self-critique\n- <weakness one>\n- <weakness two>\n\n## Confidence\n<tag main claims High/Medium/Low>";
const REDTEAM_PROMPT: &str = "You are the red-team reviewer. Judge only the labelled blind answers. Do not pick a winner. Produce exactly:\n## Contradictions\n<direct disagreements>\n\n## Shared blind spots\n<what all answers may miss>\n\n## Overconfident or unsourced claims\n<specific risky claims>";
const SYNTH_PROMPT: &str = "You are the synthesizer. Produce exactly:\n## Final answer\n<best-supported answer>\n\n## Dissent ledger\n<unresolved disagreements and minority positions>\n\n## What changed and why\n<what survived, changed, or was discarded>";
const BASELINE_PROMPT: &str =
    "Answer the question directly and completely as a solo baseline, with no blind cross-check.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BlindMapReport {
    pub blind_answer_count: usize,
    pub redteam_ran: bool,
    pub baseline_ran: bool,
}

struct BlindAnswer {
    label: char,
    content: String,
}

impl CouncilSession {
    pub async fn run_blind_map_reduce(
        &mut self,
        include_baseline: bool,
    ) -> anyhow::Result<BlindMapReport> {
        let active: Vec<String> = self.active_seats().map(|s| s.id.clone()).collect();
        if active.is_empty() {
            return Err(anyhow::anyhow!(
                "blind map-reduce needs at least one participant"
            ));
        }
        self.phase = SessionPhase::Debating;
        self.transcript.push(TranscriptEntry::system(
            self.round,
            format!(
                "BLIND MAP-REDUCE\n\n{} independent answerer(s), then red-team, synthesis, and optional baseline.",
                active.len()
            ),
        ));

        self.run_classifier(&active[0]).await;
        let answers = self.run_blind_answers(&active).await;
        if answers.is_empty() {
            return Err(anyhow::anyhow!("no blind answers completed"));
        }
        let redteam = self.run_redteam(&active, &answers).await;
        self.run_synthesis(&active[0], &answers, redteam.as_deref())
            .await?;
        let baseline_ran = include_baseline && self.run_baseline(&active[0]).await;
        self.phase = SessionPhase::Concluded;
        self.current_speaker = None;
        Ok(BlindMapReport {
            blind_answer_count: answers.len(),
            redteam_ran: redteam.is_some(),
            baseline_ran,
        })
    }

    async fn run_classifier(&mut self, seat_id: &str) {
        let user = format!("Classify this task:\n\n{}", self.topic);
        let content = self
            .call_isolated(seat_id, CLASSIFIER_PROMPT, &user, 256)
            .await
            .unwrap_or_else(|e| {
                format!("VERDICT: HIGH\nWHY: classifier failed ({e}); running full pipeline.")
            });
        self.transcript.push(TranscriptEntry::system(
            self.round,
            format!(
                "Phase 0 · Task classifier · {}\n\n{content}",
                self.seat_name(seat_id)
            ),
        ));
    }

    async fn run_blind_answers(&mut self, active: &[String]) -> Vec<BlindAnswer> {
        let mut answers = Vec::with_capacity(active.len());
        let topic = self.topic.clone();
        for (idx, seat_id) in active.iter().enumerate() {
            let label = answer_label(idx);
            let user = format!("Question:\n\n{topic}");
            match self
                .call_isolated(seat_id, BLIND_MAP_PROMPT, &user, self.max_tokens)
                .await
            {
                Ok(content) => {
                    self.transcript.push(TranscriptEntry::system(
                        self.round,
                        format!(
                            "Phase 1 · Blind answer {label} · {}\n\n{content}",
                            self.seat_name(seat_id)
                        ),
                    ));
                    answers.push(BlindAnswer { label, content });
                }
                Err(e) => self.transcript.push(TranscriptEntry::system(
                    self.round,
                    format!(
                        "Phase 1 · Blind answer {label} · {} failed: {e}",
                        self.seat_name(seat_id)
                    ),
                )),
            }
        }
        answers
    }

    async fn run_redteam(&mut self, active: &[String], answers: &[BlindAnswer]) -> Option<String> {
        if answers.len() < 2 {
            self.transcript.push(TranscriptEntry::system(
                self.round,
                "Phase 2 · Red-team skipped — only one blind answer completed.",
            ));
            return None;
        }
        let redteam_id = active.get(1).unwrap_or(&active[0]);
        let content = self
            .call_isolated(
                redteam_id,
                REDTEAM_PROMPT,
                &format_answer_block(answers),
                self.max_tokens,
            )
            .await
            .ok()?;
        self.transcript.push(TranscriptEntry::system(
            self.round,
            format!(
                "Phase 2 · Red-team report · {}\n\n{content}",
                self.seat_name(redteam_id)
            ),
        ));
        Some(content)
    }

    async fn run_synthesis(
        &mut self,
        synth_id: &str,
        answers: &[BlindAnswer],
        redteam: Option<&str>,
    ) -> anyhow::Result<()> {
        let input = match redteam {
            Some(report) => format!(
                "{}\n\n### Red-team Discrepancy Report\n{report}",
                format_answer_block(answers)
            ),
            None => format_answer_block(answers),
        };
        let content = self
            .call_isolated(synth_id, SYNTH_PROMPT, &input, self.max_tokens)
            .await?;
        self.transcript.push(TranscriptEntry::system(
            self.round,
            format!(
                "✦ Final answer · synthesized by {}\n\n{content}",
                self.seat_name(synth_id)
            ),
        ));
        Ok(())
    }

    async fn run_baseline(&mut self, baseline_id: &str) -> bool {
        let user = format!("Question:\n\n{}", self.topic);
        let Ok(content) = self
            .call_isolated(baseline_id, BASELINE_PROMPT, &user, self.max_tokens)
            .await
        else {
            return false;
        };
        self.transcript.push(TranscriptEntry::system(
            self.round,
            format!(
                "Baseline · solo {}\n\n{content}",
                self.seat_name(baseline_id)
            ),
        ));
        true
    }

    async fn call_isolated(
        &mut self,
        seat_id: &str,
        system: &str,
        user: &str,
        max_tokens: u32,
    ) -> anyhow::Result<String> {
        let (provider, model) = {
            let seat = self
                .seat(seat_id)
                .ok_or_else(|| anyhow::anyhow!("unknown blind-map seat {seat_id}"))?;
            (seat.provider.clone(), seat.model.clone())
        };
        let messages = vec![ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(user.to_owned())],
        }];
        let opts = StreamOptions::new(model)
            .system(system.to_owned())
            .max_tokens(max_tokens);
        let resp =
            crate::prompt_executor::complete_once(provider.as_ref(), messages, &opts).await?;
        let used = resp
            .usage
            .billable_tokens(user.len() + resp.content.len())
            .0;
        if let Some(seat) = self.seat_mut(seat_id) {
            seat.tokens_used = seat.tokens_used.saturating_add(used);
        }
        Ok(resp.content)
    }
}

fn format_answer_block(answers: &[BlindAnswer]) -> String {
    answers
        .iter()
        .map(|a| format!("### Answer {}\n{}", a.label, a.content))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn answer_label(index: usize) -> char {
    const LABELS: &[u8; 26] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    char::from(LABELS[index.min(LABELS.len() - 1)])
}
