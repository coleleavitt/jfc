use jfc_config::McpServerConfig;

async fn register_background_worker_mcp_registry_from_config(
    configs: std::collections::HashMap<String, McpServerConfig>,
) -> (usize, usize) {
    let _linkscope_mcp = linkscope::phase("engine.worker.mcp.register_registry_from_config");
    let configured = configs.len();
    linkscope::event_fields(
        "engine.worker.mcp.config",
        [linkscope::TraceField::count(
            "configured",
            u64::try_from(configured).unwrap_or(u64::MAX),
        )],
    );
    let registry = crate::mcp::McpRegistry::new();
    crate::tools::register_mcp_registry(registry.clone());
    crate::mcp::register_servers_from_config(&registry, &configs).await;
    let active = registry.list_active().await.len();
    linkscope::event_fields(
        "engine.worker.mcp.result",
        [
            linkscope::TraceField::count(
                "configured",
                u64::try_from(configured).unwrap_or(u64::MAX),
            ),
            linkscope::TraceField::count("active", u64::try_from(active).unwrap_or(u64::MAX)),
        ],
    );
    (configured, active)
}

pub(super) async fn register_background_worker_mcp_registry() -> (usize, usize) {
    let _linkscope_mcp = linkscope::phase("engine.worker.mcp.register_registry");
    register_background_worker_mcp_registry_from_config(crate::config::load_arc().mcp.clone()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[serial_test::serial]
    async fn background_worker_mcp_bootstrap_installs_registry_normal() {
        let (configured, active) =
            register_background_worker_mcp_registry_from_config(std::collections::HashMap::new())
                .await;

        assert_eq!(configured, 0);
        assert_eq!(active, 0);
        assert!(crate::tools::snapshot_mcp_registry().is_some());
    }
}
