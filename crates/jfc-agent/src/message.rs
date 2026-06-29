//! Unified message bus.
//!
//! Replaces four parallel message paths that all delivered "a message to an
//! agent":
//! - the swarm `TeammateEvent` in-memory channel,
//! - `mailbox.rs` per-agent JSON inbox files,
//! - the daemon's `BackgroundAgentCompletion` events,
//! - the economy's `SwarmProvider::send_message` log-only path.
//!
//! [`MessageBus`] keeps an in-memory fast path (a `parking_lot::Mutex` over a
//! per-agent queue) for same-process agents, and a file-backed inbox directory
//! for cross-process delivery (detached daemon workers). Callers use one API;
//! the bus decides the transport based on whether the recipient is registered
//! in-process.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use fs2::FileExt;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::id::AgentId;

/// A single message delivered to an agent's inbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Who sent it (`None` for system / user-origin messages).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<AgentId>,
    /// The message body.
    pub text: String,
    /// Unix-millis timestamp of delivery.
    pub sent_at_ms: u64,
}

impl Message {
    /// Build a system/user-origin message (no sender agent).
    pub fn new(text: impl Into<String>) -> Self {
        let _linkscope_message = linkscope::phase("agent.message.new");
        let text = text.into();
        linkscope::detail_event_fields(
            "agent.message.new",
            [linkscope::TraceField::bytes(
                "text_bytes",
                u64::try_from(text.len()).unwrap_or(u64::MAX),
            )],
        );
        Self {
            from: None,
            text,
            sent_at_ms: now_ms(),
        }
    }

    /// Build a message from a specific agent.
    pub fn from_agent(from: AgentId, text: impl Into<String>) -> Self {
        let _linkscope_message = linkscope::phase("agent.message.from_agent");
        let text = text.into();
        linkscope::detail_event_fields(
            "agent.message.from_agent",
            [
                linkscope::TraceField::text("from", from.to_string()),
                linkscope::TraceField::bytes(
                    "text_bytes",
                    u64::try_from(text.len()).unwrap_or(u64::MAX),
                ),
            ],
        );
        Self {
            from: Some(from),
            text,
            sent_at_ms: now_ms(),
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Errors raised while delivering or reading messages.
#[derive(Debug, thiserror::Error)]
pub enum MessageError {
    #[error("io error on inbox file: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to (de)serialize message: {0}")]
    Serde(#[from] serde_json::Error),
}

/// The unified delivery hub.
///
/// In-process recipients get an in-memory queue; everyone else falls through to
/// a JSON file under `inbox_dir`. `poll` drains both, so a recipient never has
/// to know which transport its sender used.
pub struct MessageBus {
    /// In-memory queues for same-process agents, keyed by recipient.
    inboxes: Mutex<HashMap<AgentId, Vec<Message>>>,
    /// Directory holding `<uuid>.json` inbox files for cross-process delivery.
    inbox_dir: PathBuf,
}

impl MessageBus {
    /// Create a bus rooted at `inbox_dir` (created on first file write).
    pub fn new(inbox_dir: impl Into<PathBuf>) -> Self {
        let _linkscope_new = linkscope::phase("agent.message_bus.new");
        let inbox_dir = inbox_dir.into();
        linkscope::event_fields(
            "agent.message_bus.new",
            [linkscope::TraceField::text(
                "inbox_dir",
                inbox_dir.display().to_string(),
            )],
        );
        Self {
            inboxes: Mutex::new(HashMap::new()),
            inbox_dir,
        }
    }

    /// Register an in-process recipient so its messages stay in memory.
    ///
    /// Idempotent: re-registering an existing agent keeps any queued messages.
    pub fn register(&self, agent: AgentId) {
        let _linkscope_register = linkscope::phase("agent.message_bus.register");
        linkscope::event_fields(
            "agent.message_bus.register",
            [linkscope::TraceField::text("agent", agent.to_string())],
        );
        self.inboxes.lock().entry(agent).or_default();
    }

    /// Drop an in-process recipient (e.g. when a teammate terminates).
    ///
    /// Returns any messages still queued, so the caller can flush them to the
    /// file inbox if delivery must survive.
    pub fn deregister(&self, agent: &AgentId) -> Vec<Message> {
        let _linkscope_deregister = linkscope::phase("agent.message_bus.deregister");
        let messages = self.inboxes.lock().remove(agent).unwrap_or_default();
        linkscope::event_fields(
            "agent.message_bus.deregister.result",
            [
                linkscope::TraceField::text("agent", agent.to_string()),
                linkscope::TraceField::count(
                    "leftover",
                    u64::try_from(messages.len()).unwrap_or(u64::MAX),
                ),
            ],
        );
        messages
    }

    /// Send `msg` to `to`. In-process recipients get the in-memory queue; all
    /// others are appended to their file inbox.
    pub fn send(&self, to: &AgentId, msg: Message) -> Result<(), MessageError> {
        let _linkscope_send = linkscope::phase("agent.message_bus.send");
        {
            let mut guard = self.inboxes.lock();
            if let Some(queue) = guard.get_mut(to) {
                queue.push(msg);
                linkscope::event_fields(
                    "agent.message_bus.send.result",
                    [
                        linkscope::TraceField::text("to", to.to_string()),
                        linkscope::TraceField::text("transport", "memory"),
                        linkscope::TraceField::count(
                            "queue_depth",
                            u64::try_from(queue.len()).unwrap_or(u64::MAX),
                        ),
                    ],
                );
                return Ok(());
            }
        }
        linkscope::event_fields(
            "agent.message_bus.send.result",
            [
                linkscope::TraceField::text("to", to.to_string()),
                linkscope::TraceField::text("transport", "file"),
            ],
        );
        self.append_file_inbox(to, msg)
    }

    /// Drain every pending message for `agent` (both transports), oldest first.
    pub fn poll(&self, agent: &AgentId) -> Vec<Message> {
        let _linkscope_poll = linkscope::phase("agent.message_bus.poll");
        let mut out = {
            let mut guard = self.inboxes.lock();
            match guard.get_mut(agent) {
                Some(queue) => std::mem::take(queue),
                None => Vec::new(),
            }
        };
        if let Ok(mut from_file) = self.drain_file_inbox(agent) {
            out.append(&mut from_file);
        }
        out.sort_by_key(|m| m.sent_at_ms);
        linkscope::event_fields(
            "agent.message_bus.poll.result",
            [
                linkscope::TraceField::text("agent", agent.to_string()),
                linkscope::TraceField::count(
                    "messages",
                    u64::try_from(out.len()).unwrap_or(u64::MAX),
                ),
            ],
        );
        out
    }

    /// Pop a single message for `agent`, preferring the in-memory queue.
    pub fn recv(&self, agent: &AgentId) -> Option<Message> {
        let _linkscope_recv = linkscope::phase("agent.message_bus.recv");
        {
            let mut guard = self.inboxes.lock();
            if let Some(queue) = guard.get_mut(agent) {
                if !queue.is_empty() {
                    linkscope::event_fields(
                        "agent.message_bus.recv.result",
                        [
                            linkscope::TraceField::text("agent", agent.to_string()),
                            linkscope::TraceField::text("transport", "memory"),
                            linkscope::TraceField::count("hit", 1),
                        ],
                    );
                    return Some(queue.remove(0));
                }
            }
        }
        let message = self.pop_file_inbox(agent).ok().flatten();
        linkscope::event_fields(
            "agent.message_bus.recv.result",
            [
                linkscope::TraceField::text("agent", agent.to_string()),
                linkscope::TraceField::text("transport", "file"),
                linkscope::TraceField::count("hit", u64::from(message.is_some())),
            ],
        );
        message
    }

    fn inbox_path(&self, agent: &AgentId) -> PathBuf {
        self.inbox_dir.join(format!("{}.json", agent.uuid()))
    }

    fn append_file_inbox(&self, to: &AgentId, msg: Message) -> Result<(), MessageError> {
        let _linkscope_append = linkscope::phase("agent.message_bus.append_file_inbox");
        let _lock = self.lock_file_inboxes()?;
        let path = self.inbox_path(to);
        let mut existing = read_inbox_file(&path)?;
        existing.push(msg);
        write_inbox_file(&path, &existing)?;
        linkscope::record_items(
            "agent.message_bus.file_depth",
            u64::try_from(existing.len()).unwrap_or(u64::MAX),
        );
        Ok(())
    }

    fn drain_file_inbox(&self, agent: &AgentId) -> Result<Vec<Message>, MessageError> {
        let _linkscope_drain = linkscope::phase("agent.message_bus.drain_file_inbox");
        let _lock = self.lock_file_inboxes()?;
        let path = self.inbox_path(agent);
        let messages = read_inbox_file(&path)?;
        if !messages.is_empty() {
            // Truncate the inbox now that we've taken ownership.
            write_inbox_file(&path, &[])?;
        }
        linkscope::record_items(
            "agent.message_bus.file_drained",
            u64::try_from(messages.len()).unwrap_or(u64::MAX),
        );
        Ok(messages)
    }

    fn pop_file_inbox(&self, agent: &AgentId) -> Result<Option<Message>, MessageError> {
        let _linkscope_pop = linkscope::phase("agent.message_bus.pop_file_inbox");
        let _lock = self.lock_file_inboxes()?;
        let path = self.inbox_path(agent);
        let mut messages = read_inbox_file(&path)?;
        if messages.is_empty() {
            return Ok(None);
        }
        let first = messages.remove(0);
        write_inbox_file(&path, &messages)?;
        Ok(Some(first))
    }

    fn lock_file_inboxes(&self) -> Result<File, MessageError> {
        let _linkscope_lock = linkscope::phase("agent.message_bus.lock_file_inboxes");
        std::fs::create_dir_all(&self.inbox_dir)?;
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .truncate(false)
            .write(true)
            .open(self.inbox_dir.join(".inbox.lock"))?;
        lock.lock_exclusive()?;
        Ok(lock)
    }
}

fn read_inbox_file(path: &Path) -> Result<Vec<Message>, MessageError> {
    match std::fs::read_to_string(path) {
        Ok(raw) if raw.trim().is_empty() => Ok(Vec::new()),
        Ok(raw) => Ok(serde_json::from_str(&raw)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(MessageError::Io(e)),
    }
}

fn write_inbox_file(path: &Path, messages: &[Message]) -> Result<(), MessageError> {
    let json = serde_json::to_string_pretty(messages)?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("inbox");
    let tmp = parent.join(format!(
        ".{file_name}.tmp.{}.{}",
        std::process::id(),
        now_ms()
    ));
    {
        let mut file = File::create(&tmp)?;
        file.write_all(json.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_process_send_and_poll_normal() {
        let dir = tempfile::tempdir().unwrap();
        let bus = MessageBus::new(dir.path());
        let agent = AgentId::named("researcher");
        bus.register(agent.clone());

        bus.send(&agent, Message::new("hello")).unwrap();
        bus.send(&agent, Message::new("world")).unwrap();

        let msgs = bus.poll(&agent);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].text, "hello");
        assert_eq!(msgs[1].text, "world");
        // Drained.
        assert!(bus.poll(&agent).is_empty());
    }

    #[test]
    fn cross_process_falls_through_to_file_normal() {
        let dir = tempfile::tempdir().unwrap();
        let bus = MessageBus::new(dir.path());
        // NOT registered in-process → file transport.
        let agent = AgentId::named("detached-worker");

        bus.send(&agent, Message::new("from parent")).unwrap();
        // A fresh bus over the same dir (simulating another process) sees it.
        let bus2 = MessageBus::new(dir.path());
        let msgs = bus2.poll(&agent);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text, "from parent");
    }

    #[test]
    fn file_transport_recv_keeps_leftovers_robust() {
        let dir = tempfile::tempdir().unwrap();
        let agent = AgentId::named("detached-worker");
        let sender_a = MessageBus::new(dir.path());
        let sender_b = MessageBus::new(dir.path());

        sender_a.send(&agent, Message::new("one")).unwrap();
        sender_b.send(&agent, Message::new("two")).unwrap();

        let receiver = MessageBus::new(dir.path());
        assert_eq!(receiver.recv(&agent).unwrap().text, "one");
        let rest = MessageBus::new(dir.path()).poll(&agent);
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].text, "two");
    }

    #[test]
    fn recv_pops_single_message_robust() {
        let dir = tempfile::tempdir().unwrap();
        let bus = MessageBus::new(dir.path());
        let agent = AgentId::named("a");
        bus.register(agent.clone());
        bus.send(&agent, Message::new("one")).unwrap();
        bus.send(&agent, Message::new("two")).unwrap();

        assert_eq!(bus.recv(&agent).unwrap().text, "one");
        assert_eq!(bus.recv(&agent).unwrap().text, "two");
        assert!(bus.recv(&agent).is_none());
    }

    #[test]
    fn deregister_returns_pending_robust() {
        let dir = tempfile::tempdir().unwrap();
        let bus = MessageBus::new(dir.path());
        let agent = AgentId::named("leaving");
        bus.register(agent.clone());
        bus.send(&agent, Message::new("unread")).unwrap();

        let leftover = bus.deregister(&agent);
        assert_eq!(leftover.len(), 1);
        assert_eq!(leftover[0].text, "unread");
    }

    #[test]
    fn poll_merges_both_transports_robust() {
        let dir = tempfile::tempdir().unwrap();
        let bus = MessageBus::new(dir.path());
        let agent = AgentId::named("dual");

        // File message arrives first (lower timestamp), before registration.
        bus.send(
            &agent,
            Message {
                from: None,
                text: "from-file".into(),
                sent_at_ms: 100,
            },
        )
        .unwrap();

        bus.register(agent.clone());
        bus.send(
            &agent,
            Message {
                from: None,
                text: "in-memory".into(),
                sent_at_ms: 200,
            },
        )
        .unwrap();

        let msgs = bus.poll(&agent);
        assert_eq!(msgs.len(), 2);
        // Sorted by sent_at_ms: file (100) then memory (200).
        assert_eq!(msgs[0].text, "from-file");
        assert_eq!(msgs[1].text, "in-memory");
    }
}
