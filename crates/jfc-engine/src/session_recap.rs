//! Session recap — generates a brief summary of what happened while the
//! user was away.
//!
//! When the user returns after >5 minutes of inactivity, this module
//! scans messages that arrived since their last interaction and produces
//! a concise recap: tool calls completed, errors encountered, files changed.
//!
//! The recap is rendered as an ephemeral banner at the top of the viewport
//! so the user can quickly orient without scrolling through pages of
//! tool output.

use std::collections::HashSet;
use std::time::Duration;

/// Minimum inactivity before a recap is generated.
pub const AWAY_THRESHOLD: Duration = Duration::from_secs(300); // 5 minutes

/// Summary of a message for recap purposes.
#[derive(Debug, Clone)]
pub struct RecapMessage {
    /// Was this from the assistant?
    pub is_assistant: bool,
    /// Tool calls in this message (tool name).
    pub tool_calls: Vec<String>,
    /// Whether any tool in this message errored.
    pub had_error: bool,
    /// Files written/edited in this message.
    pub files_changed: Vec<String>,
    /// Brief text content (first 200 chars of assistant text).
    pub text_preview: String,
}

/// Generate a recap of activity that happened since the user's last interaction.
///
/// Returns a formatted summary string, or `None` if nothing meaningful happened.
pub fn generate_recap(messages_since_last_interaction: &[RecapMessage]) -> Option<String> {
    if messages_since_last_interaction.is_empty() {
        return None;
    }

    let mut tool_calls: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut files_changed: HashSet<String> = HashSet::new();
    let mut assistant_snippets: Vec<String> = Vec::new();

    for msg in messages_since_last_interaction {
        if msg.is_assistant {
            // Collect tool calls
            for tool in &msg.tool_calls {
                tool_calls.push(tool.clone());
            }
            // Collect errors
            if msg.had_error {
                errors.push(format!("Error in {}", msg.tool_calls.join(", ")));
            }
            // Collect file changes
            for f in &msg.files_changed {
                files_changed.insert(f.clone());
            }
            // Collect text previews (first meaningful one)
            if !msg.text_preview.is_empty() && assistant_snippets.len() < 3 {
                assistant_snippets.push(msg.text_preview.clone());
            }
        }
    }

    // If nothing happened, no recap needed
    if tool_calls.is_empty() && errors.is_empty() && files_changed.is_empty() {
        return None;
    }

    // Plain, label-prefixed lines — no emoji, no box rules. The renderer
    // styles them; the strings just state what the stream actually did.
    let mut lines: Vec<String> = Vec::new();
    lines.push("While you were away".to_string());

    // Tool call summary
    if !tool_calls.is_empty() {
        let grouped = group_tool_calls(&tool_calls);
        let summary: Vec<String> = grouped
            .iter()
            .map(|(name, count)| {
                if *count > 1 {
                    format!("{name} ×{count}")
                } else {
                    name.clone()
                }
            })
            .collect();
        lines.push(format!("Tools: {}", summary.join(", ")));
    }

    // File changes
    if !files_changed.is_empty() {
        let file_list: Vec<&str> = files_changed.iter().map(|s| s.as_str()).collect();
        if file_list.len() <= 5 {
            lines.push(format!("Files: {}", file_list.join(", ")));
        } else {
            let shown: Vec<&str> = file_list[..3].to_vec();
            lines.push(format!(
                "Files: {} (+{} more)",
                shown.join(", "),
                file_list.len() - 3
            ));
        }
    }

    // Errors
    if !errors.is_empty() {
        lines.push(format!("Errors: {}", errors.len()));
        for err in errors.iter().take(3) {
            lines.push(format!("  · {err}"));
        }
    }

    // Last assistant message snippet
    if let Some(snippet) = assistant_snippets.last() {
        let truncated = if snippet.len() > 120 {
            format!("{}…", &snippet[..snippet.floor_char_boundary(120)])
        } else {
            snippet.clone()
        };
        lines.push(format!("Last: {truncated}"));
    }

    Some(lines.join("\n"))
}

/// Group tool calls by name and count occurrences.
fn group_tool_calls(calls: &[String]) -> Vec<(String, usize)> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for call in calls {
        if let Some(entry) = counts.iter_mut().find(|(name, _)| name == call) {
            entry.1 += 1;
        } else {
            counts.push((call.clone(), 1));
        }
    }
    // Sort by count descending
    counts.sort_by_key(|b| std::cmp::Reverse(b.1));
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_messages() {
        assert_eq!(generate_recap(&[]), None);
    }

    #[test]
    fn test_no_meaningful_activity() {
        let msgs = vec![RecapMessage {
            is_assistant: true,
            tool_calls: vec![],
            had_error: false,
            files_changed: vec![],
            text_preview: String::new(),
        }];
        assert_eq!(generate_recap(&msgs), None);
    }

    #[test]
    fn test_basic_recap() {
        let msgs = vec![
            RecapMessage {
                is_assistant: true,
                tool_calls: vec!["Read".into(), "Read".into(), "Edit".into()],
                had_error: false,
                files_changed: vec!["src/main.rs".into()],
                text_preview: "I've updated the main function".into(),
            },
            RecapMessage {
                is_assistant: true,
                tool_calls: vec!["Bash".into()],
                had_error: true,
                files_changed: vec![],
                text_preview: "The tests failed with an error".into(),
            },
        ];

        let recap = generate_recap(&msgs).unwrap();
        assert!(recap.contains("While you were away"));
        assert!(recap.contains("Read ×2"));
        assert!(recap.contains("Edit"));
        assert!(recap.contains("Bash"));
        assert!(recap.contains("src/main.rs"));
        assert!(recap.contains("Errors: 1"));
    }

    #[test]
    fn test_many_files() {
        let msgs = vec![RecapMessage {
            is_assistant: true,
            tool_calls: vec!["Write".into()],
            had_error: false,
            files_changed: vec![
                "a.rs".into(),
                "b.rs".into(),
                "c.rs".into(),
                "d.rs".into(),
                "e.rs".into(),
                "f.rs".into(),
            ],
            text_preview: String::new(),
        }];

        let recap = generate_recap(&msgs).unwrap();
        assert!(recap.contains("+3 more"));
    }
}
