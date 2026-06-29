//! DSPy Assertions — runtime constraints with assertion-driven backtracking.
//!
//! From *DSPy Assertions: Computational Constraints for Self-Refining Language
//! Model Pipelines* (arXiv:2312.13382): an LM step's output is wrapped in
//! `assert` (hard) / `suggest` (soft) constraints. On a *hard* failure the
//! framework **backtracks** — it re-runs the step with the failed output and
//! the violation message injected as feedback — up to a retry cap. The paper
//! reports JSON-validity rising 37.6% → 98.8% on one task purely from this
//! loop.
//!
//! This module is the deterministic control core of that loop for jfc, a
//! natural extension of the [`crate::routing`] / teleprompter machinery. The
//! actual production of a value (in jfc, an LLM tool/agent call) is supplied by
//! the caller as the `attempt` closure: it receives `Some(feedback)` on a retry
//! and `None` on the first try. Constraints are [`Assertion`]s evaluated against
//! each produced value; the policy here — classify each outcome, on any hard
//! violation re-prompt with the concatenated messages, accept soft violations,
//! stop at the cap — is pure and fully tested.

/// The outcome of evaluating one [`Assertion`] against a produced value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssertionOutcome {
    /// The constraint held.
    Pass,
    /// A *soft* constraint (DSPy `suggest`) was violated: recorded as advisory
    /// but does not trigger a retry or fail the run.
    Soft { msg: String },
    /// A *hard* constraint (DSPy `assert`) was violated: triggers a backtrack /
    /// retry with `msg` fed back into the next attempt.
    Hard { msg: String },
}

/// A single constraint on a produced value of type `T`. Any `Fn(&T) ->
/// AssertionOutcome` is an `Assertion` via the blanket impl below, so callers
/// can pass closures directly.
pub trait Assertion<T> {
    fn check(&self, value: &T) -> AssertionOutcome;
}

impl<T, F: Fn(&T) -> AssertionOutcome> Assertion<T> for F {
    fn check(&self, value: &T) -> AssertionOutcome {
        self(value)
    }
}

/// The record of one assertion-guided run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssertionRun<T> {
    /// The final produced value (the last attempt, whether or not every hard
    /// constraint passed).
    pub value: T,
    /// True iff the final value satisfied every hard constraint.
    pub passed: bool,
    /// Soft-constraint messages observed on the final attempt (advisory).
    pub soft_violations: Vec<String>,
    /// Hard-constraint messages from the final attempt — empty iff `passed`.
    pub hard_violations: Vec<String>,
    /// Number of attempts made (1 = passed/settled on the first try).
    pub attempts: u32,
}

/// Run `attempt` under `assertions`, backtracking on hard violations.
///
/// `attempt(feedback)` produces a value; `feedback` is `None` on the first try
/// and `Some(joined_messages)` on each retry. After each attempt every
/// assertion is evaluated:
/// - no hard violations → return immediately with `passed = true` (soft
///   violations are carried through but accepted);
/// - any hard violation → if retries remain, re-invoke `attempt` with the hard
///   messages joined by `"; "` as feedback; otherwise return the last value
///   with `passed = false`.
///
/// `max_retries` is the number of *additional* attempts after the first, so the
/// driver makes at most `max_retries + 1` calls to `attempt`.
pub fn run_with_assertions<T>(
    max_retries: u32,
    mut attempt: impl FnMut(Option<&str>) -> T,
    assertions: &[Box<dyn Assertion<T>>],
) -> AssertionRun<T> {
    let mut feedback: Option<String> = None;
    let mut attempts = 0u32;
    loop {
        attempts += 1;
        let value = attempt(feedback.as_deref());

        let mut soft = Vec::new();
        let mut hard = Vec::new();
        for a in assertions {
            match a.check(&value) {
                AssertionOutcome::Pass => {}
                AssertionOutcome::Soft { msg } => soft.push(msg),
                AssertionOutcome::Hard { msg } => hard.push(msg),
            }
        }

        if hard.is_empty() || attempts > max_retries {
            return AssertionRun {
                value,
                passed: hard.is_empty(),
                soft_violations: soft,
                hard_violations: hard,
                attempts,
            };
        }
        feedback = Some(hard.join("; "));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hard_unless<F: Fn(&String) -> bool + 'static>(
        msg: &'static str,
        ok: F,
    ) -> Box<dyn Assertion<String>> {
        Box::new(move |v: &String| {
            if ok(v) {
                AssertionOutcome::Pass
            } else {
                AssertionOutcome::Hard {
                    msg: msg.to_string(),
                }
            }
        })
    }

    // Normal: a clean first attempt passes with no retries.
    #[test]
    fn passes_on_first_try_normal() {
        let ok: Box<dyn Assertion<i32>> = Box::new(|_: &i32| AssertionOutcome::Pass);
        let run = run_with_assertions(3, |_| 42, &[ok]);
        assert!(run.passed);
        assert_eq!(run.attempts, 1);
        assert_eq!(run.value, 42);
        assert!(run.hard_violations.is_empty());
    }

    // Normal: a hard violation triggers a retry; once feedback is supplied the
    // attempt produces a passing value.
    #[test]
    fn retries_until_feedback_makes_it_pass_normal() {
        let run = run_with_assertions(
            3,
            |fb| {
                if fb.is_some() {
                    "good".to_string()
                } else {
                    "bad".to_string()
                }
            },
            &[hard_unless("must be good", |v| v == "good")],
        );
        assert!(run.passed);
        assert_eq!(run.value, "good");
        assert_eq!(run.attempts, 2); // first fails, retry-with-feedback passes
        assert!(run.hard_violations.is_empty());
    }

    // Robust: an attempt that never passes exhausts the retry cap and reports
    // failure — exactly `max_retries + 1` invocations.
    #[test]
    fn exhausts_retry_cap_and_reports_failure_robust() {
        let always_fail: Box<dyn Assertion<i32>> =
            Box::new(|_: &i32| AssertionOutcome::Hard { msg: "nope".into() });
        let mut calls = 0;
        let run = run_with_assertions(
            2,
            |_| {
                calls += 1;
                0
            },
            &[always_fail],
        );
        assert!(!run.passed);
        assert_eq!(run.attempts, 3); // 1 initial + 2 retries
        assert_eq!(calls, 3);
        assert_eq!(run.hard_violations, vec!["nope".to_string()]);
    }

    // Robust: a soft violation is recorded but accepted — no retry, run passes.
    #[test]
    fn soft_violation_is_accepted_not_retried_robust() {
        let soft_only: Box<dyn Assertion<String>> = Box::new(|_: &String| AssertionOutcome::Soft {
            msg: "style nit".into(),
        });
        let mut calls = 0;
        let run = run_with_assertions(
            5,
            |_| {
                calls += 1;
                "whatever".to_string()
            },
            &[soft_only],
        );
        assert!(run.passed);
        assert_eq!(run.attempts, 1);
        assert_eq!(calls, 1);
        assert_eq!(run.soft_violations, vec!["style nit".to_string()]);
        assert!(run.hard_violations.is_empty());
    }

    // Robust: messages from several failing hard assertions are concatenated
    // into a single feedback string for the next attempt.
    #[test]
    fn combines_multiple_hard_messages_into_feedback_robust() {
        let mut seen_feedback: Option<String> = None;
        let run = run_with_assertions(
            2,
            |fb| {
                if let Some(f) = fb {
                    seen_feedback = Some(f.to_string());
                    "fixed".to_string()
                } else {
                    "raw".to_string()
                }
            },
            &[
                hard_unless("rule-a", |v| v.contains("fixed")),
                hard_unless("rule-b", |v| v.contains("fixed")),
            ],
        );
        assert!(run.passed);
        assert_eq!(seen_feedback.as_deref(), Some("rule-a; rule-b"));
    }

    // Robust: `max_retries = 0` means a single attempt, no backtracking.
    #[test]
    fn zero_retries_is_single_attempt_robust() {
        let mut calls = 0;
        let run = run_with_assertions(
            0,
            |_| {
                calls += 1;
                "x".to_string()
            },
            &[hard_unless("never", |_| false)],
        );
        assert!(!run.passed);
        assert_eq!(calls, 1);
        assert_eq!(run.attempts, 1);
    }
}
