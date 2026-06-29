//! Session persistence: save/load chat transcripts to disk.
//!
//! | Submodule       | Contents                                              |
//! |-----------------|-------------------------------------------------------|
//! | `serialization` | `Serialized*` type definitions                        |
//! | `serialize`     | `serialize_*` — runtime → on-disk conversion          |
//! | `deserialize`   | `deserialize_*` — on-disk → runtime conversion        |
//! | `compaction`    | `coalesce_*`, `persistent_session_messages`, etc.     |
//! | `core`          | `save_session`, `load_session`, `set_session_title`   |

mod compaction;
mod context_reduction_state;
mod core;
mod deserialize;
mod entry_log;
pub(crate) mod serialization;
mod serialize;
mod store;

#[cfg(test)]
mod serialization_tests;

pub use context_reduction_state::{load_context_reduction_queue, save_context_reduction_queue};
pub use core::{
    load_session, load_session_with_model, save_session, set_post_save_hook, set_session_title,
};
pub use store::{
    AutosaveOutcome, AutosaveRequest, DefaultSessionStore, ListSessionsRequest, LoadedTranscript,
    SaveTranscriptRequest, SearchSessionsRequest, SessionStore, SessionTranscript,
    StoredSessionMessage, default_session_store,
};

pub(crate) use deserialize::deserialize_message;
pub(crate) use serialization::SerializedMessage;
pub(crate) use serialize::serialize_message;
