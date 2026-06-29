//! Lossless token-sequence delta encoding.
//!
//! This module mirrors `rcoq-tests/theorems/DiffCompression.v`: a common-prefix
//! diff, exact apply/round-trip reconstruction, bounded diff size, LCS
//! similarity, repeated-context savings, sliding-window matching, and hunk
//! selection under a unique-content budget.

pub type TokenSeq = Vec<u64>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffOp {
    Keep { offset: usize, len: usize },
    Insert(TokenSeq),
}

pub type DiffScript = Vec<DiffOp>;

pub fn diff_size(script: &[DiffOp]) -> usize {
    script
        .iter()
        .map(|op| match op {
            DiffOp::Keep { .. } => 2,
            DiffOp::Insert(tokens) => 1 + tokens.len(),
        })
        .sum()
}

pub fn apply_diff(base: &[u64], script: &[DiffOp]) -> TokenSeq {
    let mut out = Vec::new();
    for op in script {
        match op {
            DiffOp::Keep { offset, len } => {
                out.extend(base.iter().skip(*offset).take(*len).copied());
            }
            DiffOp::Insert(tokens) => out.extend(tokens.iter().copied()),
        }
    }
    out
}

pub fn common_prefix_len(left: &[u64], right: &[u64]) -> usize {
    left.iter()
        .zip(right.iter())
        .take_while(|(left, right)| left == right)
        .count()
}

pub fn compute_diff(base: &[u64], target: &[u64]) -> DiffScript {
    let prefix = common_prefix_len(base, target);
    vec![
        DiffOp::Keep {
            offset: 0,
            len: prefix,
        },
        DiffOp::Insert(target[prefix..].to_vec()),
    ]
}

pub fn lcs_length(left: &[u64], right: &[u64]) -> usize {
    let mut dp = vec![vec![0usize; right.len() + 1]; left.len() + 1];
    for i in (0..left.len()).rev() {
        for j in (0..right.len()).rev() {
            dp[i][j] = if left[i] == right[j] {
                1 + dp[i + 1][j + 1]
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    dp[0][0]
}

pub fn similarity(left: &[u64], right: &[u64]) -> usize {
    let total = left.len() + right.len();
    if total == 0 {
        100
    } else {
        (lcs_length(left, right) * 200)
            .checked_div(total)
            .unwrap_or(100)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenMessage {
    pub role: u64,
    pub tokens: TokenSeq,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaConversation {
    pub base_message: TokenMessage,
    pub deltas: Vec<(usize, DiffScript)>,
}

pub fn delta_encode(base: TokenMessage, conversation: &[TokenMessage]) -> DeltaConversation {
    let deltas = conversation
        .iter()
        .enumerate()
        .map(|(i, message)| (i, compute_diff(&base.tokens, &message.tokens)))
        .collect();
    DeltaConversation {
        base_message: base,
        deltas,
    }
}

pub fn delta_conv_size(delta: &DeltaConversation) -> usize {
    delta.base_message.tokens.len()
        + delta
            .deltas
            .iter()
            .map(|(_, script)| diff_size(script))
            .sum::<usize>()
}

pub fn raw_conv_size(conversation: &[TokenMessage]) -> usize {
    conversation
        .iter()
        .map(|message| message.tokens.len())
        .sum()
}

pub fn decode_delta(delta: &DeltaConversation) -> Vec<TokenMessage> {
    delta
        .deltas
        .iter()
        .map(|(_, script)| TokenMessage {
            role: delta.base_message.role,
            tokens: apply_diff(&delta.base_message.tokens, script),
        })
        .collect()
}

pub fn count_occurrences(needle: &[u64], haystack: &[u64]) -> usize {
    if haystack.is_empty() {
        return 0;
    }
    (0..haystack.len())
        .filter(|&i| haystack[i..].starts_with(needle))
        .count()
}

pub fn repeat_savings(occurrences: usize, block_len: usize) -> usize {
    occurrences
        .saturating_sub(1)
        .saturating_mul(block_len.saturating_sub(2))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlidingWindow {
    pub window_size: usize,
    pub window_content: TokenSeq,
}

pub fn window_wf(window: &SlidingWindow) -> bool {
    window.window_content.len() <= window.window_size
}

pub fn find_match(needle: &[u64], window: &SlidingWindow) -> Option<(usize, usize)> {
    (0..window.window_content.len()).find_map(|offset| {
        if window.window_content[offset..].starts_with(needle) {
            Some((offset, needle.len()))
        } else {
            None
        }
    })
}

pub fn preprocess_savings(base: &[u64], messages: &[TokenSeq]) -> usize {
    messages
        .iter()
        .map(|message| common_prefix_len(base, message))
        .sum()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub start: usize,
    pub length: usize,
    pub content: TokenSeq,
    pub is_common: bool,
}

pub fn select_hunks(hunks: &[Hunk], budget: usize) -> Vec<Hunk> {
    let mut selected: Vec<_> = hunks
        .iter()
        .filter(|hunk| hunk.is_common)
        .cloned()
        .collect();
    selected.extend(
        hunks
            .iter()
            .filter(|hunk| !hunk.is_common)
            .take(budget)
            .cloned(),
    );
    selected
}

pub fn semantic_lcs_length(left: &[u64], right: &[u64]) -> usize {
    lcs_length(left, right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computed_diff_round_trips_target() {
        let base = vec![1, 2, 3, 4];
        let target = vec![1, 2, 9, 10];
        let script = compute_diff(&base, &target);
        assert_eq!(apply_diff(&base, &script), target);
        assert!(diff_size(&script) <= 3 + target.len());
    }

    #[test]
    fn self_diff_has_identity_shape() {
        let base = vec![1, 2, 3];
        assert_eq!(
            compute_diff(&base, &base),
            vec![
                DiffOp::Keep { offset: 0, len: 3 },
                DiffOp::Insert(Vec::new())
            ]
        );
    }

    #[test]
    fn overlap_beats_full_insert_after_break_even() {
        let base = vec![1, 2, 3, 4, 5];
        let target = vec![1, 2, 3, 9, 10];
        assert!(common_prefix_len(&base, &target) >= 3);
        assert!(diff_size(&compute_diff(&base, &target)) <= diff_size(&[DiffOp::Insert(target)]));
    }

    #[test]
    fn similarity_is_bounded_by_100() {
        assert!(similarity(&[1, 2, 3], &[1, 4, 3]) <= 100);
        assert_eq!(similarity(&[], &[]), 100);
    }

    #[test]
    fn delta_encode_is_faithful_and_size_bounded() {
        let base = TokenMessage {
            role: 0,
            tokens: vec![1, 2, 3],
        };
        let conversation = vec![
            TokenMessage {
                role: 0,
                tokens: vec![1, 2, 3],
            },
            TokenMessage {
                role: 0,
                tokens: vec![1, 2, 9],
            },
        ];
        let delta = delta_encode(base.clone(), &conversation);
        let decoded_tokens: Vec<_> = decode_delta(&delta)
            .into_iter()
            .map(|message| message.tokens)
            .collect();
        assert_eq!(
            decoded_tokens,
            conversation
                .iter()
                .map(|message| message.tokens.clone())
                .collect::<Vec<_>>()
        );
        assert!(
            delta_conv_size(&delta)
                <= base.tokens.len() + raw_conv_size(&conversation) + 3 * conversation.len()
        );
    }

    #[test]
    fn repeated_context_has_positive_savings_past_keep_overhead() {
        let haystack = vec![1, 2, 3, 9, 1, 2, 3];
        let needle = vec![1, 2, 3];
        let occurrences = count_occurrences(&needle, &haystack);
        assert!(occurrences >= 2);
        assert!(repeat_savings(occurrences, needle.len()) >= 1);
    }

    #[test]
    fn sliding_window_match_reconstructs_needle() {
        let window = SlidingWindow {
            window_size: 5,
            window_content: vec![9, 1, 2, 3, 8],
        };
        assert!(window_wf(&window));
        let (offset, len) = find_match(&[1, 2, 3], &window).unwrap();
        assert_eq!(
            window
                .window_content
                .iter()
                .skip(offset)
                .take(len)
                .copied()
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn selected_hunks_respect_unique_budget() {
        let hunks = vec![
            Hunk {
                start: 0,
                length: 2,
                content: vec![1, 2],
                is_common: true,
            },
            Hunk {
                start: 2,
                length: 1,
                content: vec![9],
                is_common: false,
            },
            Hunk {
                start: 3,
                length: 1,
                content: vec![10],
                is_common: false,
            },
        ];
        let selected = select_hunks(&hunks, 1);
        assert_eq!(selected.iter().filter(|hunk| !hunk.is_common).count(), 1);
    }

    #[test]
    fn semantic_lcs_finds_at_least_syntactic_content() {
        let left = vec![1, 2, 3];
        let right = vec![2, 3, 4];
        assert!(lcs_length(&left, &right) <= semantic_lcs_length(&left, &right));
    }
}
