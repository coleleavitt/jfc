#[derive(Clone, Copy, Debug, PartialEq)]
pub enum McpStatus {
    Connected,
    Disabled,
    Error,
}

impl McpStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Connected => "Connected",
            Self::Disabled => "Disabled",
            Self::Error => "Error",
        }
    }
}

#[derive(Clone, Debug)]
pub struct McpServerInfo {
    pub name: String,
    pub status: McpStatus,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LspStatus {
    Active,
    Inactive,
}

#[derive(Clone, Debug)]
pub struct LspServerInfo {
    pub name: String,
    pub status: LspStatus,
}
