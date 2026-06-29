use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    Stdio,
    Http,
}

impl TransportKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Http => "http",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SpawnConfig {
    pub server_name: String,
    pub kind: TransportKind,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub url: Option<String>,
    pub cwd: Option<std::path::PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_kind_label() {
        assert_eq!(TransportKind::Stdio.label(), "stdio");
        assert_eq!(TransportKind::Http.label(), "http");
    }
}
