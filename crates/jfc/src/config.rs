//! Re-export from `jfc-config`. The config schema and loading logic now live
//! in the standalone `jfc-config` crate; this module re-exports everything
//! and adds the thin wrappers that depend on jfc-ui–specific types.

pub use jfc_config::*;

/// Persist the permission mode to config.toml so it survives sessions.
pub fn save_permission_mode(mode: &crate::app::PermissionMode) {
    let mode_str = match mode {
        crate::app::PermissionMode::Default => "default",
        crate::app::PermissionMode::AcceptEdits => "accept-edits",
        crate::app::PermissionMode::Plan => "plan",
        crate::app::PermissionMode::BypassPermissions => "bypass",
        crate::app::PermissionMode::Auto => "auto",
    };
    jfc_config::save_permission_mode_str(mode_str);
}

/// Convert the TOML-form rules from a `Config` into the runtime
/// `slate::RoutingRule` values consumed by `SlateRouter`.
pub fn slate_rules_from_config(cfg: &Config) -> Vec<crate::slate::RoutingRule> {
    let Some(ref rules) = cfg.slate_rules else {
        return Vec::new();
    };
    rules
        .iter()
        .filter_map(|r| {
            match r.query_class.as_str() {
                "trivial" => Some(crate::slate::QueryClass::Trivial),
                "exploration" => Some(crate::slate::QueryClass::Exploration),
                "code-edit" => Some(crate::slate::QueryClass::CodeEdit),
                "refactor" => Some(crate::slate::QueryClass::Refactor),
                "research" => Some(crate::slate::QueryClass::Research),
                "long-context" => Some(crate::slate::QueryClass::LongContext),
                other => {
                    tracing::warn!(
                        target: "jfc::slate",
                        query_class = other,
                        "unknown slate query_class — rule dropped"
                    );
                    None
                }
            }
            .map(|class| {
                let mut rule = crate::slate::RoutingRule::new(class, r.model.clone());
                if let Some(ref fb) = r.fallback_model {
                    rule = rule.with_fallback(fb.clone());
                }
                rule
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_slate_rules_normal() {
        let cfg: Config = toml::from_str(
            r#"
slate_enabled = true

[[slate_rules]]
query_class = "trivial"
model = "claude-haiku-4-5"

[[slate_rules]]
query_class = "refactor"
model = "claude-opus-4-7"
fallback_model = "claude-sonnet-4-6"
"#,
        )
        .unwrap();
        assert!(cfg.slate_enabled);
        let rules = cfg.slate_rules.as_ref().expect("rules present");
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].query_class, "trivial");
        assert_eq!(rules[0].model, "claude-haiku-4-5");

        let rt = slate_rules_from_config(&cfg);
        assert_eq!(rt.len(), 2);
        assert_eq!(rt[0].query_class, crate::slate::QueryClass::Trivial);
        assert_eq!(rt[1].query_class, crate::slate::QueryClass::Refactor);
        assert_eq!(rt[1].fallback_model.as_deref(), Some("claude-sonnet-4-6"));
    }

    #[test]
    fn parse_slate_unknown_class_dropped_robust() {
        let cfg: Config = toml::from_str(
            r#"
slate_enabled = true

[[slate_rules]]
query_class = "trivial"
model = "haiku"

[[slate_rules]]
query_class = "not-a-real-class"
model = "garbage"
"#,
        )
        .unwrap();
        assert_eq!(cfg.slate_rules.as_ref().unwrap().len(), 2);
        let rt = slate_rules_from_config(&cfg);
        assert_eq!(rt.len(), 1);
        assert_eq!(rt[0].query_class, crate::slate::QueryClass::Trivial);
    }

    #[test]
    fn slate_disabled_by_default_robust() {
        let cfg = Config::default();
        assert!(!cfg.slate_enabled);
        assert!(cfg.slate_rules.is_none());
        assert!(slate_rules_from_config(&cfg).is_empty());
    }

    #[test]
    fn app_startup_applies_persisted_theme_normal() {
        use crate::theme::Theme;

        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("config.toml");
        std::fs::write(
            &path,
            r#"theme = "dracula"

[default]
model = "anthropic/claude-opus-4-7"
"#,
        )
        .unwrap();

        let cfg: Config = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let mut current_theme = Theme::dark();
        let initial_bg = current_theme.bg;

        if let Some(name) = cfg.theme.as_deref()
            && let Some(theme) = Theme::by_name(name)
        {
            current_theme = theme;
        }

        assert_ne!(
            current_theme.bg, initial_bg,
            "Theme was not applied — startup would leave the user on dark()"
        );
    }
}
