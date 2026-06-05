use crate::{
    app::App,
    types::{MessagePart, Role},
};

pub(super) fn collect_recent_paths(messages: &[crate::types::ChatMessage]) -> Vec<String> {
    use crate::types::ToolOutput;

    let mut out: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for msg in messages.iter().rev() {
        let mut found_in_this_msg = false;
        for part in msg.parts.iter().rev() {
            let text: String = match part {
                MessagePart::Text(s) | MessagePart::Reasoning(s) => s.clone(),
                MessagePart::Tool(tc) => match &tc.output {
                    ToolOutput::Text(s) => s.clone(),
                    ToolOutput::LargeText(lt) => lt.content.clone(),
                    ToolOutput::Command { stdout, stderr, .. } => {
                        format!("{stdout}\n{stderr}")
                    }
                    ToolOutput::FileContent { content, path, .. } => {
                        format!("{path}\n{content}")
                    }
                    _ => continue,
                },
                _ => continue,
            };
            for matched in scan_path_refs(&text) {
                if seen.insert(matched.clone()) {
                    out.push(matched);
                    found_in_this_msg = true;
                }
            }
        }
        if found_in_this_msg {
            break;
        }
    }
    out
}

pub(super) fn scan_path_refs(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_alphanumeric()
            || b == b'/'
            || b == b'.'
            || b == b'_'
            || b == b'-'
            || b == b'+'
        {
            let start = i;
            while i < bytes.len() {
                let c = bytes[i];
                if c.is_ascii_alphanumeric()
                    || c == b'/'
                    || c == b'.'
                    || c == b'_'
                    || c == b'-'
                    || c == b'+'
                {
                    i += 1;
                } else {
                    break;
                }
            }
            let path_end = i;
            if i + 1 < bytes.len() && bytes[i] == b':' && bytes[i + 1].is_ascii_digit() {
                let after_colon = i + 1;
                let mut j = after_colon;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                let line_end = j;
                let col_end =
                    if j + 1 < bytes.len() && bytes[j] == b':' && bytes[j + 1].is_ascii_digit() {
                        let mut k = j + 1;
                        while k < bytes.len() && bytes[k].is_ascii_digit() {
                            k += 1;
                        }
                        k
                    } else {
                        line_end
                    };
                let path_slice = &text[start..path_end];
                let is_url = path_slice.starts_with("http://")
                    || path_slice.starts_with("https://")
                    || path_slice.starts_with("file://");
                let is_pure_number = path_slice.bytes().all(|c| c.is_ascii_digit());
                let has_path_char = path_slice.contains('/') || path_slice.contains('.');
                if !is_url && !is_pure_number && has_path_char && path_end > start {
                    let captured = &text[start..col_end];
                    out.push(captured.to_owned());
                }
                i = col_end;
                continue;
            }
        }
        i += 1;
    }
    out
}

pub(super) fn refresh_search_matches(app: &mut App, query: &str) {
    let q = query.to_lowercase();
    let mut matches: Vec<usize> = Vec::new();
    if !q.is_empty() {
        for (idx, msg) in app.messages.iter().enumerate() {
            let body_hit = msg.parts.iter().any(|part| match part {
                crate::types::MessagePart::Text(text) => text.to_lowercase().contains(&q),
                crate::types::MessagePart::Reasoning(text) => text.to_lowercase().contains(&q),
                crate::types::MessagePart::Tool(tool) => {
                    tool.input.summary().to_lowercase().contains(&q)
                        || match &tool.output {
                            crate::types::ToolOutput::Text(text) => {
                                text.to_lowercase().contains(&q)
                            }
                            crate::types::ToolOutput::LargeText(large_text) => {
                                large_text.content.to_lowercase().contains(&q)
                            }
                            _ => false,
                        }
                }
                _ => false,
            });
            if body_hit {
                matches.push(idx);
            }
        }
    }
    let first_target = if let Some(search) = app.transcript_search.as_mut() {
        search.matches = matches;
        search.cursor = 0;
        search.matches.first().copied()
    } else {
        None
    };
    if let Some(target) = first_target {
        scroll_to_message(app, target);
    }
}

pub(super) fn scroll_to_message(app: &mut App, target_idx: usize) {
    if target_idx >= app.messages.len() {
        return;
    }

    let approx_width: usize = 80;
    let mut offset = 0usize;
    for (idx, msg) in app.messages.iter().enumerate() {
        if idx >= target_idx {
            break;
        }
        offset += 1;
        for part in &msg.parts {
            let chars = part.approx_text_len();
            if chars == 0 {
                offset += 1;
            } else {
                offset += chars.div_ceil(approx_width);
            }
        }
        offset += 1;
    }
    app.scroll_offset = offset;
    app.follow_bottom = false;
    crate::toast::push_with_cap(
        &mut app.toasts,
        crate::toast::Toast::new(
            crate::toast::ToastKind::Info,
            format!(
                "jumped to message {}/{}",
                target_idx + 1,
                app.messages.len()
            ),
        ),
    );
}

pub(super) fn jump_to_last_error(app: &mut App) {
    use crate::types::ToolStatus;

    let target = app.messages.iter().enumerate().rev().find(|(_, message)| {
        message.parts.iter().any(|part| {
            matches!(
                part,
                MessagePart::Tool(tool) if tool.status == ToolStatus::Failed
            )
        })
    });
    match target {
        Some((idx, _)) => scroll_to_message(app, idx),
        None => crate::toast::push_with_cap(
            &mut app.toasts,
            crate::toast::Toast::new(
                crate::toast::ToastKind::Warning,
                "no failed tools in this session".to_string(),
            ),
        ),
    }
}

pub(super) fn jump_to_last_tool(app: &mut App) {
    let target = app.messages.iter().enumerate().rev().find(|(_, message)| {
        message
            .parts
            .iter()
            .any(|part| matches!(part, MessagePart::Tool(_)))
    });
    match target {
        Some((idx, _)) => scroll_to_message(app, idx),
        None => crate::toast::push_with_cap(
            &mut app.toasts,
            crate::toast::Toast::new(
                crate::toast::ToastKind::Warning,
                "no tool calls in this session".to_string(),
            ),
        ),
    }
}

pub(super) fn jump_to_last_user(app: &mut App) {
    let target = app
        .messages
        .iter()
        .enumerate()
        .rev()
        .find(|(_, message)| message.role_is_user() && !message.is_compact_boundary());
    match target {
        Some((idx, _)) => scroll_to_message(app, idx),
        None => crate::toast::push_with_cap(
            &mut app.toasts,
            crate::toast::Toast::new(
                crate::toast::ToastKind::Warning,
                "no user messages yet".to_string(),
            ),
        ),
    }
}

pub(super) fn jump_to_last_assistant(app: &mut App) {
    let target = app
        .messages
        .iter()
        .enumerate()
        .rev()
        .find(|(_, message)| !message.role_is_user());
    match target {
        Some((idx, _)) => scroll_to_message(app, idx),
        None => crate::toast::push_with_cap(
            &mut app.toasts,
            crate::toast::Toast::new(
                crate::toast::ToastKind::Warning,
                "no assistant messages yet".to_string(),
            ),
        ),
    }
}

pub(super) fn user_prompts(app: &App) -> Vec<String> {
    app.messages
        .iter()
        .filter(|message| message.role == Role::User)
        .filter_map(|message| {
            let text: String = message
                .parts
                .iter()
                .filter_map(|part| match part {
                    MessagePart::Text(text) if !text.is_empty() => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            if text.is_empty() { None } else { Some(text) }
        })
        .collect()
}

pub(super) fn recall_previous_prompt(app: &mut App) -> Option<String> {
    let prompts = user_prompts(app);
    if prompts.is_empty() {
        return None;
    }
    let next = match app.history_cursor {
        None => prompts.len() - 1,
        Some(0) => return None,
        Some(index) => index - 1,
    };
    app.history_cursor = Some(next);
    prompts.get(next).cloned()
}

pub(super) fn recall_next_prompt(app: &mut App) -> Option<String> {
    let prompts = user_prompts(app);
    let current = app.history_cursor?;
    if current + 1 >= prompts.len() {
        app.history_cursor = None;
        return None;
    }
    let next = current + 1;
    app.history_cursor = Some(next);
    prompts.get(next).cloned()
}
