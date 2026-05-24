//! StructuredOutput validation — JSON Schema validation for subagent outputs.
//!
//! Mirrors Claude Code's `StructuredOutput` tool. When a subagent is spawned
//! with a `schema` parameter, the schema is stored in thread-local state so
//! the subagent's `StructuredOutput` tool call can validate against it.

use jsonschema::Validator;
use serde_json::Value;
use std::cell::RefCell;
use std::sync::Arc;

thread_local! {
    /// Active schema validator for the current task/subagent.
    /// Set by `set_active_schema` before subagent execution, cleared after.
    static ACTIVE_SCHEMA: RefCell<Option<Arc<Validator>>> = const { RefCell::new(None) };
}

/// Install a schema for the current thread. Subsequent calls to
/// `validate_output` will check against it. Pass `None` to clear.
///
/// Returns an error if the provided schema is malformed.
pub fn set_active_schema(schema: Option<&Value>) -> Result<(), String> {
    let validator = match schema {
        Some(s) => {
            let v = Validator::new(s).map_err(|e| format!("invalid JSON Schema: {e}"))?;
            Some(Arc::new(v))
        }
        None => None,
    };
    ACTIVE_SCHEMA.with(|cell| {
        *cell.borrow_mut() = validator;
    });
    Ok(())
}

/// Validate a StructuredOutput payload against the active schema (if any).
/// Returns `Ok(())` when no schema is active OR the data matches.
/// Returns `Err(messages)` listing every validation error.
pub fn validate_output(data: &Value) -> Result<(), String> {
    ACTIVE_SCHEMA.with(|cell| {
        let guard = cell.borrow();
        let Some(validator) = guard.as_ref() else {
            return Ok(());
        };
        let errors: Vec<String> = validator
            .iter_errors(data)
            .map(|e| {
                let path = e.instance_path().to_string();
                let loc = if path.is_empty() { "root".to_string() } else { path };
                format!("{loc}: {e}")
            })
            .collect();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    })
}

/// Clear the active schema for this thread.
pub fn clear_active_schema() {
    ACTIVE_SCHEMA.with(|cell| {
        *cell.borrow_mut() = None;
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn no_schema_means_pass_normal() {
        clear_active_schema();
        assert!(validate_output(&json!({"x": 1})).is_ok());
    }

    #[test]
    fn matching_data_passes_normal() {
        let schema = json!({
            "type": "object",
            "properties": {"name": {"type": "string"}},
            "required": ["name"]
        });
        set_active_schema(Some(&schema)).unwrap();
        assert!(validate_output(&json!({"name": "x"})).is_ok());
        clear_active_schema();
    }

    #[test]
    fn missing_required_field_fails_robust() {
        let schema = json!({
            "type": "object",
            "properties": {"name": {"type": "string"}},
            "required": ["name"]
        });
        set_active_schema(Some(&schema)).unwrap();
        let err = validate_output(&json!({"other": 1})).expect_err("should fail");
        assert!(err.contains("name") || err.contains("required"));
        clear_active_schema();
    }

    #[test]
    fn wrong_type_fails_robust() {
        let schema = json!({
            "type": "object",
            "properties": {"n": {"type": "integer"}},
            "required": ["n"]
        });
        set_active_schema(Some(&schema)).unwrap();
        assert!(validate_output(&json!({"n": "not an integer"})).is_err());
        clear_active_schema();
    }

    #[test]
    fn malformed_schema_returns_error_robust() {
        let bad = json!({"type": "not-a-real-type"});
        assert!(set_active_schema(Some(&bad)).is_err());
    }
}
