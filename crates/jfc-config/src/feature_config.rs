//! Feature configuration loaded from `.jfc/features.toml`.
//!
//! Missing file → defaults. Malformed TOML → warning + defaults.

use serde::Deserialize;
use std::path::Path;

use crate::trace;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct FeatureConfig {
    pub permissions: PermissionsConfig,
    pub hooks: HooksConfig,
    pub intent: IntentConfig,
    pub background: BackgroundConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PermissionsConfig {
    pub enabled: bool,
    pub allowed_tools: Vec<String>,
    pub denied_tools: Vec<String>,
    pub rules: Vec<PermissionRuleConfig>,
    pub ceiling: Vec<String>,
}

impl Default for PermissionsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allowed_tools: Vec::new(),
            denied_tools: Vec::new(),
            rules: Vec::new(),
            ceiling: vec![
                "Bash:rm -rf *".to_owned(),
                "Bash:dd *".to_owned(),
                "Bash:mkfs *".to_owned(),
            ],
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PermissionRuleConfig {
    pub action: String,
    pub tool: String,
    pub path: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct HooksConfig {
    pub enabled: bool,
    pub comment_check: CommentCheckConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CommentCheckConfig {
    pub enabled: bool,
    pub patterns: Vec<String>,
}

impl Default for CommentCheckConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            patterns: vec![
                "// This function".to_owned(),
                "// TODO: implement".to_owned(),
            ],
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct IntentConfig {
    pub enabled: bool,
    pub confidence_threshold: f32,
}

impl Default for IntentConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            confidence_threshold: 0.6,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct BackgroundConfig {
    pub max_concurrent: usize,
    pub max_depth: usize,
}

impl Default for BackgroundConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 5,
            max_depth: 2,
        }
    }
}

impl FeatureConfig {
    /// Load from `.jfc/features.toml` relative to `base_dir`.
    /// Returns defaults if file missing or malformed.
    pub fn load(base_dir: &Path) -> Self {
        let _linkscope_load = linkscope::phase("config.features.load");
        let path = base_dir.join(".jfc").join("features.toml");
        trace::record_path_shape("config.features.load.start", &path);
        match std::fs::read_to_string(&path) {
            Ok(content) => match toml::from_str(&content) {
                Ok(config) => {
                    trace::record_config_load(trace::ConfigLoadTrace {
                        label: "config.features.toml.load",
                        depth: 0,
                        bytes: content.len(),
                    });
                    trace::record_feature_shape("config.features.shape", &config);
                    trace::record_status("config.features.load_result", "parsed");
                    config
                }
                Err(e) => {
                    trace::record_config_load(trace::ConfigLoadTrace {
                        label: "config.features.toml.load",
                        depth: 0,
                        bytes: content.len(),
                    });
                    trace::record_status("config.features.load_result", "parse_error");
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "malformed features.toml, using defaults"
                    );
                    Self::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                trace::record_status("config.features.load_result", "missing");
                Self::default()
            }
            Err(_) => {
                trace::record_status("config.features.load_result", "read_error");
                Self::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feature_config_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let config = FeatureConfig::load(tmp.path());
        assert!(!config.permissions.enabled);
        assert_eq!(config.background.max_concurrent, 5);
    }

    #[test]
    fn test_feature_config_valid_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let jfc_dir = tmp.path().join(".jfc");
        std::fs::create_dir_all(&jfc_dir).unwrap();
        std::fs::write(
            jfc_dir.join("features.toml"),
            r#"
            [permissions]
            enabled = true

            [background]
            max_concurrent = 10
            "#,
        )
        .unwrap();
        let config = FeatureConfig::load(tmp.path());
        assert!(config.permissions.enabled);
        assert_eq!(config.background.max_concurrent, 10);
    }

    #[test]
    fn test_feature_config_malformed_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let jfc_dir = tmp.path().join(".jfc");
        std::fs::create_dir_all(&jfc_dir).unwrap();
        std::fs::write(jfc_dir.join("features.toml"), "{{invalid toml").unwrap();
        let config = FeatureConfig::load(tmp.path());
        // Should return defaults without panicking
        assert!(!config.permissions.enabled);
    }

    #[test]
    fn feature_config_trace_records_shape_without_rule_payload_normal() {
        linkscope::trace_detail_enable();
        let tmp = tempfile::tempdir().unwrap();
        let jfc_dir = tmp.path().join(".jfc");
        std::fs::create_dir_all(&jfc_dir).unwrap();
        std::fs::write(
            jfc_dir.join("features.toml"),
            r#"
            [permissions]
            enabled = true
            allowed_tools = ["private-allowed-tool"]
            denied_tools = ["private-denied-tool"]

            [[permissions.rules]]
            action = "allow"
            tool = "private-rule-tool"

            [hooks]
            enabled = true

            [hooks.comment_check]
            patterns = ["private-comment-pattern"]

            [intent]
            enabled = true
            "#,
        )
        .unwrap();

        let config = FeatureConfig::load(tmp.path());

        assert!(config.permissions.enabled);
        let snapshot = linkscope::snapshot();
        let rendered = format!("{snapshot:?}");
        assert!(rendered.contains("config.features.load_result"));
        assert!(rendered.contains("config.features.shape"));
        assert!(rendered.contains("allowed_tools"));
        assert!(rendered.contains("permission_rules"));
        assert!(!rendered.contains("private-allowed-tool"));
        assert!(!rendered.contains("private-denied-tool"));
        assert!(!rendered.contains("private-rule-tool"));
        assert!(!rendered.contains("private-comment-pattern"));
    }
}
