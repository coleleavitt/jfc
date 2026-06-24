use super::defs::all_tool_defs;

#[test]
fn task_tool_schema_advertises_structured_output_schema_regression() {
    let defs = all_tool_defs();
    let task = defs
        .iter()
        .find(|def| def.name == "Task")
        .expect("Task tool should be present");

    let schema_prop = task.input_schema["properties"]
        .get("schema")
        .expect("Task schema must advertise schema");

    assert_eq!(schema_prop["type"], "object");
    assert!(
        schema_prop["description"]
            .as_str()
            .unwrap_or_default()
            .contains("StructuredOutput"),
        "Task schema field must explain StructuredOutput: {}",
        schema_prop
    );
    assert!(
        task.description.contains("Subagents have isolated context"),
        "Task description must explain explicit context passing: {}",
        task.description
    );

    let allowed_tools = task.input_schema["properties"]
        .get("allowed_tools")
        .expect("Task schema must advertise per-call allowed_tools");
    assert_eq!(allowed_tools["type"], "array");
    assert!(
        allowed_tools["description"]
            .as_str()
            .unwrap_or_default()
            .contains("narrows"),
        "allowed_tools description must explain restrictive merge semantics: {}",
        allowed_tools
    );

    let disallowed_tools = task.input_schema["properties"]
        .get("disallowed_tools")
        .expect("Task schema must advertise per-call disallowed_tools");
    assert_eq!(disallowed_tools["type"], "array");
}
