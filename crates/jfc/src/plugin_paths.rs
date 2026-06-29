use std::path::PathBuf;

pub(crate) fn plugins_root() -> anyhow::Result<PathBuf> {
    let root = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("could not resolve config directory"))?
        .join("jfc")
        .join("plugins");
    std::fs::create_dir_all(&root)?;
    Ok(root)
}

pub(crate) fn sanitize_plugin_name(name: &str) -> anyhow::Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        anyhow::bail!("invalid plugin name: {name:?}");
    }
    Ok(trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_rejects_path_traversal_robust() {
        assert!(sanitize_plugin_name("../x").is_err());
        assert!(sanitize_plugin_name("x/y").is_err());
        assert!(sanitize_plugin_name("ok-name_1.2").is_ok());
    }
}
