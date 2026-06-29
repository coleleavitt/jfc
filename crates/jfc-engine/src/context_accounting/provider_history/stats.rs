use jfc_provider::{ProviderContent, ProviderMessage, ProviderRole};

#[derive(Default)]
pub(super) struct HistoryStats {
    pub(super) user_messages: usize,
    pub(super) assistant_messages: usize,
    pub(super) text_blocks: usize,
    pub(super) tool_use_blocks: usize,
    pub(super) tool_result_blocks: usize,
    pub(super) attachment_blocks: usize,
    pub(super) thinking_blocks: usize,
}

impl HistoryStats {
    pub(super) fn from_messages(messages: &[ProviderMessage]) -> Self {
        let mut stats = Self::default();
        for message in messages {
            match message.role {
                ProviderRole::User => stats.user_messages += 1,
                ProviderRole::Assistant => stats.assistant_messages += 1,
            }
            for content in &message.content {
                stats.add_content(content);
            }
        }
        stats
    }

    fn add_content(&mut self, content: &ProviderContent) {
        match content {
            ProviderContent::Text(_) => self.text_blocks += 1,
            ProviderContent::Thinking { .. } | ProviderContent::RedactedThinking { .. } => {
                self.thinking_blocks += 1;
            }
            ProviderContent::ToolResult { .. } | ProviderContent::ServerToolResult { .. } => {
                self.tool_result_blocks += 1;
            }
            ProviderContent::ToolUse { .. } | ProviderContent::ServerToolUse { .. } => {
                self.tool_use_blocks += 1;
            }
            ProviderContent::Attachment(_) => self.attachment_blocks += 1,
        }
    }
}
