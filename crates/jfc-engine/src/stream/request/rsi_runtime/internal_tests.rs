use super::*;

#[test]
fn prompt_safe_body_rejects_raw_thinking_markers_regression() {
    assert!(prompt_safe_body("<thinking>private</thinking>").is_none());
}

#[test]
fn tool_visibility_lines_formats_budget_metadata_normal() {
    let lines = tool_visibility_lines(
        r#"{"rsi":{"budget":{"tool_visibility":[{"tool_name":"Bash","action":"show_earlier","reason":"verified traces needed it"}]}}}"#,
    );

    assert_eq!(
        lines,
        vec!["show earlier: `Bash` - verified traces needed it"]
    );
}
