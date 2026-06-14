//! Strongly-typed identifier newtypes.
//!
//! Mirrors the t-libs-api guidance: domain ids should be newtypes so the
//! type checker prevents accidentally passing a `task_id` where a
//! `tool_use_id` was expected. Each is a transparent wrapper around
//! `String` (`#[serde(transparent)]`) so the JSON wire format is
//! unchanged — old session files still load.
//!
//! Each newtype provides:
//! - `new(impl Into<String>)` constructor
//! - `as_str(&self) -> &str` accessor
//! - `AsRef<str>` for ergonomic interop with byte/string APIs
//! - `Borrow<str>` so `HashMap<Id, _>` can be looked up by `&str`
//! - `From<String>` and `From<&str>` so existing literal-construction
//!   sites compile cheaply
//! - `Display` (delegates to inner)
//! - `PartialEq<&str>` (and the symmetric `&str == Id`) so the legions
//!   of `tc.id == tool_id` comparisons through the codebase keep
//!   working without forcing every call site into `tc.id.as_str()`.
//!
//! Module-qualified names (`ids::TaskId`) intentionally avoid collision
//! with the existing `tasks::TaskId` (v126 todo store) and the
//! feature-gated `background::AgentId(u64)` — those are unrelated
//! domains that happened to grab the obvious names first.

#![allow(dead_code)]

use std::borrow::Borrow;
use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! id_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(s: impl Into<String>) -> Self {
                Self(s.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Consumes the id and returns the wrapped `String`. Useful
            /// when round-tripping through provider APIs that haven't
            /// adopted the typed id yet.
            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl From<&String> for $name {
            fn from(value: &String) -> Self {
                Self(value.clone())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool {
                self.0 == other
            }
        }

        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool {
                self.0 == *other
            }
        }

        impl PartialEq<String> for $name {
            fn eq(&self, other: &String) -> bool {
                &self.0 == other
            }
        }

        impl PartialEq<$name> for str {
            fn eq(&self, other: &$name) -> bool {
                self == other.0
            }
        }

        impl PartialEq<$name> for &str {
            fn eq(&self, other: &$name) -> bool {
                *self == other.0
            }
        }

        impl PartialEq<$name> for String {
            fn eq(&self, other: &$name) -> bool {
                self == &other.0
            }
        }
    };
}

id_newtype! {
    /// Stable identity of a single tool invocation. Issued by the
    /// provider (`tool_use_id` on Anthropic, `tool_call_id` on
    /// OpenAI-style) and threaded through `ToolCall`, `ToolResult`,
    /// streaming chunks, and tool-cancel events.
    ToolId
}

id_newtype! {
    /// Stable identity of a streaming subagent / runner task. Drives
    /// `BackgroundTask` lookup and `TaskStatusPart` routing. Distinct
    /// from `tasks::TaskId` (the v126 `/task-*` todo store), which
    /// happens to share the bare name in a different module.
    TaskId
}

id_newtype! {
    /// Active session identity. Persisted to disk under
    /// `~/.config/jfc/sessions/<id>.json`; the on-disk format stays a
    /// bare string thanks to `#[serde(transparent)]`.
    SessionId
}

id_newtype! {
    /// Identity of a swarm/teammate agent within a session. Distinct
    /// from the feature-gated `background::AgentId(u64)`, which is a
    /// numeric handle in a deferred manager.
    AgentId
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_id_round_trips_as_bare_string() {
        let id = ToolId::new("call_abc");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"call_abc\"");
        let back: ToolId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn id_compares_with_str() {
        let id = TaskId::new("t1");
        assert!(id == "t1");
        assert!("t1" == id);
        assert!(id != "t2");
    }

    #[test]
    fn id_borrows_as_str_for_hashmap_lookup() {
        use std::collections::HashMap;
        let mut map: HashMap<SessionId, u32> = HashMap::new();
        map.insert(SessionId::new("ses_1"), 7);
        // Lookup by &str works because of the Borrow<str> impl.
        assert_eq!(map.get("ses_1"), Some(&7));
    }

    #[test]
    fn agent_id_display_delegates_to_inner() {
        let id = AgentId::from("agent-42");
        assert_eq!(format!("{id}"), "agent-42");
    }
}
