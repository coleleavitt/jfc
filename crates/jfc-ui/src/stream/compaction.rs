use futures::StreamExt;

use crate::provider::{
    Provider, ProviderContent, ProviderMessage, ProviderRole, StreamEvent, StreamOptions,
};

/// Mirrors v131 Claude Code's `Yd6 = 1e5` constant. Auto-compaction triggers
/// when the running subagent transcript crosses this many estimated tokens.
pub(crate) const SUBAGENT_AUTO_COMPACT_TOKEN_THRESHOLD: usize = 100_000;

/// Same chars-per-token heuristic v131 uses (`z_$ = 4`).
pub(crate) const BYTES_PER_TOKEN: usize = 4;

/// Verbatim summary prompt from v131 deob (`fd6` constant in
/// cli.2.1.131.beautified.js).
pub(crate) const SUBAGENT_AUTO_COMPACT_PROMPT: &str = "\
You have been working on the task described above but have not yet completed it. \
Write a continuation summary that will allow you (or another instance of yourself) \
to resume work efficiently in a future context window where the conversation history \
will be replaced with this summary. Your summary should be structured, concise, and \
actionable. Include:\n\
1. Task Overview\n\
The user's core request and success criteria\n\
Any clarifications or constraints they specified\n\
2. Current State\n\
What has been completed so far\n\
Files created, modified, or analyzed (with paths if relevant)\n\
Key outputs or artifacts produced\n\
3. Important Discoveries\n\
Technical constraints or requirements uncovered\n\
Decisions made and their rationale\n\
Errors encountered and how they were resolved\n\
What approaches were tried that didn't work (and why)\n\
4. Next Steps\n\
Specific actions needed to complete the task\n\
Any blockers or open questions to resolve\n\
\n\
Wrap the entire summary in <summary>...</summary> tags so it can be parsed.";

/// Render a provider message as plain text for inclusion in a summary request.
pub(crate) fn render_message_as_text(msg: &ProviderMessage) -> String {
    let role = match msg.role {
        ProviderRole::User => "user",
        ProviderRole::Assistant => "assistant",
    };
    let mut out = format!("[{role}] ");
    for c in &msg.content {
        match c {
            ProviderContent::Text(t) => out.push_str(t),
            ProviderContent::ToolUse { name, input, .. } => {
                let preview = serde_json::to_string(input)
                    .unwrap_or_default()
                    .chars()
                    .take(200)
                    .collect::<String>();
                out.push_str(&format!(
                    "\n  <tool_use name=\"{name}\" input=\"{preview}\"/>"
                ));
            }
            ProviderContent::ToolResult {
                content, is_error, ..
            } => {
                let head: String = content.chars().take(400).collect();
                let err = if *is_error { " error" } else { "" };
                out.push_str(&format!(
                    "\n  <tool_result{err} bytes=\"{}\">{head}…</tool_result>",
                    content.len()
                ));
            }
            ProviderContent::Attachment(att) => {
                out.push_str(&format!(
                    "\n  <attachment kind=\"{}\" bytes=\"{}\"/>",
                    att.kind.mime_type(),
                    att.bytes.len()
                ));
            }
        }
    }
    out
}

/// Pull the contents of a single `<summary>...</summary>` tag from the model's
/// reply.
pub(crate) fn extract_summary_tag(s: &str) -> Option<String> {
    let open = s.find("<summary>")?;
    let after_open = open + "<summary>".len();
    let close_rel = s[after_open..].find("</summary>")?;
    Some(s[after_open..after_open + close_rel].trim().to_owned())
}

/// Run an LLM-based summarization pass over the subagent's running history.
/// Returns true when the transcript was rewritten.
pub(crate) async fn auto_compact_subagent_history(
    messages: &mut Vec<ProviderMessage>,
    provider: &dyn Provider,
    model: crate::provider::ModelId,
) -> bool {
    let total_bytes: usize = messages.iter().map(estimate_provider_message_bytes).sum();
    let est_tokens = total_bytes / BYTES_PER_TOKEN;
    if est_tokens < SUBAGENT_AUTO_COMPACT_TOKEN_THRESHOLD {
        return false;
    }
    if messages.len() < 4 {
        return false;
    }

    const PRECOMPACT_MAX_CHARS: usize = 500;
    let preserve_start = messages.len().saturating_sub(2);
    for msg in messages.iter_mut().take(preserve_start) {
        for content in &mut msg.content {
            if let ProviderContent::ToolResult { content: c, .. } = content
                && c.len() > PRECOMPACT_MAX_CHARS
            {
                let boundary = c.floor_char_boundary(PRECOMPACT_MAX_CHARS);
                let total = c.len();
                *c = format!(
                    "{}… [pre-compact truncated, {} chars total]",
                    &c[..boundary],
                    total
                );
            }
        }
    }

    let to_summarize_end = messages.len().saturating_sub(2);
    let mut transcript = String::new();
    for msg in messages.iter().take(to_summarize_end).skip(1) {
        transcript.push_str(&render_message_as_text(msg));
        transcript.push_str("\n\n");
    }
    if transcript.trim().is_empty() {
        return false;
    }

    let original_task = messages
        .first()
        .map(render_message_as_text)
        .unwrap_or_default();

    let opts = StreamOptions::new(model)
        .system(SUBAGENT_AUTO_COMPACT_PROMPT.to_owned())
        .max_tokens(4_096);
    let summary_request = vec![ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::Text(format!(
            "Original task:\n{original_task}\n\nTranscript so far:\n{transcript}"
        ))],
    }];

    let stream = match provider.stream(summary_request, &opts).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(target: "jfc::stream", error = %e, "subagent compaction stream failed");
            return false;
        }
    };
    futures::pin_mut!(stream);
    let mut text = String::new();
    while let Some(ev) = stream.next().await {
        match ev {
            Ok(StreamEvent::TextDelta { delta, .. }) => text.push_str(&delta),
            Ok(StreamEvent::TextDone { text: t, .. }) => {
                if text.is_empty() {
                    text = t;
                }
            }
            Ok(StreamEvent::Error { message }) => {
                tracing::warn!(target: "jfc::stream", error = %message, "subagent compaction error");
                return false;
            }
            Err(e) => {
                tracing::warn!(target: "jfc::stream", error = %e, "subagent compaction stream error");
                return false;
            }
            _ => {}
        }
    }

    let summary = extract_summary_tag(&text).unwrap_or_else(|| text.trim().to_owned());
    if summary.trim().is_empty() {
        return false;
    }

    let summary_msg = ProviderMessage {
        role: ProviderRole::Assistant,
        content: vec![ProviderContent::Text(format!(
            "[earlier subagent turns auto-compacted to fit context]\n\n<summary>\n{summary}\n</summary>"
        ))],
    };
    messages.splice(1..to_summarize_end, std::iter::once(summary_msg));
    tracing::info!(
        target: "jfc::stream",
        est_tokens_before = est_tokens,
        summary_chars = summary.len(),
        new_msg_count = messages.len(),
        "subagent auto-compaction applied"
    );
    true
}

/// Soft cap on total request bytes for a subagent / teammate provider call.
pub(crate) const SUBAGENT_HISTORY_BUDGET_BYTES: usize = 500_000;

/// Rough byte count of a provider message used for budget enforcement.
pub(crate) fn estimate_provider_message_bytes(msg: &ProviderMessage) -> usize {
    msg.content
        .iter()
        .map(|c| match c {
            ProviderContent::Text(t) => t.len(),
            ProviderContent::ToolUse { name, input, .. } => {
                name.len() + serde_json::to_string(input).map(|s| s.len()).unwrap_or(0)
            }
            ProviderContent::ToolResult { content, .. } => content.len(),
            ProviderContent::Attachment(att) => att.bytes.len() * 4 / 3,
        })
        .sum::<usize>()
        + 16
}

/// Drop oldest assistant/tool-result pairs until the byte estimate fits.
pub(crate) fn cap_messages_for_budget(
    messages: &mut Vec<ProviderMessage>,
    max_bytes: usize,
) -> bool {
    let total: usize = messages.iter().map(estimate_provider_message_bytes).sum();
    if total <= max_bytes || messages.len() <= 1 {
        return false;
    }

    let mut running = total;
    let mut drop_until: usize = 1;
    while running > max_bytes && drop_until < messages.len() {
        running -= estimate_provider_message_bytes(&messages[drop_until]);
        drop_until += 1;
    }
    if drop_until > 1 {
        if let Some(last_dropped) = messages.get(drop_until.saturating_sub(1))
            && matches!(last_dropped.role, ProviderRole::Assistant)
            && drop_until < messages.len()
        {
            drop_until += 1;
        }
        messages.drain(1..drop_until);
        messages.insert(
            1,
            ProviderMessage {
                role: ProviderRole::Assistant,
                content: vec![ProviderContent::Text(
                    "[earlier subagent turns elided to fit the request budget — \
                     continuing from the most recent results]"
                        .to_owned(),
                )],
            },
        );
        true
    } else {
        false
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    fn user_text(s: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(s.to_owned())],
        }
    }
    fn assistant_text(s: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::Assistant,
            content: vec![ProviderContent::Text(s.to_owned())],
        }
    }
    fn assistant_tool_use(id: &str, name: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::Assistant,
            content: vec![ProviderContent::ToolUse {
                id: id.to_owned(),
                name: name.to_owned(),
                input: serde_json::json!({"path": "x"}),
            }],
        }
    }
    fn user_tool_result(id: &str, content: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::ToolResult {
                tool_use_id: id.to_owned(),
                content: content.to_owned(),
                is_error: false,
            }],
        }
    }

    #[test]
    fn cap_messages_under_budget_passes_through_normal() {
        let mut msgs = vec![user_text("hi"), assistant_text("hello"), user_text("ok")];
        let elided = cap_messages_for_budget(&mut msgs, 1_000_000);
        assert!(!elided);
        assert_eq!(msgs.len(), 3);
    }

    #[test]
    fn cap_messages_single_message_no_op_robust() {
        let mut msgs = vec![user_text("just one")];
        let elided = cap_messages_for_budget(&mut msgs, 0);
        assert!(!elided);
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn cap_messages_drops_oldest_pairs_keeps_prompt_and_tail_normal() {
        let big = "x".repeat(20_000);
        let mut msgs = vec![
            user_text("PROMPT"),
            assistant_tool_use("t1", "Read"),
            user_tool_result("t1", &big),
            assistant_tool_use("t2", "Read"),
            user_tool_result("t2", &big),
            assistant_tool_use("t3", "Read"),
            user_tool_result("t3", &big),
            assistant_text("recent assistant turn"),
        ];
        let elided = cap_messages_for_budget(&mut msgs, 25_000);
        assert!(elided, "should have truncated");
        match &msgs[0].content[0] {
            ProviderContent::Text(t) => assert_eq!(t, "PROMPT"),
            _ => panic!("expected prompt preserved"),
        }
        match msgs.last().unwrap().content[0] {
            ProviderContent::Text(ref t) => assert_eq!(t, "recent assistant turn"),
            _ => panic!("expected tail preserved"),
        }
    }

    #[test]
    fn cap_messages_inserts_truncation_marker_normal() {
        let big = "x".repeat(20_000);
        let mut msgs = vec![
            user_text("PROMPT"),
            assistant_tool_use("t1", "Read"),
            user_tool_result("t1", &big),
            assistant_text("recent"),
        ];
        cap_messages_for_budget(&mut msgs, 5_000);
        match &msgs[1].content[0] {
            ProviderContent::Text(t) => assert!(t.contains("elided")),
            _ => panic!("expected marker text"),
        }
        assert!(matches!(msgs[1].role, ProviderRole::Assistant));
    }

    #[test]
    fn cap_messages_drops_orphaned_tool_result_robust() {
        let big = "x".repeat(50_000);
        let mut msgs = vec![
            user_text("PROMPT"),
            assistant_tool_use("t1", "Read"),
            user_tool_result("t1", &big),
            assistant_text("recent"),
        ];
        cap_messages_for_budget(&mut msgs, 1_000);
        let has_orphan_tool_result = msgs.iter().any(|m| {
            m.content
                .iter()
                .any(|c| matches!(c, ProviderContent::ToolResult { .. }))
        });
        assert!(
            !has_orphan_tool_result,
            "tool_result left without its tool_use"
        );
    }

    #[test]
    fn estimate_provider_message_bytes_counts_each_variant_normal() {
        let t = estimate_provider_message_bytes(&user_text("abcde"));
        assert!(t >= 5 + 16, "got {t}");
        let tu = estimate_provider_message_bytes(&assistant_tool_use("id", "Read"));
        assert!(tu >= 4);
        let tr = estimate_provider_message_bytes(&user_tool_result("id", "x".repeat(100).as_str()));
        assert!(tr >= 100);
    }

    #[test]
    fn subagent_history_budget_constant_normal() {
        assert_eq!(SUBAGENT_HISTORY_BUDGET_BYTES, 500_000);
    }
}

#[cfg(test)]
mod auto_compact_tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::provider::{EventStream, ModelId, ModelInfo, StopReason};

    struct CannedSummaryProvider {
        reply: String,
        calls: Arc<Mutex<usize>>,
    }

    impl CannedSummaryProvider {
        fn new(reply: impl Into<String>) -> Self {
            Self {
                reply: reply.into(),
                calls: Arc::new(Mutex::new(0)),
            }
        }
    }

    #[async_trait::async_trait]
    impl Provider for CannedSummaryProvider {
        fn name(&self) -> &str {
            "canned"
        }
        fn available_models(&self) -> Vec<ModelInfo> {
            vec![ModelInfo::new("stub", "Stub", "canned")]
        }
        async fn stream(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            *self.calls.lock().unwrap() += 1;
            let events = vec![
                Ok(StreamEvent::TextDelta {
                    index: 0,
                    delta: self.reply.clone(),
                }),
                Ok(StreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                }),
            ];
            Ok(Box::pin(futures::stream::iter(events)))
        }
    }
    impl crate::provider::seal::Sealed for CannedSummaryProvider {}

    struct ErrorProvider;

    #[async_trait::async_trait]
    impl Provider for ErrorProvider {
        fn name(&self) -> &str {
            "error"
        }
        fn available_models(&self) -> Vec<ModelInfo> {
            vec![]
        }
        async fn stream(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            Err(anyhow::anyhow!("simulated stream failure"))
        }
    }
    impl crate::provider::seal::Sealed for ErrorProvider {}

    fn user_text(s: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(s.to_owned())],
        }
    }
    fn assistant_text(s: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::Assistant,
            content: vec![ProviderContent::Text(s.to_owned())],
        }
    }
    fn assistant_tool_use(id: &str, name: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::Assistant,
            content: vec![ProviderContent::ToolUse {
                id: id.to_owned(),
                name: name.to_owned(),
                input: serde_json::json!({"path": "x"}),
            }],
        }
    }
    fn user_tool_result(id: &str, content: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::ToolResult {
                tool_use_id: id.to_owned(),
                content: content.to_owned(),
                is_error: false,
            }],
        }
    }

    #[test]
    fn extract_summary_tag_finds_content_normal() {
        let s = "preamble <summary>core fact</summary> afterword";
        assert_eq!(extract_summary_tag(s), Some("core fact".to_owned()));
    }

    #[test]
    fn extract_summary_tag_missing_returns_none_robust() {
        assert_eq!(extract_summary_tag("no tags here"), None);
    }

    #[test]
    fn render_message_text_basic_normal() {
        let r = render_message_as_text(&user_text("hello"));
        assert!(r.starts_with("[user]"));
        assert!(r.contains("hello"));
    }

    #[test]
    fn render_message_text_tool_result_summarizes_body_normal() {
        let big = "x".repeat(10_000);
        let r = render_message_as_text(&user_tool_result("id1", &big));
        assert!(r.contains("bytes=\"10000\""));
        assert!(!r.contains(&"x".repeat(1_000)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn auto_compact_under_threshold_no_op_normal() {
        let provider = CannedSummaryProvider::new("<summary>x</summary>");
        let mut msgs = vec![user_text("PROMPT"), assistant_text("hi"), user_text("ok")];
        let did = auto_compact_subagent_history(&mut msgs, &provider, ModelId::new("stub")).await;
        assert!(!did);
        assert_eq!(msgs.len(), 3);
        assert_eq!(*provider.calls.lock().unwrap(), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn auto_compact_over_threshold_summarizes_normal() {
        let provider = CannedSummaryProvider::new(
            "<summary>The agent read three files and reported their structure.</summary>",
        );
        let big = "x".repeat(200_000);
        let mut msgs = vec![
            user_text("PROMPT"),
            assistant_tool_use("t1", "Read"),
            user_tool_result("t1", &big),
            assistant_tool_use("t2", "Read"),
            user_tool_result("t2", &big),
            assistant_tool_use("t3", "Read"),
            user_tool_result("t3", &big),
            assistant_text("recent assistant"),
            user_text("recent user"),
        ];
        let did = auto_compact_subagent_history(&mut msgs, &provider, ModelId::new("stub")).await;
        assert!(did, "expected compaction to fire");
        assert_eq!(msgs.len(), 4);
        match &msgs[0].content[0] {
            ProviderContent::Text(t) => assert_eq!(t, "PROMPT"),
            _ => panic!("prompt not preserved"),
        }
        match &msgs[1].content[0] {
            ProviderContent::Text(t) => {
                assert!(t.contains("auto-compacted"));
                assert!(t.contains("read three files"));
            }
            _ => panic!("expected summary message"),
        }
        assert_eq!(*provider.calls.lock().unwrap(), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn auto_compact_provider_error_no_op_robust() {
        let provider = ErrorProvider;
        let big = "x".repeat(200_000);
        let mut msgs = vec![
            user_text("PROMPT"),
            assistant_tool_use("t1", "Read"),
            user_tool_result("t1", &big),
            assistant_text("recent"),
            user_text("ok"),
        ];
        let original_len = msgs.len();
        let did = auto_compact_subagent_history(&mut msgs, &provider, ModelId::new("stub")).await;
        assert!(!did);
        assert_eq!(msgs.len(), original_len);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn auto_compact_empty_summary_no_op_robust() {
        let provider = CannedSummaryProvider::new("<summary>   </summary>");
        let big = "x".repeat(200_000);
        let mut msgs = vec![
            user_text("PROMPT"),
            assistant_tool_use("t1", "Read"),
            user_tool_result("t1", &big),
            assistant_tool_use("t2", "Read"),
            user_tool_result("t2", &big),
            assistant_text("recent"),
            user_text("ok"),
        ];
        let original_len = msgs.len();
        let did = auto_compact_subagent_history(&mut msgs, &provider, ModelId::new("stub")).await;
        assert!(!did);
        assert_eq!(msgs.len(), original_len);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn auto_compact_falls_back_to_raw_text_when_no_tags_robust() {
        let provider = CannedSummaryProvider::new("the agent did things and finished.");
        let big = "x".repeat(200_000);
        let mut msgs = vec![
            user_text("PROMPT"),
            assistant_tool_use("t1", "Read"),
            user_tool_result("t1", &big),
            assistant_tool_use("t2", "Read"),
            user_tool_result("t2", &big),
            assistant_text("recent"),
            user_text("ok"),
        ];
        let did = auto_compact_subagent_history(&mut msgs, &provider, ModelId::new("stub")).await;
        assert!(did);
        match &msgs[1].content[0] {
            ProviderContent::Text(t) => assert!(t.contains("the agent did things")),
            _ => panic!("expected summary body"),
        }
    }

    #[test]
    fn constants_match_v131_normal() {
        assert_eq!(SUBAGENT_AUTO_COMPACT_TOKEN_THRESHOLD, 100_000);
        assert_eq!(BYTES_PER_TOKEN, 4);
        assert!(SUBAGENT_AUTO_COMPACT_PROMPT.contains("Task Overview"));
        assert!(SUBAGENT_AUTO_COMPACT_PROMPT.contains("Current State"));
        assert!(SUBAGENT_AUTO_COMPACT_PROMPT.contains("Important Discoveries"));
        assert!(SUBAGENT_AUTO_COMPACT_PROMPT.contains("Next Steps"));
        assert!(SUBAGENT_AUTO_COMPACT_PROMPT.contains("<summary>"));
    }
}
