use serde_json::{Value, json};

use jfc_provider::{ModelId, ToolDef};

pub fn build_tools(tools: &[ToolDef]) -> Value {
    build_tools_with_advisor(tools, None)
}

pub fn build_tools_with_advisor(tools: &[ToolDef], advisor_model: Option<&ModelId>) -> Value {
    tracing::trace!(
        target: "jfc::provider::sse",
        tool_count = tools.len(),
        advisor_model = ?advisor_model.map(|m| m.as_str()),
        "build_tools"
    );
    let mut out = tools
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema,
            })
        })
        .collect::<Vec<_>>();
    if let Some(model) = advisor_model {
        out.push(json!({
            "type": "advisor_20260301",
            "name": "advisor",
            "model": model.as_str(),
        }));
    }
    out.into()
}

/// Apply Anthropic-native per-tool schema controls to local tool definitions.
/// Server tools such as `advisor_20260301` have their own wire shape and must
/// not receive local-tool-only fields.
pub fn apply_anthropic_tool_schema_controls(
    tools: &mut Value,
    eager_input_streaming: bool,
    strict_tool_schemas: bool,
) {
    if !eager_input_streaming && !strict_tool_schemas {
        return;
    }
    let Some(arr) = tools.as_array_mut() else {
        return;
    };
    for tool in arr {
        let Some(obj) = tool.as_object_mut() else {
            continue;
        };
        if obj.contains_key("type") {
            continue;
        }
        if eager_input_streaming {
            obj.insert("eager_input_streaming".to_owned(), json!(true));
        }
        if strict_tool_schemas {
            obj.insert("strict".to_owned(), json!(true));
        }
    }
}
