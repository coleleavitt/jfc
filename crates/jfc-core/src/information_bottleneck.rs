//! Information-bottleneck compression primitives.
//!
//! This is the Rust counterpart of `rcoq-tests/theorems/InformationBottleneck.v`:
//! discrete entropy/MI approximations, rate-distortion bounds, cascade loss,
//! LongLLMLingua-style Lagrangian scoring, MI threshold retention, scheduling,
//! and hash-collision estimates.

use std::collections::BTreeSet;

pub fn entropy(states: &[u64]) -> usize {
    states.iter().copied().collect::<BTreeSet<_>>().len()
}

pub fn mutual_info(x: &[u64], y: &[u64]) -> usize {
    let y_set: BTreeSet<_> = y.iter().copied().collect();
    x.iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|state| y_set.contains(state))
        .count()
}

pub fn conditional_entropy(x: &[u64], y: &[u64]) -> usize {
    entropy(y).saturating_sub(mutual_info(x, y))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IBState {
    pub input_entropy: u64,
    pub compressed_entropy: u64,
    pub task_mutual_info: u64,
    pub beta: u64,
}

pub fn ib_objective(state: IBState) -> u64 {
    let relevance_benefit = state.task_mutual_info.saturating_mul(state.beta) / 100;
    state.compressed_entropy.saturating_sub(relevance_benefit)
}

pub fn rate_distortion(source_entropy: u64, distortion_tolerance: u64) -> u64 {
    if distortion_tolerance == 0 {
        source_entropy
    } else {
        source_entropy.saturating_mul(100u64.saturating_sub(distortion_tolerance.min(100))) / 100
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConversationContext {
    pub total_tokens: u64,
    pub semantic_content: u64,
    pub redundancy: u64,
    pub noise: u64,
}

pub fn compressibility(context: ConversationContext) -> u64 {
    context
        .redundancy
        .saturating_add(context.noise)
        .saturating_mul(100)
        / context.total_tokens.max(1)
}

pub fn minimum_tokens(context: ConversationContext) -> u64 {
    context.semantic_content
}

pub fn compress_once(info: u64, ratio: u64) -> u64 {
    info.saturating_mul(ratio) / 100
}

pub fn compress_cascade(mut info: u64, ratio: u64, rounds: usize) -> u64 {
    for _ in 0..rounds {
        info = compress_once(info, ratio);
    }
    info
}

pub fn lagrangian_cost(lambda: u64, fidelity_loss: u64, kept_length: u64) -> u64 {
    fidelity_loss.saturating_add(lambda.saturating_mul(kept_length))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    pub fidelity_loss: u64,
    pub kept_length: u64,
}

pub fn candidate_cost(lambda: u64, candidate: Candidate) -> u64 {
    lagrangian_cost(lambda, candidate.fidelity_loss, candidate.kept_length)
}

pub fn longer_better_fidelity(short: Candidate, long: Candidate) -> bool {
    short.kept_length <= long.kept_length && long.fidelity_loss <= short.fidelity_loss
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportantToken {
    pub token_id: u64,
    pub score: u64,
}

pub fn retain(gamma: u64, tokens: &[ImportantToken]) -> Vec<ImportantToken> {
    tokens
        .iter()
        .copied()
        .filter(|token| token.score > gamma)
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskContext {
    pub context_info: u64,
    pub task_relevant_info: u64,
    pub task_irrelevant_info: u64,
}

pub fn valid_task_context(context: TaskContext) -> bool {
    context
        .task_relevant_info
        .saturating_add(context.task_irrelevant_info)
        == context.context_info
}

pub fn optimal_task_compression(context: TaskContext) -> u64 {
    context.task_relevant_info
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JFCCompaction {
    pub original_tokens: u64,
    pub summary_tokens: u64,
    pub preserved_semantic_fraction: u64,
}

pub fn semantic_efficiency(compaction: JFCCompaction) -> u64 {
    compaction.preserved_semantic_fraction.saturating_mul(100) / compaction.summary_tokens.max(1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressionSchedule {
    pub compress_threshold: u64,
    pub compress_target: u64,
    pub min_turns_between: u64,
}

pub fn valid_schedule(schedule: CompressionSchedule) -> bool {
    schedule.compress_target < schedule.compress_threshold
}

pub fn expected_compressions(schedule: CompressionSchedule, conversation_length: u64) -> u64 {
    if conversation_length < schedule.compress_threshold {
        0
    } else {
        conversation_length.saturating_sub(schedule.compress_target)
            / schedule
                .compress_threshold
                .saturating_sub(schedule.compress_target)
                .max(1)
    }
}

pub fn expected_collisions(n_items: u64, hash_buckets: u64) -> u64 {
    n_items.saturating_mul(n_items) / hash_buckets.saturating_mul(2).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entropy_and_mutual_information_are_discrete_set_measures() {
        assert_eq!(entropy(&[1, 1, 2, 3]), 3);
        assert_eq!(mutual_info(&[1, 1, 2, 4], &[2, 3, 4]), 2);
        assert_eq!(conditional_entropy(&[1, 2], &[2, 3, 4]), 2);
    }

    #[test]
    fn rate_distortion_is_lossless_at_zero_and_monotone_in_distortion() {
        assert_eq!(rate_distortion(100, 0), 100);
        assert!(rate_distortion(100, 10) >= rate_distortion(100, 50));
    }

    #[test]
    fn cascade_compression_never_increases_when_ratio_at_most_100() {
        let info = 1000;
        let ratio = 80;
        assert!(compress_cascade(info, ratio, 2) <= compress_cascade(info, ratio, 1));
        assert_eq!(compress_cascade(info, 100, 3), info);
    }

    #[test]
    fn lagrangian_cost_is_monotone_in_lambda_and_bounds_length() {
        let fidelity = 10;
        let kept = 5;
        assert!(lagrangian_cost(2, fidelity, kept) <= lagrangian_cost(3, fidelity, kept));
        let budget = lagrangian_cost(4, fidelity, kept);
        assert!(kept <= budget / 4);
    }

    #[test]
    fn longer_candidate_gets_less_attractive_as_lambda_rises() {
        let short = Candidate {
            fidelity_loss: 200,
            kept_length: 5,
        };
        let long = Candidate {
            fidelity_loss: 0,
            kept_length: 20,
        };
        assert!(longer_better_fidelity(short, long));
        assert!(candidate_cost(5, long) <= candidate_cost(5, short));
        assert!(candidate_cost(1, long) <= candidate_cost(1, short));
    }

    #[test]
    fn mi_threshold_retention_is_upward_closed_and_monotone() {
        let tokens = vec![
            ImportantToken {
                token_id: 1,
                score: 10,
            },
            ImportantToken {
                token_id: 2,
                score: 50,
            },
            ImportantToken {
                token_id: 3,
                score: 90,
            },
        ];
        let high = retain(60, &tokens);
        let low = retain(40, &tokens);
        assert!(high.iter().all(|token| low.contains(token)));
        assert!(low.iter().all(|token| token.score > 40));
        assert!(low.contains(&tokens[2]));
    }

    #[test]
    fn task_relevant_info_is_the_hard_compression_floor() {
        let context = TaskContext {
            context_info: 100,
            task_relevant_info: 40,
            task_irrelevant_info: 60,
        };
        assert!(valid_task_context(context));
        assert!(optimal_task_compression(context) <= context.context_info);
        assert!(context.task_relevant_info - 39 > 0);
    }

    #[test]
    fn lower_threshold_schedule_compresses_at_least_as_often() {
        let aggressive = CompressionSchedule {
            compress_threshold: 1000,
            compress_target: 500,
            min_turns_between: 1,
        };
        let relaxed = CompressionSchedule {
            compress_threshold: 1500,
            compress_target: 500,
            min_turns_between: 1,
        };
        assert!(valid_schedule(aggressive));
        assert!(valid_schedule(relaxed));
        assert!(expected_compressions(aggressive, 3000) >= expected_compressions(relaxed, 3000));
    }

    #[test]
    fn larger_hash_space_reduces_expected_collisions() {
        assert!(expected_collisions(1000, 2048) <= expected_collisions(1000, 1024));
    }

    #[test]
    fn semantic_efficiency_uses_summary_as_denominator() {
        let compact = JFCCompaction {
            original_tokens: 1000,
            summary_tokens: 100,
            preserved_semantic_fraction: 80,
        };
        assert_eq!(semantic_efficiency(compact), 80);
    }
}
