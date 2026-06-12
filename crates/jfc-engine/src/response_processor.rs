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
        .push(RequireObjectProcessor)
}

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
        assert_eq!(
            output.findings,
            vec![ProcessorFinding {
                processor: "require_object",
                message: "expected JSON object, got array".to_owned(),
            }]
        );
    }
}
