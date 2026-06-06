//! Offline trajectory compression for eval/training corpora.
//!
//! Mirrors hermes-agent's `trajectory_compressor.py`: a finished agent run is a
//! long list of turns (user / assistant / tool-call / tool-result). For
//! *offline* use — building eval sets, distillation corpora, replaying a run —
//! the full middle is rarely needed: the **opening** (task + first moves) and
//! the **ending** (resolution + outcome) carry the training signal, while the
//! long middle of tool churn can be folded into one summary turn.
//!
//! This is deliberately **distinct from runtime compaction**
//! ([`jfc_core::compaction`] / the jfc live loop): that one runs *during* a
//! session under a live window and must preserve the reasoning trace for the
//! next model call. This one runs *after* a session is complete, optimising a
//! stored trajectory for size while protecting a head and tail span the caller
//! names explicitly. The summarisation of the collapsed middle is injected as a
//! closure (an LLM call in production), so the selection policy — token
//! accounting, the protect-bounds, where the middle starts and ends — is pure
//! and fully tested.

/// One turn in a trajectory. `role` is free-form (`"user"`, `"assistant"`,
/// `"tool"`, …); `content` is the rendered text whose length drives the token
/// estimate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub role: String,
    pub content: String,
}

impl Turn {
    pub fn new(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
        }
    }

    /// Token estimate for this turn: `ceil(chars / 4)`, the same cheap rule of
    /// thumb used across jfc.
    pub fn est_tokens(&self) -> u64 {
        (self.content.chars().count() as u64).div_ceil(4)
    }
}

/// Total estimated tokens across a slice of turns.
pub fn total_tokens(turns: &[Turn]) -> u64 {
    turns.iter().map(Turn::est_tokens).sum()
}

/// Compress a finished trajectory to fit `budget_tokens`.
///
/// Keeps the first `protect_first` and last `protect_last` turns verbatim. If
/// the whole trajectory already fits the budget — or there is nothing between
/// the protected head and tail to collapse — it is returned unchanged. When it
/// is over budget and a collapsible middle exists, that middle span is replaced
/// by a single summary turn produced by `summarize`, yielding
/// `head ++ [summary] ++ tail`.
///
/// The protected regions are clamped so the head and tail never overlap: if
/// `protect_first + protect_last >= turns.len()`, every turn is protected and
/// the input is returned unchanged (nothing to summarise).
///
/// Note the contract is *best-effort toward* the budget, not a hard guarantee:
/// if the protected head + tail alone exceed `budget_tokens`, they are still
/// kept verbatim (dropping protected turns would defeat the purpose) and only
/// the middle is collapsed. The summary turn uses role `"summary"`.
pub fn compress(
    turns: &[Turn],
    budget_tokens: u64,
    protect_first: usize,
    protect_last: usize,
    summarize: impl Fn(&[Turn]) -> Turn,
) -> Vec<Turn> {
    // Already within budget — nothing to do.
    if total_tokens(turns) <= budget_tokens {
        return turns.to_vec();
    }
    let n = turns.len();
    // Everything is protected (or the protected spans meet/overlap) — no middle
    // to collapse, so we can't shrink without violating the protect contract.
    if protect_first.saturating_add(protect_last) >= n {
        return turns.to_vec();
    }

    let head = &turns[..protect_first];
    let tail = &turns[n - protect_last..];
    let middle = &turns[protect_first..n - protect_last];

    // Defensive: middle is non-empty here because protect_first+protect_last < n.
    debug_assert!(!middle.is_empty());

    let mut out: Vec<Turn> = Vec::with_capacity(protect_first + 1 + protect_last);
    out.extend_from_slice(head);
    out.push(summarize(middle));
    out.extend_from_slice(tail);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(role: &str, content: &str) -> Turn {
        Turn::new(role, content)
    }

    /// A summariser that records how many turns it folded, for assertions.
    fn count_summary(middle: &[Turn]) -> Turn {
        Turn::new("summary", format!("folded {} turns", middle.len()))
    }

    // Normal: a trajectory already under budget is returned unchanged.
    #[test]
    fn under_budget_is_noop_normal() {
        let traj = vec![turn("user", "hi"), turn("assistant", "hello")];
        let out = compress(&traj, 1_000, 1, 1, count_summary);
        assert_eq!(out, traj);
    }

    // Normal: an over-budget trajectory collapses its middle into one summary,
    // preserving the protected head and tail verbatim.
    #[test]
    fn middle_collapses_when_over_budget_normal() {
        let traj = vec![
            turn("user", "the task description here"),
            turn("assistant", "tool churn one ......................"),
            turn("tool", "result one ........................."),
            turn("assistant", "tool churn two ......................"),
            turn("tool", "result two ........................."),
            turn("assistant", "final answer here"),
        ];
        // Tiny budget forces compression; protect first 1 + last 1.
        let out = compress(&traj, 5, 1, 1, count_summary);
        assert_eq!(out.len(), 3); // head + summary + tail
        assert_eq!(out[0], traj[0]); // head verbatim
        assert_eq!(out[2], traj[traj.len() - 1]); // tail verbatim
        assert_eq!(out[1].role, "summary");
        assert_eq!(out[1].content, "folded 4 turns"); // the 4 middle turns
    }

    // Robust: when head+tail cover every turn, nothing is collapsed.
    #[test]
    fn all_protected_is_noop_robust() {
        let traj = vec![
            turn("user", "aaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            turn("assistant", "bbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
        ];
        // protect_first + protect_last == len -> everything protected.
        let out = compress(&traj, 1, 1, 1, count_summary);
        assert_eq!(out, traj);
    }

    // Robust: protect bounds larger than the trajectory don't panic and return
    // the input unchanged.
    #[test]
    fn oversized_protect_bounds_are_safe_robust() {
        let traj = vec![
            turn("user", "xxxxxxxxxxxxxxxxxxxx"),
            turn("assistant", "yyyy"),
        ];
        let out = compress(&traj, 1, 10, 10, count_summary);
        assert_eq!(out, traj);
    }

    // Robust: with zero protected turns, the whole trajectory folds into a
    // single summary when over budget.
    #[test]
    fn zero_protect_folds_everything_robust() {
        let traj = vec![
            turn("user", "aaaaaaaaaaaaaaaaaaaa"),
            turn("assistant", "bbbbbbbbbbbbbbbbbbbb"),
            turn("tool", "cccccccccccccccccccc"),
        ];
        let out = compress(&traj, 1, 0, 0, count_summary);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].role, "summary");
        assert_eq!(out[0].content, "folded 3 turns");
    }

    // Normal: the token estimate is ceil(chars / 4).
    #[test]
    fn est_tokens_is_ceil_quarter_normal() {
        assert_eq!(turn("user", "").est_tokens(), 0);
        assert_eq!(turn("user", "abcd").est_tokens(), 1);
        assert_eq!(turn("user", "abcde").est_tokens(), 2);
        assert_eq!(total_tokens(&[turn("a", "abcd"), turn("b", "abcd")]), 2);
    }

    // Robust: only the head and tail are kept verbatim; an over-budget run with
    // protect_first=2 protect_last=1 keeps exactly those.
    #[test]
    fn protect_bounds_keep_exact_head_and_tail_robust() {
        let traj: Vec<Turn> = (0..8)
            .map(|i| turn("assistant", &"z".repeat(40 + i)))
            .collect();
        let out = compress(&traj, 5, 2, 1, count_summary);
        assert_eq!(out.len(), 4); // 2 head + summary + 1 tail
        assert_eq!(out[0], traj[0]);
        assert_eq!(out[1], traj[1]);
        assert_eq!(out[2].role, "summary");
        assert_eq!(out[3], traj[7]);
    }
}
