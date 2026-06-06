/// Load recently used models from `~/.config/jfc/recent_models.json`.
pub fn load_recent_models() -> Vec<String> {
    let path = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("jfc")
        .join("recent_models.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Save recently used models (max 5, most recent first).
pub fn save_recent_models(models: &[String]) {
    let path = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("jfc")
        .join("recent_models.json");
    let capped: Vec<&String> = models.iter().take(5).collect();
    if let Ok(json) = serde_json::to_string(&capped) {
        let _ = std::fs::write(&path, json);
    }
}

/// Push a model to the front of the recent list (deduplicates).
pub fn push_recent_model(recent: &mut Vec<String>, model: &str) {
    recent.retain(|m| m != model);
    recent.insert(0, model.to_owned());
    recent.truncate(5);
    save_recent_models(recent);
}
