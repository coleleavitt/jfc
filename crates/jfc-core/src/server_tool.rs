//! Server-side tool result discrimination, shared across the provider wire
//! layer and the domain `ToolOutput` type. Lives in jfc-core so both
//! jfc-provider and downstream consumers can name it without a cycle.

/// Discriminates the wire `type` of a `ProviderContent::ServerToolResult`.
/// Each variant maps to a concrete Anthropic block type. `Other(String)`
/// catches future server-tool result shapes so a forward-rolled provider
/// doesn't drop the block on the floor — it round-trips with the same
/// `type` string it arrived with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerToolResultKind {
    /// `web_search_tool_result` — see cli.js v142:394261.
    WebSearch,
    /// `code_execution_tool_result` — see cli.js v142:246154.
    CodeExecution,
    /// `web_fetch_tool_result` — see cli.js v142:246159.
    WebFetch,
    /// `advisor_tool_result` — server-side stronger-model reviewer response.
    Advisor,
    /// Catch-all for unknown future shapes; preserves the wire `type`
    /// string so resends are still byte-faithful.
    Other(String),
}

impl ServerToolResultKind {
    /// Wire `type` field that Anthropic uses for this kind.
    pub fn wire_type(&self) -> &str {
        match self {
            Self::WebSearch => "web_search_tool_result",
            Self::CodeExecution => "code_execution_tool_result",
            Self::WebFetch => "web_fetch_tool_result",
            Self::Advisor => "advisor_tool_result",
            Self::Other(s) => s.as_str(),
        }
    }

    pub fn from_wire_type(s: &str) -> Self {
        match s {
            "web_search_tool_result" => Self::WebSearch,
            "code_execution_tool_result" => Self::CodeExecution,
            "web_fetch_tool_result" => Self::WebFetch,
            "advisor_tool_result" => Self::Advisor,
            other => Self::Other(other.to_owned()),
        }
    }
}
