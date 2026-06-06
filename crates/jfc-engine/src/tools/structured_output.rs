//! StructuredOutput validation — JSON Schema validation for subagent outputs.
//!
//! Mirrors Claude Code's `StructuredOutput` tool. When a subagent is spawned
//! with a `schema` parameter, the schema is installed as a tokio task-local so
//! the subagent's `StructuredOutput` tool call can validate against it.
//!
//! # DSPy Assertions on the retry path
//!
//! A `StructuredOutput` call is one *attempt* in the sense of
//! [`jfc_core::run_with_assertions`]; the agent's *next turn* is the retry. The
//! DSPy-Assertions finding (arXiv:2312.13382) is that a bare "validation failed"
//! error makes that retry flail, but a structured, **actionable** feedback
//! message — what failed and how to fix it — drives JSON validity sharply up
//! (37.6% → 98.8% in the paper). [`schema_outcome`] classifies a payload into a
//! [`jfc_core::AssertionOutcome`] and [`format_retry_feedback`] renders a hard
//! violation as that actionable guidance, so the failure the model sees tells it
//! exactly which fields to fix on the next attempt.

use jfc_core::AssertionOutcome;
use jsonschema::Validator;
use serde_json::Value;
use std::sync::Arc;

tokio::task_local! {
    /// Active schema validator for the current task/subagent. Installed via
    /// [`with_schema`]'s task-local scope. This used to be a `thread_local!`
    /// with imperative set/clear around an `.await` — broken under tokio's
    /// work-stealing scheduler (the task can resume on a different worker
    /// thread, and the stale value leaks to unrelated tasks on the original
    /// thread). A task-local scope travels with the task.
    static ACTIVE_SCHEMA: Option<Arc<Validator>>;
}

/// Compile a JSON Schema into a reusable validator, rejecting malformed
/// schemas up front (before any subagent work runs).
pub fn compile_schema(schema: &Value) -> Result<Arc<Validator>, String> {
    Validator::new(schema)
        .map(Arc::new)
        .map_err(|e| format!("invalid JSON Schema: {e}"))
}

/// Run `fut` with `validator` installed as the task-local active schema.
/// `validate_output` calls anywhere inside the future (including after
/// `.await` migrations across worker threads) see this schema; tasks outside
/// the scope see none.
pub async fn with_schema<F>(validator: Option<Arc<Validator>>, fut: F) -> F::Output
where
    F: std::future::Future,
{
    ACTIVE_SCHEMA.scope(validator, fut).await
}

/// Validate a StructuredOutput payload against the active schema (if any).
/// Returns `Ok(())` when no schema is active OR the data matches.
/// Returns `Err(messages)` listing every validation error.
pub fn validate_output(data: &Value) -> Result<(), String> {
    let validator = ACTIVE_SCHEMA
        .try_with(|v| v.clone())
        .ok()
        .flatten();
    let Some(validator) = validator else {
        return Ok(());
    };
    let errors: Vec<String> = validator
        .iter_errors(data)
        .map(|e| {
            let path = e.instance_path().to_string();
            let loc = if path.is_empty() {
                "root".to_string()
            } else {
                path
            };
            format!("{loc}: {e}")
        })
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// Classify a `StructuredOutput` payload as a [`jfc_core::AssertionOutcome`].
///
/// - non-object → `Hard` (the tool contract requires a JSON object)
/// - schema mismatch → `Hard` carrying the joined validation errors
/// - matches (or no active schema) → `Pass`
///
/// This is the assertion the agent's per-turn attempt is checked against; the
/// `Hard` message is what [`format_retry_feedback`] turns into retry guidance.
pub fn schema_outcome(data: &Value) -> AssertionOutcome {
    if !data.is_object() {
        return AssertionOutcome::Hard {
            msg: "the value must be a JSON object (got a non-object)".to_string(),
        };
    }
    match validate_output(data) {
        Ok(()) => AssertionOutcome::Pass,
        Err(errors) => AssertionOutcome::Hard { msg: errors },
    }
}

/// Render an [`AssertionOutcome`] as the tool's textual result body.
///
/// On a hard violation this produces DSPy-style *actionable* feedback — it
/// names the failure and instructs the model to re-emit a corrected
/// `StructuredOutput` — rather than a bare error string, so the next-turn retry
/// converges instead of guessing. Returns `None` for a passing outcome (the
/// caller renders its own success body).
pub fn format_retry_feedback(outcome: &AssertionOutcome) -> Option<String> {
    match outcome {
        AssertionOutcome::Pass => None,
        AssertionOutcome::Soft { msg } => Some(format!("Note (non-blocking): {msg}")),
        AssertionOutcome::Hard { msg } => Some(format!(
            "Output does not satisfy the required schema:\n  {msg}\n\n\
             Fix the field(s) named above and call StructuredOutput again with \
             the corrected JSON object — do not change any field that already \
             validated."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn no_schema_means_pass_normal() {
        // Outside any with_schema scope there is no active schema.
        assert!(validate_output(&json!({"x": 1})).is_ok());
    }

    #[test]
    fn matching_data_passes_normal() {
        let schema = json!({
            "type": "object",
            "properties": {"name": {"type": "string"}},
            "required": ["name"]
        });
        let v = compile_schema(&schema).unwrap();
        ACTIVE_SCHEMA.sync_scope(Some(v), || {
            assert!(validate_output(&json!({"name": "x"})).is_ok());
        });
    }

    #[test]
    fn missing_required_field_fails_robust() {
        let schema = json!({
            "type": "object",
            "properties": {"name": {"type": "string"}},
            "required": ["name"]
        });
        let v = compile_schema(&schema).unwrap();
        ACTIVE_SCHEMA.sync_scope(Some(v), || {
            let err = validate_output(&json!({"other": 1})).expect_err("should fail");
            assert!(err.contains("name") || err.contains("required"));
        });
    }

    #[test]
    fn wrong_type_fails_robust() {
        let schema = json!({
            "type": "object",
            "properties": {"n": {"type": "integer"}},
            "required": ["n"]
        });
        let v = compile_schema(&schema).unwrap();
        ACTIVE_SCHEMA.sync_scope(Some(v), || {
            assert!(validate_output(&json!({"n": "not an integer"})).is_err());
        });
    }

    #[test]
    fn malformed_schema_returns_error_robust() {
        let bad = json!({"type": "not-a-real-type"});
        assert!(compile_schema(&bad).is_err());
    }

    // DSPy: a non-object payload is a hard assertion violation.
    #[test]
    fn schema_outcome_non_object_is_hard_normal() {
        let outcome = schema_outcome(&json!("just a string"));
        assert!(matches!(outcome, AssertionOutcome::Hard { .. }));
    }

    // DSPy: a schema mismatch is a hard violation carrying the field errors,
    // and the rendered feedback is actionable (names the field + says retry).
    #[test]
    fn schema_outcome_mismatch_yields_actionable_feedback_normal() {
        let schema = json!({
            "type": "object",
            "properties": {"name": {"type": "string"}},
            "required": ["name"]
        });
        let v = compile_schema(&schema).unwrap();
        ACTIVE_SCHEMA.sync_scope(Some(v), || {
            let outcome = schema_outcome(&json!({"other": 1}));
            assert!(matches!(outcome, AssertionOutcome::Hard { .. }));
            let feedback = format_retry_feedback(&outcome).expect("hard → feedback");
            assert!(feedback.contains("name") || feedback.contains("required"));
            assert!(feedback.contains("StructuredOutput again")); // actionable retry instruction
        });
    }

    // DSPy: a matching payload passes and produces no retry feedback.
    #[test]
    fn schema_outcome_match_passes_robust() {
        let schema = json!({
            "type": "object",
            "properties": {"name": {"type": "string"}},
            "required": ["name"]
        });
        let v = compile_schema(&schema).unwrap();
        ACTIVE_SCHEMA.sync_scope(Some(v), || {
            let outcome = schema_outcome(&json!({"name": "ok"}));
            assert!(matches!(outcome, AssertionOutcome::Pass));
            assert!(format_retry_feedback(&outcome).is_none());
        });
    }
}
