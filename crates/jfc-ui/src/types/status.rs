#[derive(Clone, Copy, Debug, PartialEq)]
pub enum McpStatus {
    #[allow(dead_code)]
    Connected,
    #[allow(dead_code)]
    Disabled,
    #[allow(dead_code)]
    Error,
}

impl McpStatus {
    #[allow(dead_code)]
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
    #[allow(dead_code)]
    Active,
    #[allow(dead_code)]
    Inactive,
}

#[derive(Clone, Debug)]
pub struct LspServerInfo {
    pub name: String,
    pub status: LspStatus,
}
