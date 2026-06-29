//! Loader for ~/.claude/keybindings.json — user-global custom key bindings.
//!
//! CC 2.1.167 reads `~/.claude/keybindings.json` at startup. The file has the
//! shape `{"bindings": [...]}` where each binding is a block object. JFC loads
//! this non-fatally (missing or malformed file produces a warning + empty list).

use std::path::PathBuf;

/// Returns the path to the keybindings file: `~/.claude/keybindings.json`.
pub fn keybindings_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".claude").join("keybindings.json"))
}

/// Load custom keybindings from `~/.claude/keybindings.json`.
///
/// Returns an empty `Vec` when the file is absent, empty, or malformed.
/// Emits a `tracing::warn` on parse errors so the user knows their config is
/// being ignored rather than quietly discarded.
pub fn load_keybindings() -> Vec<serde_json::Value> {
    let Some(path) = keybindings_path() else {
        return Vec::new();
    };
    let raw = match std::fs::read_to_string(&path) {
        Ok(r) => r,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(err) => {
            tracing::warn!(
                target: "jfc::config::keybindings",
                path = %path.display(),
                error = %err,
                "failed to read keybindings.json — using empty keybindings"
            );
            return Vec::new();
        }
    };
    let value: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(err) => {
            tracing::warn!(
                target: "jfc::config::keybindings",
                path = %path.display(),
                error = %err,
                "keybindings.json must contain valid JSON — using empty keybindings"
            );
            return Vec::new();
        }
    };
    match value.get("bindings").and_then(|b| b.as_array()) {
        Some(bindings) => bindings.clone(),
        None => {
            tracing::warn!(
                target: "jfc::config::keybindings",
                path = %path.display(),
                "keybindings.json must have a \"bindings\" array — using empty keybindings"
            );
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_keybindings_missing_file_returns_empty_normal() {
        // Non-existent path — should return empty without panicking.
        let result = std::panic::catch_unwind(load_keybindings);
        assert!(result.is_ok());
    }

    #[test]
    fn parse_valid_keybindings_normal() {
        let json = r#"{"bindings": [{"key": "ctrl+k", "action": "clear"}]}"#;
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        let bindings = value["bindings"].as_array().unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0]["key"], "ctrl+k");
    }

    #[test]
    fn parse_missing_bindings_key_returns_empty_robust() {
        let json = r#"{"keys": []}"#;
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        // No "bindings" key — simulates the malformed-file path.
        assert!(value.get("bindings").and_then(|b| b.as_array()).is_none());
    }
}
