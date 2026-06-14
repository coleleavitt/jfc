//! CCR backends — only the in-memory store is ported into jfc (the
//! headroom SQLite/Redis backends are proxy-deployment concerns).

pub mod in_memory;

pub use in_memory::InMemoryCcrStore;
