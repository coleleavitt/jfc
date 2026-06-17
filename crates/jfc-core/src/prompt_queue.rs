use std::collections::VecDeque;
use std::time::Instant;

use crate::Attachment;

pub const DEFERRED_TOOL_USES_CAP: usize = 64;
pub const TOOL_USE_SUMMARIES_CAP: usize = 32;

/// Priority levels for the prompt queue. Higher priority prompts are drained
/// first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum QueuePriority {
    /// Drain at end of turn.
    Later = 0,
    /// Drain between tool batches.
    Next = 1,
    /// Immediate; jump the queue.
    Now = 2,
}

#[derive(Debug, Clone)]
pub struct QueuedPrompt {
    pub text: String,
    pub is_meta: bool,
    pub priority: QueuePriority,
    pub attachments: Vec<Attachment>,
}

pub fn queued_prompt_placeholder(text: &str, is_meta: bool) -> String {
    let prefix = if is_meta {
        "[command queued]"
    } else {
        "[queued]"
    };
    format!("{prefix} {text}")
}

#[derive(Debug, Clone)]
pub struct DeferredToolUse {
    pub id: String,
    pub name: String,
    pub input_preview: String,
    pub reason: String,
    pub queued_at: Instant,
}

#[derive(Debug, Clone)]
pub struct ToolUseSummary {
    pub summary: String,
    pub preceding_tool_use_ids: Vec<String>,
    pub created_at: Instant,
}

/// Append `item` to the back of a bounded FIFO queue, dropping the oldest
/// entries until `queue.len() <= cap`.
///
/// This is the Rust implementation of the proved `push_deferred` shape in
/// `rcoq-tests/theorems/MessageQueue.v`: append first, then evict from the
/// front when the cap would be exceeded. A cap of zero intentionally retains
/// nothing.
pub fn push_bounded_drop_oldest<T>(queue: &mut VecDeque<T>, cap: usize, item: T) {
    queue.push_back(item);
    while queue.len() > cap {
        queue.pop_front();
    }
}

/// Whether a queued prompt should be preserved across active compaction.
///
/// Matches the proved `should_preserve`: when compaction is active, `Now` and
/// `Next` survive, while `Later` may be deferred. With no compaction, every
/// prompt is preserved.
pub fn should_preserve_prompt(prompt: &QueuedPrompt, compaction_active: bool) -> bool {
    !compaction_active || prompt.priority >= QueuePriority::Next
}

/// Priority-based prompt queue. Higher priority prompts are popped first, and
/// prompts with the same priority preserve FIFO order.
#[derive(Debug, Clone, Default)]
pub struct MessageQueue {
    entries: VecDeque<QueuedPrompt>,
}

impl MessageQueue {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::new(),
        }
    }

    pub fn push(&mut self, prompt: QueuedPrompt) {
        self.entries.push_back(prompt);
    }

    pub fn push_later(&mut self, text: String, is_meta: bool, attachments: Vec<Attachment>) {
        self.entries.push_back(QueuedPrompt {
            text,
            is_meta,
            priority: QueuePriority::Later,
            attachments,
        });
    }

    pub fn pop_max_priority(&mut self) -> Option<QueuedPrompt> {
        let max_priority = self.entries.iter().map(|entry| entry.priority).max()?;
        let max_idx = self
            .entries
            .iter()
            .position(|entry| entry.priority == max_priority)?;
        self.entries.remove(max_idx)
    }

    pub fn drain_at_least(&mut self, min_priority: QueuePriority) -> Vec<QueuedPrompt> {
        let mut drained = Vec::new();
        let mut remaining = VecDeque::new();
        for entry in self.entries.drain(..) {
            if entry.priority >= min_priority {
                drained.push(entry);
            } else {
                remaining.push_back(entry);
            }
        }
        self.entries = remaining;
        drained.sort_by_key(|entry| std::cmp::Reverse(entry.priority));
        drained
    }

    pub fn drain_all(&mut self) -> Vec<QueuedPrompt> {
        let mut drained: Vec<QueuedPrompt> = self.entries.drain(..).collect();
        drained.sort_by_key(|entry| std::cmp::Reverse(entry.priority));
        drained
    }

    pub fn pop_back(&mut self) -> Option<QueuedPrompt> {
        self.entries.pop_back()
    }

    pub fn pop_front(&mut self) -> Option<QueuedPrompt> {
        self.entries.pop_front()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn get(&self, index: usize) -> Option<&QueuedPrompt> {
        self.entries.get(index)
    }

    pub fn iter(&self) -> impl Iterator<Item = &QueuedPrompt> {
        self.entries.iter()
    }
}

impl std::ops::Index<usize> for MessageQueue {
    type Output = QueuedPrompt;

    fn index(&self, index: usize) -> &Self::Output {
        &self.entries[index]
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::{
        MessageQueue, QueuePriority, QueuedPrompt, push_bounded_drop_oldest, should_preserve_prompt,
    };

    fn prompt(text: &str, priority: QueuePriority) -> QueuedPrompt {
        QueuedPrompt {
            text: text.to_owned(),
            is_meta: false,
            priority,
            attachments: Vec::new(),
        }
    }

    #[test]
    fn pop_max_priority_preserves_fifo_for_equal_priority() {
        let mut queue = MessageQueue::new();
        queue.push(prompt("first", QueuePriority::Next));
        queue.push(prompt("second", QueuePriority::Next));
        queue.push(prompt("later", QueuePriority::Later));

        assert_eq!(
            queue.pop_max_priority().map(|prompt| prompt.text),
            Some("first".to_owned())
        );
        assert_eq!(
            queue.pop_max_priority().map(|prompt| prompt.text),
            Some("second".to_owned())
        );
        assert_eq!(
            queue.pop_max_priority().map(|prompt| prompt.text),
            Some("later".to_owned())
        );
    }

    #[test]
    fn drain_all_orders_by_priority_and_fifo_within_priority_normal() {
        let mut queue = MessageQueue::new();
        queue.push(prompt("later-1", QueuePriority::Later));
        queue.push(prompt("now-1", QueuePriority::Now));
        queue.push(prompt("next-1", QueuePriority::Next));
        queue.push(prompt("now-2", QueuePriority::Now));
        queue.push(prompt("next-2", QueuePriority::Next));

        let drained: Vec<_> = queue
            .drain_all()
            .into_iter()
            .map(|prompt| prompt.text)
            .collect();
        assert_eq!(
            drained,
            vec!["now-1", "now-2", "next-1", "next-2", "later-1"]
        );
        assert!(queue.is_empty());
    }

    #[test]
    fn drain_at_least_partitions_and_preserves_remaining_robust() {
        let mut queue = MessageQueue::new();
        queue.push(prompt("later", QueuePriority::Later));
        queue.push(prompt("now", QueuePriority::Now));
        queue.push(prompt("next", QueuePriority::Next));

        let drained: Vec<_> = queue
            .drain_at_least(QueuePriority::Next)
            .into_iter()
            .map(|prompt| prompt.text)
            .collect();
        assert_eq!(drained, vec!["now", "next"]);
        assert_eq!(queue.len(), 1);
        assert_eq!(
            queue.get(0).map(|prompt| prompt.text.as_str()),
            Some("later")
        );
    }

    #[test]
    fn bounded_push_drops_oldest_robust() {
        let mut queue = VecDeque::from([1, 2, 3]);
        push_bounded_drop_oldest(&mut queue, 3, 4);
        assert_eq!(queue.into_iter().collect::<Vec<_>>(), vec![2, 3, 4]);

        let mut queue = VecDeque::from([1, 2]);
        push_bounded_drop_oldest(&mut queue, 0, 3);
        assert!(queue.is_empty());
    }

    #[test]
    fn high_priority_prompts_survive_compaction_normal() {
        assert!(should_preserve_prompt(
            &prompt("now", QueuePriority::Now),
            true
        ));
        assert!(should_preserve_prompt(
            &prompt("next", QueuePriority::Next),
            true
        ));
        assert!(!should_preserve_prompt(
            &prompt("later", QueuePriority::Later),
            true
        ));
        assert!(should_preserve_prompt(
            &prompt("later", QueuePriority::Later),
            false
        ));
    }
}
