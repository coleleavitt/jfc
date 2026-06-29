use std::path::{Path, PathBuf};

use serial_test::serial;

use crate::runtime::ToolSource;
use crate::types::{ToolInput, ToolKind};
use crate::workflows::registry::plugin_discovery_options_for;

use super::{
    descriptor_catalog, execute_tool, register_discovered_plugin_tool_descriptors,
    reload_discovered_plugin_tool_descriptors,
};

#[tokio::test]
#[serial]
async fn discovered_manifest_process_bridge_tool_dispatches_through_execute_tool_normal() {
    descriptor_catalog::clear_external_tool_descriptors_for_tests();
    let dir = tempfile::tempdir().expect("temp dir");
    let plugin = dir.path().join("plugins").join("manifest-bridge");
    std::fs::create_dir_all(&plugin).expect("create plugin dir");
    let script = bridge_script(&plugin);
    write_manifest_tool(&plugin, &script);

    assert_eq!(
        register_discovered_plugin_tool_descriptors(plugin_discovery_options_for(dir.path()))
            .expect("register discovered tools"),
        1
    );
    let result = execute_tool(
        ToolKind::from_name("manifest_echo"),
        ToolInput::Generic {
            summary: "{}".to_owned(),
        },
        dir.path().to_path_buf(),
        None,
        None,
        None,
    )
    .await;
    descriptor_catalog::clear_external_tool_descriptors_for_tests();

    assert!(!result.is_error(), "{}", result.output);
    assert_eq!(result.output, "bridge ok");
    assert_eq!(
        result
            .provenance
            .as_ref()
            .map(|provenance| &provenance.source),
        Some(&ToolSource::Plugin {
            plugin_id: "manifest-bridge".to_owned()
        })
    );
}

#[tokio::test]
#[serial]
async fn discovered_process_bridge_describe_tool_dispatches_through_execute_tool_normal() {
    descriptor_catalog::clear_external_tool_descriptors_for_tests();
    let dir = tempfile::tempdir().expect("temp dir");
    let plugin = dir.path().join("plugins").join("described-bridge");
    std::fs::create_dir_all(&plugin).expect("create plugin dir");
    let script = describe_bridge_script(&plugin);
    write_bridge_manifest(&plugin, &script);

    assert_eq!(
        register_discovered_plugin_tool_descriptors(plugin_discovery_options_for(dir.path()))
            .expect("register discovered tools"),
        1
    );
    let result = execute_tool(
        ToolKind::from_name("described_echo"),
        ToolInput::Generic {
            summary: "{}".to_owned(),
        },
        dir.path().to_path_buf(),
        None,
        None,
        None,
    )
    .await;
    descriptor_catalog::clear_external_tool_descriptors_for_tests();

    assert!(!result.is_error(), "{}", result.output);
    assert_eq!(result.output, "bridge ok");
}

#[test]
#[serial]
fn reload_discovered_plugin_tool_descriptors_replaces_catalog_after_manifest_change_normal() {
    descriptor_catalog::clear_external_tool_descriptors_for_tests();
    let dir = tempfile::tempdir().expect("temp dir");
    let plugin = dir.path().join("plugins").join("reloadable");
    std::fs::create_dir_all(&plugin).expect("create plugin dir");
    write_manifest_external_tool(&plugin, "first_echo");

    let first = reload_discovered_plugin_tool_descriptors(plugin_discovery_options_for(dir.path()))
        .expect("first reload succeeds");
    let first_snapshot = descriptor_catalog::snapshot_external_tool_descriptors();

    write_manifest_external_tool(&plugin, "second_echo");
    let second =
        reload_discovered_plugin_tool_descriptors(plugin_discovery_options_for(dir.path()))
            .expect("second reload succeeds");
    let second_snapshot = descriptor_catalog::snapshot_external_tool_descriptors();
    descriptor_catalog::clear_external_tool_descriptors_for_tests();

    assert_eq!(first.before_count, 0);
    assert_eq!(first.after_count, 1);
    assert!(first.changed);
    assert_eq!(first_snapshot[0].name, "first_echo");
    assert_eq!(second.before_count, 1);
    assert_eq!(second.after_count, 1);
    assert!(second.changed);
    assert_ne!(first.after_digest, second.after_digest);
    assert_eq!(second_snapshot[0].name, "second_echo");
}

fn write_manifest_tool(plugin: &Path, script: &Path) {
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        format!(
            r#"[plugin]
name = "manifest-bridge"

[process_bridge]
command = "{}"

[[tools]]
name = "manifest_echo"
description = "manifest echo"
visibility = "model_visible"
input_schema = {{ type = "object" }}

[tools.executor]
kind = "process_bridge"
"#,
            script.file_name().and_then(|name| name.to_str()).unwrap()
        ),
    )
    .expect("write manifest");
}

fn write_manifest_external_tool(plugin: &Path, name: &str) {
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        format!(
            r#"[plugin]
name = "reloadable"

[[tools]]
name = "{name}"
description = "reloadable echo"
visibility = "model_visible"
input_schema = {{ type = "object" }}

[tools.executor]
kind = "process_bridge"
handler = "bridge.sh"
"#
        ),
    )
    .expect("write manifest");
}

fn write_bridge_manifest(plugin: &Path, script: &Path) {
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        format!(
            r#"[plugin]
name = "described-bridge"

[process_bridge]
command = "{}"
"#,
            script.file_name().and_then(|name| name.to_str()).unwrap()
        ),
    )
    .expect("write manifest");
}

fn bridge_script(dir: &Path) -> PathBuf {
    let script = dir.join("bridge.sh");
    std::fs::write(
        &script,
        r#"#!/bin/sh
read line
id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
printf '{"type":"response","id":"%s","response":{"kind":"tool_result","output":"bridge ok","is_error":false}}\n' "$id"
"#,
    )
    .expect("write bridge script");
    make_executable(&script);
    script
}

fn describe_bridge_script(dir: &Path) -> PathBuf {
    let script = dir.join("describe_bridge.sh");
    std::fs::write(
        &script,
        r#"#!/bin/sh
read line
id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
if printf '%s\n' "$line" | grep -q '"kind":"describe"'; then
  printf '{"type":"response","id":"%s","response":{"kind":"descriptors","descriptors":{"tools":[{"plugin_id":"ignored","name":"described_echo","description":"described echo","input_schema":{"type":"object"},"executor":{"kind":"process_bridge","handler":""},"approval_policy":"read_only","visibility":"model_visible"}]}}}\n' "$id"
else
  printf '{"type":"response","id":"%s","response":{"kind":"tool_result","output":"bridge ok","is_error":false}}\n' "$id"
fi
"#,
    )
    .expect("write describe bridge script");
    make_executable(&script);
    script
}

fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path)
            .expect("script metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).expect("chmod bridge script");
    }
}
