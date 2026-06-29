use serde_json::Value;

use jfc_provider::ServerToolResultKind;

pub enum BlockState {
    Text {
        accumulated: String,
    },
    Thinking {
        accumulated: String,
        estimated_tokens: u32,
        signature: Option<String>,
    },
    /// Opaque redacted thinking — no deltas, complete at start.
    /// Must be round-tripped in subsequent requests verbatim.
    RedactedThinking {
        data: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: String,
    },
    /// Server-side tool invocation block (web_search, code_execution, etc.).
    /// Input is pre-populated from the start block and emits a prefixed
    /// ToolDone name so the rendering layer can distinguish server tools
    /// from locally-dispatched ones.
    ServerToolUse {
        id: String,
        name: String,
        input: String,
    },
    /// Server-side tool result block. Anthropic emits the entire
    /// content blob in the start event (cli.js v142:548307 routes the
    /// raw block straight into the result accumulator with no
    /// `input_json_delta` continuation), so we just hold the parsed
    /// JSON until `content_block_stop` releases it as a
    /// `StreamEvent::ServerToolResult`.
    ServerToolResult {
        tool_use_id: String,
        tool_kind: ServerToolResultKind,
        content: Value,
    },
    Ignored {
        kind: String,
    },
}

pub(crate) fn initial_input_json(input: Value) -> String {
    match input {
        Value::Null => String::new(),
        Value::Object(map) if map.is_empty() => String::new(),
        other => other.to_string(),
    }
}

pub(crate) fn append_input_delta(input: &mut String, partial_json: &str) {
    if partial_json.is_empty() {
        return;
    }
    if input == "{}" {
        input.clear();
    }
    input.push_str(partial_json);
}
