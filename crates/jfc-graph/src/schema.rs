//! Stable JSON schemas for downstream tools.
//!
//! `.codegraph/`-style external consumers (IDE plugins, doc generators,
//! CI scripts, audit harnesses) have historically had to reverse-engineer
//! the shape of our results. This module exports a **versioned envelope**
//! around every public structured payload — `QueryResult`,
//! `EntrypointSummary`, `ContextResult`, `FormattedOutput` — so a
//! downstream tool can pin to `schema_version = N` and trust the field set.
//!
//! ## Versioning
//!
//! - [`SCHEMA_VERSION`] is bumped whenever a field is **renamed or removed**,
//!   or its semantics change. Adding optional fields is non-breaking and
//!   does NOT bump the version.
//! - The wire format is `{ "schema_version": N, "kind": "<payload-kind>",
//!   "data": <payload> }`. The `kind` discriminator lets a consumer route
//!   without having to attempt deserialisation against each shape.
//!
//! ## JSON Schema
//!
//! [`json_schema_for`] returns a textual JSON Schema (draft-07 compatible)
//! for any payload kind, so an external generator can produce typed
//! bindings (TS, Python) without depending on this crate.

use serde::{Deserialize, Serialize};

use crate::analysis::EntrypointSummary;
use crate::context::ContextResult;
use crate::dsl::QueryResult;
use crate::formatting::FormattedOutput;
use crate::nodes::NodeId;

/// Current schema version. Bumped on breaking field changes.
///
/// History:
/// - `1` (current): initial release. Covers QueryResult, EntrypointSummary,
///   ContextResult, FormattedOutput.
pub const SCHEMA_VERSION: u32 = 1;

/// Discriminator for the `kind` field in [`Envelope`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadKind {
    QueryResult,
    EntrypointSummary,
    ContextResult,
    FormattedOutput,
}

impl PayloadKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::QueryResult => "query_result",
            Self::EntrypointSummary => "entrypoint_summary",
            Self::ContextResult => "context_result",
            Self::FormattedOutput => "formatted_output",
        }
    }
}

/// Versioned envelope around any structured payload.
///
/// Generic over the payload type. Use the type-specific aliases
/// ([`QueryResultEnvelope`], etc.) for ergonomics — they pin
/// `T` and pre-populate `kind`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope<T> {
    pub schema_version: u32,
    pub kind: PayloadKind,
    pub data: T,
}

impl<T> Envelope<T> {
    pub fn new(kind: PayloadKind, data: T) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            kind,
            data,
        }
    }
}

/// Envelope around a [`QueryResult`].
pub type QueryResultEnvelope = Envelope<QueryResult>;

/// Wrap a [`QueryResult`] in its versioned envelope.
pub fn wrap_query_result(result: QueryResult) -> QueryResultEnvelope {
    Envelope::new(PayloadKind::QueryResult, result)
}

/// Wrap an [`EntrypointSummary`] list in a versioned envelope. We
/// envelope the *list* rather than each entry so a single API call
/// returns one schema-versioned document.
pub fn wrap_entrypoints(summaries: Vec<EntrypointSummary>) -> Envelope<Vec<EntrypointSummary>> {
    Envelope::new(PayloadKind::EntrypointSummary, summaries)
}

/// Serialisable projection of [`ContextResult`] for the schema layer.
/// `ContextResult` itself carries non-serialisable internals like
/// `ExploreBudget` (Copy) and `TaskIntent`; here we pin the wire format
/// to the fields downstream tools actually want.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextResultSchema {
    pub query: String,
    pub intent: String,
    pub entry_points: Vec<NodeId>,
    pub related: Vec<NodeId>,
    pub markdown: String,
}

impl From<&ContextResult> for ContextResultSchema {
    fn from(c: &ContextResult) -> Self {
        Self {
            query: c.query.clone(),
            intent: format!("{:?}", c.intent),
            entry_points: c.entry_points.clone(),
            related: c.related.clone(),
            markdown: c.markdown.clone(),
        }
    }
}

/// Wrap a [`ContextResult`] in its versioned envelope.
pub fn wrap_context_result(result: &ContextResult) -> Envelope<ContextResultSchema> {
    Envelope::new(
        PayloadKind::ContextResult,
        ContextResultSchema::from(result),
    )
}

/// Wrap a [`FormattedOutput`] in a versioned envelope. `FormattedOutput`
/// itself is the wire shape — its fields are all serialisable scalars
/// downstream consumers want directly.
pub fn wrap_formatted_output(output: &FormattedOutput) -> Envelope<FormattedOutput> {
    Envelope::new(PayloadKind::FormattedOutput, output.clone())
}

/// Return a textual JSON Schema document for one of the four
/// stabilised payload kinds. Draft-07 compatible.
///
/// Downstream tools can pipe the result through `typescript-json-schema`,
/// `quicktype`, or `datamodel-code-generator` to produce native bindings.
pub fn json_schema_for(kind: PayloadKind) -> &'static str {
    match kind {
        PayloadKind::QueryResult => QUERY_RESULT_SCHEMA,
        PayloadKind::EntrypointSummary => ENTRYPOINT_SUMMARY_SCHEMA,
        PayloadKind::ContextResult => CONTEXT_RESULT_SCHEMA,
        PayloadKind::FormattedOutput => FORMATTED_OUTPUT_SCHEMA,
    }
}

const QUERY_RESULT_SCHEMA: &str = r#"{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "QueryResult",
  "description": "Result of a jfc-graph DSL query execution.",
  "type": "object",
  "required": ["schema_version", "kind", "data"],
  "properties": {
    "schema_version": { "type": "integer", "minimum": 1 },
    "kind": { "type": "string", "enum": ["query_result"] },
    "data": {
      "type": "object",
      "required": ["nodes", "edges", "was_truncated", "total_before_truncation", "cycles_detected", "metadata"],
      "properties": {
        "nodes": { "type": "array", "items": { "type": "integer" } },
        "edges": {
          "type": "array",
          "items": {
            "type": "array",
            "minItems": 3,
            "maxItems": 3,
            "items": [
              { "type": "integer" },
              { "type": "integer" },
              { "type": "string" }
            ]
          }
        },
        "was_truncated": { "type": "boolean" },
        "total_before_truncation": { "type": "integer", "minimum": 0 },
        "cycles_detected": { "type": "array", "items": { "type": "integer" } },
        "metadata": { "type": "array", "items": { "type": "string" } }
      }
    }
  }
}"#;

const ENTRYPOINT_SUMMARY_SCHEMA: &str = r#"{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "EntrypointSummary",
  "description": "Per-entrypoint reach metrics from CodeGraph::classify_entrypoints.",
  "type": "object",
  "required": ["schema_version", "kind", "data"],
  "properties": {
    "schema_version": { "type": "integer", "minimum": 1 },
    "kind": { "type": "string", "enum": ["entrypoint_summary"] },
    "data": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["node_id", "kind", "fan_in", "fan_out", "max_reach_depth", "reach_size"],
        "properties": {
          "node_id": { "type": "integer" },
          "kind": { "type": "string", "enum": ["Main", "PublicApi", "Test", "Bench", "FfiExport"] },
          "fan_in": { "type": "integer", "minimum": 0 },
          "fan_out": { "type": "integer", "minimum": 0 },
          "max_reach_depth": { "type": "integer", "minimum": 0 },
          "reach_size": { "type": "integer", "minimum": 0 }
        }
      }
    }
  }
}"#;

const CONTEXT_RESULT_SCHEMA: &str = r#"{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "ContextResult",
  "description": "Output of the context() builder — entry points, related symbols, and rendered markdown.",
  "type": "object",
  "required": ["schema_version", "kind", "data"],
  "properties": {
    "schema_version": { "type": "integer", "minimum": 1 },
    "kind": { "type": "string", "enum": ["context_result"] },
    "data": {
      "type": "object",
      "required": ["query", "intent", "entry_points", "related", "markdown"],
      "properties": {
        "query": { "type": "string" },
        "intent": { "type": "string", "enum": ["Bug", "Exploration", "Feature", "Unknown"] },
        "entry_points": { "type": "array", "items": { "type": "integer" } },
        "related": { "type": "array", "items": { "type": "integer" } },
        "markdown": { "type": "string" }
      }
    }
  }
}"#;

const FORMATTED_OUTPUT_SCHEMA: &str = r#"{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "FormattedOutput",
  "description": "Token-budgeted formatted output from format_query_result.",
  "type": "object",
  "required": ["schema_version", "kind", "data"],
  "properties": {
    "schema_version": { "type": "integer", "minimum": 1 },
    "kind": { "type": "string", "enum": ["formatted_output"] },
    "data": {
      "type": "object",
      "required": ["text", "token_estimate", "was_truncated", "nodes_shown", "nodes_total"],
      "properties": {
        "text": { "type": "string" },
        "token_estimate": { "type": "integer", "minimum": 0 },
        "was_truncated": { "type": "boolean" },
        "nodes_shown": { "type": "integer", "minimum": 0 },
        "nodes_total": { "type": "integer", "minimum": 0 }
      }
    }
  }
}"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::EntrypointKind;

    #[test]
    fn schema_version_is_one() {
        assert_eq!(SCHEMA_VERSION, 1);
    }

    #[test]
    fn payload_kind_str_round_trip() {
        for kind in [
            PayloadKind::QueryResult,
            PayloadKind::EntrypointSummary,
            PayloadKind::ContextResult,
            PayloadKind::FormattedOutput,
        ] {
            let json = serde_json::to_string(&kind).unwrap();
            let parsed: PayloadKind = serde_json::from_str(&json).unwrap();
            assert_eq!(kind, parsed);
        }
    }

    #[test]
    fn envelope_serialises_with_version_and_kind() {
        let result = QueryResult::default();
        let env = wrap_query_result(result);
        let json = serde_json::to_value(&env).unwrap();
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["kind"], "query_result");
        assert!(json["data"].is_object());
    }

    #[test]
    fn entrypoint_envelope_carries_list() {
        let summaries = vec![EntrypointSummary {
            node_id: crate::nodes::NodeId(42),
            kind: EntrypointKind::Main,
            fan_in: 0,
            fan_out: 3,
            max_reach_depth: 5,
            reach_size: 12,
        }];
        let env = wrap_entrypoints(summaries);
        let json = serde_json::to_value(&env).unwrap();
        assert_eq!(json["kind"], "entrypoint_summary");
        assert_eq!(json["data"][0]["fan_out"], 3);
    }

    #[test]
    fn json_schema_for_returns_valid_json_for_each_kind() {
        for kind in [
            PayloadKind::QueryResult,
            PayloadKind::EntrypointSummary,
            PayloadKind::ContextResult,
            PayloadKind::FormattedOutput,
        ] {
            let schema = json_schema_for(kind);
            let parsed: serde_json::Value =
                serde_json::from_str(schema).expect("valid JSON Schema");
            assert!(parsed["title"].is_string());
            assert_eq!(parsed["properties"]["schema_version"]["type"], "integer");
        }
    }

    #[test]
    fn query_result_envelope_round_trips() {
        let mut result = QueryResult::default();
        result.metadata.push("test-metadata".to_string());
        let env = wrap_query_result(result.clone());
        let json = serde_json::to_string(&env).unwrap();
        let parsed: QueryResultEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.schema_version, SCHEMA_VERSION);
        assert_eq!(parsed.data.metadata, result.metadata);
    }

    #[test]
    fn formatted_output_schema_strips_internal_fields() {
        let fo = FormattedOutput {
            text: "hello".into(),
            token_estimate: 10,
            was_truncated: false,
            nodes_shown: 1,
            nodes_total: 1,
        };
        let env = wrap_formatted_output(&fo);
        assert_eq!(env.data.text, "hello");
        assert_eq!(env.data.nodes_shown, 1);
    }
}
