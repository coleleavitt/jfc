pub(crate) mod input;
pub(crate) mod tick;
pub(crate) mod ui_actions;

// Engine handlers (stream/tool/task/team/compaction/workflow/provider) are
// re-exported so the pump's `handlers::x::y` call paths survive stage 5.
pub use jfc_engine::runtime::event_loop::handlers::*;
