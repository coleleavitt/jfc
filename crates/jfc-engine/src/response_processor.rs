//! Composable response processors.
//!
//! Koog models post-processing as a chain: deterministic repair first, then a
//! model-powered repair only when a concrete validation error remains. JFC
//! already has several repair passes (tool-result pairing, tool-input coercion,
//! structured-output feedback, review normalization); this module provides the
//! shared chain shape for new JSON/tool/review repair steps.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessorFinding {
    pub processor: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessorOutput {
    pub value: Value,
    pub findings: Vec<ProcessorFinding>,
}

impl ProcessorOutput {
    pub fn new(value: Value) -> Self {
        Self {
            value,
            findings: Vec::new(),
        }
    }
}

pub trait JsonResponseProcessor: Send + Sync {
    fn name(&self) -> &'static str;
    fn process(&self, output: ProcessorOutput) -> ProcessorOutput;
}

#[derive(Default)]
pub struct JsonProcessorChain {
    processors: Vec<Box<dyn JsonResponseProcessor>>,
}

impl JsonProcessorChain {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push<P>(mut self, processor: P) -> Self
    where
        P: JsonResponseProcessor + 'static,
    {
        self.processors.push(Box::new(processor));
        self
    }

    pub fn process(&self, value: Value) -> ProcessorOutput {
        self.processors
            .iter()
            .fold(ProcessorOutput::new(value), |output, processor| {
                processor.process(output)
            })
    }
}

/// Deterministic repair for providers/models that return a JSON object as a
/// string. This is intentionally narrow: only strings whose entire trimmed body
/// parses as JSON are rewritten.
#[derive(Debug, Clone, Copy, Default)]
pub struct ParseJsonStringProcessor;

impl JsonResponseProcessor for ParseJsonStringProcessor {
    fn name(&self) -> &'static str {
        "parse_json_string"
    }

    fn process(&self, mut output: ProcessorOutput) -> ProcessorOutput {
        let Value::String(text) = &output.value else {
            return output;
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return output;
        }
        if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
            output.value = parsed;
            output.findings.push(ProcessorFinding {
                processor: self.name(),
                message: "parsed JSON value from string response".to_owned(),
            });
        }
        output
    }
}

/// Deterministic tool-call argument repair: when a tool's argument *value* is a
/// JSON object encoded as a string (a common failure mode for models that
/// double-encode tool inputs), parse it in place.
///
/// This walks the top-level object's fields and rewrites any field whose value
/// is a string that wholly parses as a JSON object or array. Scalars that
/// happen to be JSON (e.g. the string `"true"` or `"42"`) are left untouched so
/// a legitimately-stringly field isn't silently retyped — only structured
/// payloads are repaired.
#[derive(Debug, Clone, Copy, Default)]
pub struct ToolCallArgumentProcessor;

impl JsonResponseProcessor for ToolCallArgumentProcessor {
    fn name(&self) -> &'static str {
        "tool_call_arguments"
    }

    fn process(&self, mut output: ProcessorOutput) -> ProcessorOutput {
        let Value::Object(map) = &mut output.value else {
            return output;
        };
        let mut repaired_fields = Vec::new();
        for (key, value) in map.iter_mut() {
            let Value::String(text) = value else {
                continue;
            };
            let trimmed = text.trim();
            if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
                continue;
            }
            if let Ok(parsed @ (Value::Object(_) | Value::Array(_))) =
                serde_json::from_str::<Value>(trimmed)
            {
                *value = parsed;
                repaired_fields.push(key.clone());
            }
        }
        for key in repaired_fields {
            output.findings.push(ProcessorFinding {
                processor: self.name(),
                message: format!("parsed double-encoded JSON in tool argument `{key}`"),
            });
        }
        output
    }
}

/// Deterministic review-output key normalization: models name the review
/// summary/confidence fields inconsistently (`final_report`/`summary` for the
/// explanation, `confidence` for the score). Canonicalize those synonyms onto
/// the schema keys *before* schema validation so a well-formed review with
/// off-spec key names isn't rejected and bounced to an LLM-repair round-trip.
///
/// Only fills a canonical key when it is absent, so an explicit canonical value
/// always wins over a synonym.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReviewOutputProcessor;

impl ReviewOutputProcessor {
    const SYNONYMS: &'static [(&'static str, &'static [&'static str])] = &[
        ("overall_explanation", &["final_report", "summary"]),
        ("overall_confidence_score", &["confidence"]),
    ];
}

impl JsonResponseProcessor for ReviewOutputProcessor {
    fn name(&self) -> &'static str {
        "review_output"
    }

    fn process(&self, mut output: ProcessorOutput) -> ProcessorOutput {
        let Value::Object(map) = &mut output.value else {
            return output;
        };
        for (canonical, synonyms) in Self::SYNONYMS {
            if map.contains_key(*canonical) {
                continue;
            }
            let Some(source_key) = synonyms.iter().find(|syn| map.contains_key(**syn)).copied()
            else {
                continue;
            };
            if let Some(value) = map.get(source_key).cloned() {
                map.insert((*canonical).to_owned(), value);
                output.findings.push(ProcessorFinding {
                    processor: self.name(),
                    message: format!("mapped review key `{source_key}` -> `{canonical}`"),
                });
            }
        }
        output
    }
}

/// Hard-shape validator for structured/review outputs that must be objects.
#[derive(Debug, Clone, Copy, Default)]
pub struct RequireObjectProcessor;

impl JsonResponseProcessor for RequireObjectProcessor {
    fn name(&self) -> &'static str {
        "require_object"
    }

    fn process(&self, mut output: ProcessorOutput) -> ProcessorOutput {
        if !output.value.is_object() {
            output.findings.push(ProcessorFinding {
                processor: self.name(),
                message: format!(
                    "expected JSON object, got {}",
                    json_type_name(&output.value)
                ),
            });
        }
        output
    }
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

pub fn deterministic_json_repair_chain() -> JsonProcessorChain {
    JsonProcessorChain::new()
        .push(ParseJsonStringProcessor)
        .push(ToolCallArgumentProcessor)
        .push(RequireObjectProcessor)
}

/// Repair chain for review-tool output: parse a stringified body, canonicalize
/// review key synonyms, then enforce the object shape. Runs before the review
/// normalizer/validator so off-spec-but-recoverable review payloads don't need
/// an LLM round-trip.
pub fn review_repair_chain() -> JsonProcessorChain {
    JsonProcessorChain::new()
        .push(ParseJsonStringProcessor)
        .push(ReviewOutputProcessor)
        .push(RequireObjectProcessor)
}

/// Emit a chain's findings to tracing so processor repairs are observable in
/// logs (the telemetry half of "telemetry/finding persistence"; the structured
/// result body carries the same notes back to the model). No-op when the chain
/// made no repairs.
pub fn record_processor_findings(target_run: &str, findings: &[ProcessorFinding]) {
    for finding in findings {
        tracing::debug!(
            target: "jfc::response_processor",
            run = target_run,
            processor = finding.processor,
            message = %finding.message,
            "response processor repair"
        );
    }
}

/// Result of the model-backed repair stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmRepairOutcome {
    /// Deterministic repair already produced a valid value; the model was not
    /// called.
    NotNeeded,
    /// The model returned a value that now validates.
    Repaired(Value),
    /// The model was called but its output still failed validation.
    Failed(String),
}

/// Model-backed repair stage that runs *after* the deterministic chain, only
/// when a concrete validation error remains. Kept separate from the sync
/// [`JsonProcessorChain`] because it is async, fallible, and needs a provider —
/// Koog's "deterministic first, model only on a surviving error" shape.
///
/// `validate` is the caller's hard check (e.g. JSON-schema validation): it
/// returns `Ok(())` for a valid value or `Err(message)` describing the failure.
/// When the deterministic value already passes, the model is never called.
pub struct LlmRepairStage<'a> {
    provider: &'a dyn jfc_provider::Provider,
    model: jfc_provider::ModelId,
    /// Hard validation predicate; `Err` carries the actionable failure message
    /// handed to the model.
    validate: Box<dyn Fn(&Value) -> Result<(), String> + Send + 'a>,
}

impl<'a> LlmRepairStage<'a> {
    pub fn new(
        provider: &'a dyn jfc_provider::Provider,
        model: jfc_provider::ModelId,
        validate: impl Fn(&Value) -> Result<(), String> + Send + 'a,
    ) -> Self {
        Self {
            provider,
            model,
            validate: Box::new(validate),
        }
    }

    /// Attempt a model-backed repair of `value`. Returns [`LlmRepairOutcome`].
    pub async fn repair(&self, value: &Value) -> LlmRepairOutcome {
        let error = match (self.validate)(value) {
            Ok(()) => return LlmRepairOutcome::NotNeeded,
            Err(message) => message,
        };

        let prompt = format!(
            "The following JSON failed validation and must be corrected. Return \
             ONLY the corrected JSON value, no prose, no code fences.\n\n\
             Validation error:\n{error}\n\nInvalid JSON:\n{}",
            serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
        );
        let opts = jfc_provider::StreamOptions::new(self.model.clone())
            .system(LLM_REPAIR_SYSTEM_PROMPT)
            .max_tokens(LLM_REPAIR_MAX_TOKENS);
        let messages = vec![jfc_provider::ProviderMessage {
            role: jfc_provider::ProviderRole::User,
            content: vec![jfc_provider::ProviderContent::Text(prompt)],
        }];

        let resp = match crate::prompt_executor::complete_once(self.provider, messages, &opts).await
        {
            Ok(resp) => resp,
            Err(e) => return LlmRepairOutcome::Failed(format!("repair call failed: {e}")),
        };

        // The model may still wrap the JSON; reuse the deterministic
        // string-parse processor to unwrap a stringified body.
        let parsed = ParseJsonStringProcessor
            .process(ProcessorOutput::new(Value::String(
                resp.content.trim().to_owned(),
            )))
            .value;
        match (self.validate)(&parsed) {
            Ok(()) => LlmRepairOutcome::Repaired(parsed),
            Err(message) => {
                LlmRepairOutcome::Failed(format!("model repair still invalid: {message}"))
            }
        }
    }
}

const LLM_REPAIR_SYSTEM_PROMPT: &str = "You repair malformed JSON so it satisfies a schema. Output only the corrected \
     JSON value — no explanation, no markdown fences.";
const LLM_REPAIR_MAX_TOKENS: u32 = 2048;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_chain_parses_json_string_normal() {
        let output = deterministic_json_repair_chain()
            .process(Value::String("{\"findings\":[]}".to_owned()));
        assert!(output.value.is_object());
        assert!(
            output
                .findings
                .iter()
                .any(|finding| finding.processor == "parse_json_string")
        );
        assert!(
            !output
                .findings
                .iter()
                .any(|finding| finding.processor == "require_object")
        );
    }

    #[test]
    fn deterministic_chain_reports_non_object_robust() {
        let output = deterministic_json_repair_chain().process(Value::Array(Vec::new()));
        assert!(output.value.is_array());
        assert!(
            output
                .findings
                .iter()
                .any(|f| f.processor == "require_object"
                    && f.message == "expected JSON object, got array")
        );
    }

    // Normal: a tool argument that is a double-encoded JSON object is parsed in
    // place and a finding is recorded naming the field.
    #[test]
    fn tool_call_argument_parses_double_encoded_object_normal() {
        let value = serde_json::json!({
            "edits": "[{\"old\":\"a\",\"new\":\"b\"}]",
            "path": "src/lib.rs"
        });
        let output = ToolCallArgumentProcessor.process(ProcessorOutput::new(value));
        assert!(output.value["edits"].is_array());
        assert_eq!(output.value["path"], serde_json::json!("src/lib.rs"));
        assert!(
            output
                .findings
                .iter()
                .any(|f| f.processor == "tool_call_arguments" && f.message.contains("edits"))
        );
    }

    // Robust: a stringly scalar that merely looks numeric/boolean is NOT retyped
    // — only object/array bodies are repaired, so legitimately-string fields are
    // preserved.
    #[test]
    fn tool_call_argument_leaves_plain_scalars_robust() {
        let value = serde_json::json!({ "flag": "true", "count": "42", "name": "x" });
        let output = ToolCallArgumentProcessor.process(ProcessorOutput::new(value.clone()));
        assert_eq!(output.value, value);
        assert!(output.findings.is_empty());
    }

    // Normal: review key synonyms are mapped onto the canonical schema keys.
    #[test]
    fn review_output_maps_synonyms_normal() {
        let value = serde_json::json!({
            "findings": [],
            "final_report": "looks correct",
            "confidence": 0.9
        });
        let output = ReviewOutputProcessor.process(ProcessorOutput::new(value));
        assert_eq!(
            output.value["overall_explanation"],
            serde_json::json!("looks correct")
        );
        assert_eq!(
            output.value["overall_confidence_score"],
            serde_json::json!(0.9)
        );
        assert_eq!(output.findings.len(), 2);
    }

    // Robust: an explicit canonical key is never overwritten by a synonym.
    #[test]
    fn review_output_keeps_explicit_canonical_robust() {
        let value = serde_json::json!({
            "overall_explanation": "canonical wins",
            "summary": "synonym loses"
        });
        let output = ReviewOutputProcessor.process(ProcessorOutput::new(value));
        assert_eq!(
            output.value["overall_explanation"],
            serde_json::json!("canonical wins")
        );
        assert!(output.findings.is_empty());
    }

    // Normal: the review repair chain parses a stringified body and canonicalizes
    // keys in one pass.
    #[test]
    fn review_repair_chain_parses_and_maps_normal() {
        let body = "{\"findings\":[],\"summary\":\"ok\"}".to_owned();
        let output = review_repair_chain().process(Value::String(body));
        assert!(output.value.is_object());
        assert_eq!(output.value["overall_explanation"], serde_json::json!("ok"));
    }

    // ─── LlmRepairStage ──────────────────────────────────────────────────────

    use async_trait::async_trait;
    use jfc_provider::{
        CompletionResponse, EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions,
        TokenUsage,
    };

    struct RepairProvider {
        reply: String,
    }

    impl jfc_provider::seal::Sealed for RepairProvider {}

    #[async_trait]
    impl Provider for RepairProvider {
        fn name(&self) -> &str {
            "repair"
        }
        fn available_models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }
        async fn stream(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            anyhow::bail!("stream unused")
        }
        async fn complete(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<CompletionResponse> {
            Ok(CompletionResponse {
                content: self.reply.clone(),
                usage: TokenUsage::default(),
                context_signals: None,
            })
        }
    }

    // Normal: when the deterministic value already validates, the model is never
    // called and the outcome is NotNeeded.
    #[tokio::test]
    async fn llm_repair_skips_when_valid_normal() {
        let provider = RepairProvider {
            reply: "should not be used".into(),
        };
        let stage = LlmRepairStage::new(&provider, "m".into(), |_| Ok(()));
        let outcome = stage.repair(&serde_json::json!({"ok": true})).await;
        assert_eq!(outcome, LlmRepairOutcome::NotNeeded);
    }

    // Normal: a surviving validation error triggers a model call whose corrected
    // JSON now validates.
    #[tokio::test]
    async fn llm_repair_fixes_invalid_normal() {
        let provider = RepairProvider {
            reply: "{\"fixed\": true}".into(),
        };
        let stage = LlmRepairStage::new(&provider, "m".into(), |v: &Value| {
            if v.get("fixed").is_some() {
                Ok(())
            } else {
                Err("missing `fixed`".to_owned())
            }
        });
        let outcome = stage.repair(&serde_json::json!({"wrong": 1})).await;
        assert_eq!(
            outcome,
            LlmRepairOutcome::Repaired(serde_json::json!({"fixed": true}))
        );
    }

    // Robust: if the model's repaired output still fails validation, the outcome
    // is Failed rather than a silently-accepted invalid value.
    #[tokio::test]
    async fn llm_repair_reports_persistent_failure_robust() {
        let provider = RepairProvider {
            reply: "{\"still\": \"wrong\"}".into(),
        };
        let stage = LlmRepairStage::new(&provider, "m".into(), |_: &Value| {
            Err("never valid".to_owned())
        });
        let outcome = stage.repair(&serde_json::json!({"x": 1})).await;
        assert!(matches!(outcome, LlmRepairOutcome::Failed(_)));
    }
}
