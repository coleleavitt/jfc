//! Parse workflow script metadata and validate determinism constraints.

use serde::{Deserialize, Serialize};

/// Extracted metadata from `export const meta = { ... }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowMeta {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub when_to_use: Option<String>,
    #[serde(default)]
    pub phases: Vec<PhaseMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseMeta {
    pub title: String,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

/// Maximum script size in bytes (512KB, matching CC 146).
pub const MAX_SCRIPT_SIZE: usize = 524_288;

/// Parse the `export const meta = { ... }` literal from a workflow script.
/// Returns the meta object and the remaining script body (everything after
/// the meta declaration).
pub fn parse_meta(script: &str) -> Result<(WorkflowMeta, String), String> {
    // Find `export const meta = {`
    let Some(start) = script.find("export const meta") else {
        return Err(
            "Script must begin with `export const meta = { name, description, phases }`".to_owned(),
        );
    };

    // Find the opening brace
    let after_eq = &script[start..];
    let Some(brace_offset) = after_eq.find('{') else {
        return Err("Expected `{` after `export const meta =`".to_owned());
    };
    let brace_start = start + brace_offset;

    // Match braces to find the end of the meta object
    let mut depth = 0u32;
    let mut end = brace_start;
    for (i, ch) in script[brace_start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = brace_start + i + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err("Unbalanced braces in meta object".to_owned());
    }

    let meta_text = &script[brace_start..end];
    // Convert JS object literal to JSON (basic transforms: unquoted keys, trailing commas, single quotes)
    let json = js_object_to_json(meta_text);

    let meta: WorkflowMeta =
        serde_json::from_str(&json).map_err(|e| format!("Failed to parse meta as JSON: {e}"))?;

    if meta.name.is_empty() {
        return Err("meta.name is required".to_owned());
    }
    if meta.description.is_empty() {
        return Err("meta.description is required".to_owned());
    }

    // Script body = everything after the meta declaration
    let body = script[end..].trim_start().to_owned();
    Ok((meta, body))
}

/// Validate that a script doesn't use non-deterministic APIs.
pub fn validate_script(script: &str) -> Result<(), String> {
    if script.len() > MAX_SCRIPT_SIZE {
        return Err(format!(
            "Script exceeds {} bytes (got {})",
            MAX_SCRIPT_SIZE,
            script.len()
        ));
    }

    let determinism_violations =
        regex::Regex::new(r"\bDate\s*\.\s*now\b|\bMath\s*\.\s*random\b|\bnew\s+Date\s*\(\s*\)")
            .unwrap();

    if determinism_violations.is_match(script) {
        return Err(
            "Workflow scripts must be deterministic: Date.now()/Math.random()/new Date() \
             are unavailable (breaks resume). Stamp results after the workflow returns, \
             or pass timestamps via args."
                .to_owned(),
        );
    }

    Ok(())
}

/// Convert a JS object literal to valid JSON.
/// Handles: unquoted keys, single-quoted strings, trailing commas.
fn js_object_to_json(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut string_delim = '"';

    while let Some(ch) = chars.next() {
        if in_string {
            if ch == string_delim {
                out.push('"');
                in_string = false;
            } else if ch == '"' && string_delim == '\'' {
                out.push_str("\\\"");
            } else if ch == '\\' {
                out.push('\\');
                if let Some(esc) = chars.next() {
                    out.push(esc);
                }
            } else {
                out.push(ch);
            }
        } else {
            match ch {
                '\'' => {
                    in_string = true;
                    string_delim = '\'';
                    out.push('"');
                }
                '"' => {
                    in_string = true;
                    string_delim = '"';
                    out.push('"');
                }
                // Unquoted identifier key — quote it
                'a'..='z' | 'A'..='Z' | '_' => {
                    let mut key = String::new();
                    key.push(ch);
                    while let Some(&next) = chars.peek() {
                        if next.is_alphanumeric() || next == '_' {
                            key.push(next);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    // Check if followed by `:` (it's a key) or not
                    let rest_trimmed: String =
                        chars.clone().take_while(|c| c.is_whitespace()).collect();
                    let after_ws = chars.clone().nth(rest_trimmed.len());
                    if after_ws == Some(':') {
                        out.push('"');
                        out.push_str(&key);
                        out.push('"');
                    } else {
                        // It's a value (like `true`, `false`, `null`) or inside a string
                        out.push_str(&key);
                    }
                }
                // Strip trailing commas before } or ]
                ',' => {
                    let rest: String = chars.clone().take_while(|c| c.is_whitespace()).collect();
                    let after = chars.clone().nth(rest.len());
                    if after == Some('}') || after == Some(']') {
                        // trailing comma — skip
                    } else {
                        out.push(',');
                    }
                }
                _ => out.push(ch),
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_meta_simple_normal() {
        let script = r#"export const meta = {
  name: 'find-bugs',
  description: 'Find bugs in the codebase',
  phases: [
    { title: 'Scan', detail: 'grep for patterns' },
    { title: 'Verify' },
  ],
}
phase('Scan')
const results = await agent('Find bugs')
"#;
        let (meta, body) = parse_meta(script).unwrap();
        assert_eq!(meta.name, "find-bugs");
        assert_eq!(meta.description, "Find bugs in the codebase");
        assert_eq!(meta.phases.len(), 2);
        assert_eq!(meta.phases[0].title, "Scan");
        assert!(body.starts_with("phase('Scan')"));
    }

    #[test]
    fn validate_rejects_date_now_robust() {
        assert!(validate_script("const t = Date.now()").is_err());
    }

    #[test]
    fn validate_rejects_math_random_robust() {
        assert!(validate_script("const x = Math.random()").is_err());
    }

    #[test]
    fn validate_allows_clean_script_normal() {
        assert!(validate_script("const x = await agent('hello')").is_ok());
    }

    #[test]
    fn validate_rejects_oversize_robust() {
        let big = "x".repeat(MAX_SCRIPT_SIZE + 1);
        assert!(validate_script(&big).is_err());
    }

    // End-to-end AFlow proof: a WorkflowVariant compiled by the offline
    // optimizer (jfc-learn) produces a script the LIVE engine parser + validator
    // accept. This is the cross-crate check that the compiler's output is
    // actually runnable, not just structurally self-consistent.
    #[test]
    fn aflow_compiled_variant_parses_and_validates_normal() {
        use jfc_learn::{WorkflowOp, WorkflowVariant};

        let variant = WorkflowVariant::from_ops(vec![
            WorkflowOp::Generate,
            WorkflowOp::Ensemble(3),
            WorkflowOp::Review,
            WorkflowOp::Revise,
        ]);
        let script = variant.to_workflow_script("aflow-solver", "fix the failing test");

        // The real engine meta-parser accepts it and recovers the metadata.
        let (meta, body) = parse_meta(&script).expect("compiled script must parse");
        assert_eq!(meta.name, "aflow-solver");
        assert_eq!(meta.phases.len(), 1);
        assert_eq!(meta.phases[0].title, "Solve");
        assert!(body.contains("phase('Solve')"));
        assert!(body.contains("await parallel(["));

        // The real determinism/size validator accepts it (no Date.now/Math.random,
        // under the size cap).
        validate_script(&script).expect("compiled script must validate");
    }

    // Robust: a task prompt containing JS-breaking characters compiles to a
    // script that STILL parses + validates (escaping holds through the engine).
    #[test]
    fn aflow_compiled_variant_escapes_safely_robust() {
        use jfc_learn::WorkflowVariant;
        let variant = WorkflowVariant::seed();
        let script =
            variant.to_workflow_script("esc", "handle 'quotes' and \\ backslashes\nand newlines");
        // Must still be a parseable, valid workflow despite the nasty task text.
        let (meta, _body) = parse_meta(&script).expect("escaped script must parse");
        assert_eq!(meta.name, "esc");
        validate_script(&script).expect("escaped script must validate");
    }
}
