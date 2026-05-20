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
mod core;
mod deserialize;
mod serialize;
mod serialization;

#[cfg(test)]
mod serialization_tests;

pub use core::{load_session, load_session_with_model, save_session, set_session_title};
