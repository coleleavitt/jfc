use super::model::{ArchivedProviderContent, ArchivedProviderRole, ProviderHistoryArchive};

const MAX_RENDER_MESSAGE_CHARS: usize = 2_000;
const MAX_RENDER_TOTAL_CHARS: usize = 16_000;

pub(super) fn render_archive(archive: &ProviderHistoryArchive) -> String {
    let mut out = format!(
        "Provider-history archive `{}` ({} messages, pre-transform estimate {} tokens, saved {}).\n\
         Raw provider-visible messages below are the exact replay omitted from this request.\n",
        archive.id,
        archive.messages.len(),
        archive.pre_tokens,
        archive.created_at
    );

    for (idx, message) in archive.messages.iter().enumerate() {
        let role = match message.role {
            ArchivedProviderRole::User => "user",
            ArchivedProviderRole::Assistant => "assistant",
        };
        let text = render_message_content(&message.content);
        if text.trim().is_empty() {
            continue;
        }
        let entry = format!(
            "\n[{role} #{idx}]\n{}\n",
            truncate_chars(text.trim(), MAX_RENDER_MESSAGE_CHARS)
        );
        if out.len() + entry.len() > MAX_RENDER_TOTAL_CHARS {
            out.push_str("\n... [provider-history archive truncated]\n");
            break;
        }
        out.push_str(&entry);
    }
    out
}

pub(super) fn archive_text(archive: &ProviderHistoryArchive) -> String {
    let mut out = format!(
        "{}\n{}\n{}\n{} messages\n",
        archive.id,
        archive.created_at,
        archive.summary,
        archive.messages.len()
    );
    for (idx, message) in archive.messages.iter().enumerate() {
        let role = match message.role {
            ArchivedProviderRole::User => "user",
            ArchivedProviderRole::Assistant => "assistant",
        };
        let text = render_message_content(&message.content);
        if text.trim().is_empty() {
            continue;
        }
        out.push_str(&format!("\n[{role} #{idx}]\n{}\n", text.trim()));
    }
    out
}

fn render_message_content(content: &[ArchivedProviderContent]) -> String {
    content
        .iter()
        .map(render_content)
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_content(content: &ArchivedProviderContent) -> String {
    match content {
        ArchivedProviderContent::Text { text }
        | ArchivedProviderContent::Thinking { text, .. }
        | ArchivedProviderContent::ToolResult { content: text, .. } => text.clone(),
        ArchivedProviderContent::ToolUse { name, input, .. }
        | ArchivedProviderContent::ServerToolUse { name, input, .. } => {
            format!(
                "[tool_use name={name} input={}]",
                truncate_chars(&input.to_string(), 800)
            )
        }
        ArchivedProviderContent::ServerToolResult {
            tool_kind, content, ..
        } => format!(
            "[server_tool_result kind={tool_kind} content={}]",
            truncate_chars(&content.to_string(), 800)
        ),
        ArchivedProviderContent::Attachment {
            id,
            mime_type,
            byte_len,
            ..
        } => format!("[attachment id={id} mime={mime_type} bytes={byte_len}]"),
        ArchivedProviderContent::RedactedThinking { byte_len, .. } => {
            format!("[redacted_thinking bytes={byte_len}]")
        }
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let mut out: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        out.push_str("...");
    }
    out
}
