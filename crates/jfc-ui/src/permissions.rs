//! TOML-driven permission automation engine.
//!
//! Rules are evaluated by specificity, with first-match-wins as the
//! tie-breaker. A ceiling list provides hard denials that cannot be overridden.

use crate::config::feature_config::{FeatureConfig, PermissionRuleConfig};

/// Action to take for a permission decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionAction {
    Allow,
    Deny,
    /// No rule matched — fall through to interactive prompt.
    Ask,
}

/// Result of evaluating permission rules.
#[derive(Debug, Clone)]
pub struct PermissionDecision {
    pub action: PermissionAction,
    pub reason: Option<String>,
    pub rule_index: Option<usize>,
}

/// A compiled permission rule with glob patterns.
#[derive(Debug, Clone)]
pub struct PermissionRule {
    pub action: PermissionAction,
    pub tool_pattern: GlobPattern,
    pub path_pattern: Option<GlobPattern>,
    pub reason: Option<String>,
}

/// Simple glob pattern matcher (supports * and **).
#[derive(Debug, Clone)]
pub struct GlobPattern {
    pattern: String,
}

impl GlobPattern {
    pub fn new(pattern: &str) -> Self {
        Self {
            pattern: pattern.to_owned(),
        }
    }

    pub fn matches(&self, input: &str) -> bool {
        glob_matches(self.pattern.as_bytes(), input.as_bytes())
    }

    fn matches_anything(&self) -> bool {
        self.pattern == "*"
    }

    fn matches_path(&self, path: &str) -> bool {
        glob_matches_with_separator(
            self.pattern.as_bytes(),
            path.as_bytes(),
            !self.pattern.contains('/'),
        )
    }

    fn specificity(&self) -> usize {
        self.pattern
            .bytes()
            .filter(|byte| !matches!(byte, b'*' | b'?'))
            .count()
    }
}

#[derive(Debug, Clone)]
pub struct RuleSet {
    ceiling: Vec<GlobPattern>,
    rules: Vec<PermissionRule>,
}

impl RuleSet {
    pub fn from_config(config: &FeatureConfig) -> Self {
        Self {
            ceiling: config
                .permissions
                .ceiling
                .iter()
                .map(|pattern| GlobPattern::new(pattern))
                .collect(),
            rules: config
                .permissions
                .rules
                .iter()
                .map(PermissionRule::from_config)
                .collect(),
        }
    }

    pub fn evaluate(&self, tool_name: &str, path: Option<&str>) -> PermissionDecision {
        let target = RuleTarget::new(tool_name, path);

        if self.ceiling.iter().any(|pattern| {
            ceiling_matches(pattern, tool_name, path) || pattern.matches(target.combined())
        }) {
            return PermissionDecision {
                action: PermissionAction::Deny,
                reason: Some("blocked by escalation ceiling".to_owned()),
                rule_index: None,
            };
        }

        let mut best_match: Option<(usize, &PermissionRule)> = None;

        for (index, rule) in self.rules.iter().enumerate() {
            if !rule.matches(tool_name, path) {
                continue;
            }

            let Some((_, current)) = best_match else {
                best_match = Some((index, rule));
                continue;
            };

            if rule.is_stronger_than(current) {
                best_match = Some((index, rule));
            }
        }

        if let Some((index, rule)) = best_match {
            return PermissionDecision {
                action: rule.action,
                reason: rule.reason.clone(),
                rule_index: Some(index),
            };
        }

        PermissionDecision {
            action: PermissionAction::Ask,
            reason: None,
            rule_index: None,
        }
    }
}

/// Check permission for a tool invocation.
/// Returns the decision without executing anything.
pub fn check_tool_permission(
    rules: &RuleSet,
    tool_name: &str,
    path: Option<&str>,
) -> PermissionDecision {
    rules.evaluate(tool_name, path)
}

impl PermissionRule {
    fn from_config(config: &PermissionRuleConfig) -> Self {
        let (tool_pattern, path_pattern) =
            split_tool_path_pattern(&config.tool, config.path.as_deref());

        Self {
            action: PermissionAction::from_config_value(&config.action),
            tool_pattern: GlobPattern::new(tool_pattern),
            path_pattern: path_pattern.map(GlobPattern::new),
            reason: config.reason.clone(),
        }
    }

    fn matches(&self, tool_name: &str, path: Option<&str>) -> bool {
        if !self.tool_pattern.matches(tool_name) {
            return false;
        }

        match (&self.path_pattern, path) {
            (Some(pattern), Some(path)) => pattern.matches_anything() || pattern.matches_path(path),
            (Some(_), None) => false,
            (None, _) => true,
        }
    }

    fn is_stronger_than(&self, other: &Self) -> bool {
        match self.specificity().cmp(&other.specificity()) {
            std::cmp::Ordering::Greater => true,
            // Deny > Allow tiebreaker for defense-in-depth — when two glob
            // patterns have equal specificity, prefer the safer (deny)
            // verdict.
            std::cmp::Ordering::Equal => {
                self.action == PermissionAction::Deny && other.action != PermissionAction::Deny
            }
            std::cmp::Ordering::Less => false,
        }
    }

    fn specificity(&self) -> usize {
        self.tool_pattern.specificity()
            + self
                .path_pattern
                .as_ref()
                .map_or(0, GlobPattern::specificity)
    }
}

impl PermissionAction {
    fn from_config_value(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "allow" => Self::Allow,
            "deny" => Self::Deny,
            _ => Self::Ask,
        }
    }
}

struct RuleTarget {
    combined: String,
}

impl RuleTarget {
    fn new(tool_name: &str, path: Option<&str>) -> Self {
        let combined = path.map_or_else(
            || tool_name.to_owned(),
            |path| format!("{tool_name}:{path}"),
        );
        Self { combined }
    }

    fn combined(&self) -> &str {
        &self.combined
    }
}

fn split_tool_path_pattern<'a>(
    tool_pattern: &'a str,
    path_pattern: Option<&'a str>,
) -> (&'a str, Option<&'a str>) {
    if path_pattern.is_some() {
        return (tool_pattern, path_pattern);
    }

    tool_pattern
        .split_once(':')
        .map_or((tool_pattern, None), |(tool, path)| (tool, Some(path)))
}

fn ceiling_matches(pattern: &GlobPattern, tool_name: &str, path: Option<&str>) -> bool {
    let Some((tool_pattern, path_pattern)) = pattern.pattern.split_once(':') else {
        return false;
    };

    if !GlobPattern::new(tool_pattern).matches(tool_name) {
        return false;
    }

    path.is_some_and(|path| {
        let path_pattern = GlobPattern::new(path_pattern);
        path_pattern.matches_anything() || path_pattern.matches_path(path)
    })
}

fn glob_matches(pattern: &[u8], input: &[u8]) -> bool {
    glob_matches_with_separator(pattern, input, false)
}

fn glob_matches_with_separator(pattern: &[u8], input: &[u8], star_matches_separator: bool) -> bool {
    if pattern.is_empty() {
        return input.is_empty();
    }

    if pattern.starts_with(b"**") {
        let rest = &pattern[2..];
        return glob_matches_with_separator(rest, input, star_matches_separator)
            || (!input.is_empty()
                && glob_matches_with_separator(pattern, &input[1..], star_matches_separator));
    }

    if pattern[0] == b'*' {
        let rest = &pattern[1..];
        return glob_matches_with_separator(rest, input, star_matches_separator)
            || (!input.is_empty()
                && (star_matches_separator || input[0] != b'/')
                && glob_matches_with_separator(pattern, &input[1..], star_matches_separator));
    }

    !input.is_empty()
        && pattern[0] == input[0]
        && glob_matches_with_separator(&pattern[1..], &input[1..], star_matches_separator)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::feature_config::FeatureConfig;

    #[test]
    fn test_glob_pattern_star() {
        let pattern = GlobPattern::new("Edit:src/*");

        assert!(pattern.matches("Edit:src/lib.rs"));
        assert!(!pattern.matches("Edit:src/sub/file.rs"));
    }

    #[test]
    fn test_glob_pattern_doublestar() {
        let pattern = GlobPattern::new("Edit:src/**");

        assert!(pattern.matches("Edit:src/sub/deep/file.rs"));
    }

    #[test]
    fn test_rule_parse_from_config() {
        let mut config = FeatureConfig::default();
        config.permissions.rules = vec![
            rule("allow", "Edit", Some("src/**"), Some("source edits")),
            rule("deny", "Bash:*", None, Some("shell denied")),
            rule("allow", "Read", Some("README.md"), None),
        ];

        let rules = RuleSet::from_config(&config);

        assert_eq!(rules.rules.len(), 3);
        assert_eq!(rules.rules[0].action, PermissionAction::Allow);
        assert!(rules.rules[0].tool_pattern.matches("Edit"));
        assert!(
            rules.rules[0]
                .path_pattern
                .as_ref()
                .is_some_and(|pattern| pattern.matches("src/lib.rs"))
        );
        assert_eq!(rules.rules[1].action, PermissionAction::Deny);
        assert!(rules.rules[1].tool_pattern.matches("Bash"));
        assert!(
            rules.rules[1]
                .path_pattern
                .as_ref()
                .is_some_and(|pattern| pattern.matches("anything"))
        );
        assert_eq!(rules.rules[2].action, PermissionAction::Allow);
        assert!(rules.rules[2].tool_pattern.matches("Read"));
    }

    #[test]
    fn test_evaluate_first_match_wins() {
        let mut config = FeatureConfig::default();
        config.permissions.ceiling.clear();
        config.permissions.rules = vec![
            rule("deny", "Edit", Some("src/**"), Some("first match")),
            rule("allow", "Edit", Some("src/**"), Some("would allow")),
        ];

        let rules = RuleSet::from_config(&config);
        let decision = rules.evaluate("Edit", Some("src/lib.rs"));

        assert_eq!(decision.action, PermissionAction::Deny);
        assert_eq!(decision.reason.as_deref(), Some("first match"));
        assert_eq!(decision.rule_index, Some(0));
    }

    #[test]
    fn test_ceiling_blocks_even_with_allow() {
        let mut config = FeatureConfig::default();
        config.permissions.ceiling = vec!["Bash:rm -rf*".to_owned()];
        config.permissions.rules = vec![rule("allow", "Bash:*", None, Some("allow shell"))];

        let rules = RuleSet::from_config(&config);
        let decision = rules.evaluate("Bash", Some("rm -rf /"));

        assert_eq!(decision.action, PermissionAction::Deny);
        assert_eq!(
            decision.reason.as_deref(),
            Some("blocked by escalation ceiling")
        );
        assert_eq!(decision.rule_index, None);
    }

    #[test]
    fn test_ceiling_blocks_even_with_allow_all() {
        let mut config = FeatureConfig::default();
        config.permissions.ceiling = vec!["Bash:rm -rf*".to_owned()];
        config.permissions.rules = vec![rule("allow", "*", Some("*"), Some("allow all"))];

        let rules = RuleSet::from_config(&config);
        let decision = rules.evaluate("Bash", Some("rm -rf /"));

        assert_eq!(decision.action, PermissionAction::Deny);
        assert_eq!(
            decision.reason.as_deref(),
            Some("blocked by escalation ceiling")
        );
        assert_eq!(decision.rule_index, None);
    }

    #[test]
    fn test_ceiling_active_in_auto_approve_mode() {
        let mut config = FeatureConfig::default();
        config.permissions.ceiling = vec!["Bash:rm -rf *".to_owned()];
        config.permissions.rules = vec![
            rule("allow", "Bash", Some("*"), Some("auto approve shell")),
            rule("allow", "*", Some("*"), Some("auto approve all")),
        ];

        let rules = RuleSet::from_config(&config);
        let decision = rules.evaluate("Bash", Some("rm -rf /"));

        assert_eq!(decision.action, PermissionAction::Deny);
        assert_eq!(
            decision.reason.as_deref(),
            Some("blocked by escalation ceiling")
        );
        assert_eq!(decision.rule_index, None);
    }

    #[test]
    fn test_default_ceiling_blocks_destructive() {
        let mut config = FeatureConfig::default();
        config.permissions.rules = vec![rule("allow", "*", Some("*"), Some("allow all"))];

        assert_eq!(
            config.permissions.ceiling,
            vec!["Bash:rm -rf *", "Bash:dd *", "Bash:mkfs *"]
        );

        let rules = RuleSet::from_config(&config);

        for command in ["rm -rf /", "dd if=/dev/zero of=/dev/sda", "mkfs /dev/sda"] {
            let decision = rules.evaluate("Bash", Some(command));

            assert_eq!(decision.action, PermissionAction::Deny, "{command}");
            assert_eq!(
                decision.reason.as_deref(),
                Some("blocked by escalation ceiling"),
                "{command}"
            );
            assert_eq!(decision.rule_index, None, "{command}");
        }
    }

    #[test]
    fn test_no_match_returns_ask() {
        let mut config = FeatureConfig::default();
        config.permissions.ceiling.clear();
        config.permissions.rules = vec![rule("allow", "Edit", Some("src/**"), None)];

        let rules = RuleSet::from_config(&config);
        let decision = rules.evaluate("Read", Some("README.md"));

        assert_eq!(decision.action, PermissionAction::Ask);
        assert_eq!(decision.reason, None);
        assert_eq!(decision.rule_index, None);
    }

    #[test]
    fn test_deny_overrides_allow_same_path() {
        let mut config = FeatureConfig::default();
        config.permissions.ceiling.clear();
        config.permissions.rules = vec![
            rule("allow", "Edit", Some("src/**"), Some("allow src")),
            rule("deny", "Edit", Some("src/secrets/**"), Some("deny secrets")),
        ];

        let rules = RuleSet::from_config(&config);
        let decision = rules.evaluate("Edit", Some("src/secrets/key.txt"));

        assert_eq!(decision.action, PermissionAction::Deny);
        assert_eq!(decision.reason.as_deref(), Some("deny secrets"));
        assert_eq!(decision.rule_index, Some(1));
    }

    #[test]
    fn test_evaluate_with_path() {
        let mut config = FeatureConfig::default();
        config.permissions.ceiling.clear();
        config.permissions.rules = vec![rule("allow", "Edit:src/**", None, Some("source edits"))];

        let rules = RuleSet::from_config(&config);
        let decision = rules.evaluate("Edit", Some("src/lib.rs"));

        assert_eq!(decision.action, PermissionAction::Allow);
        assert_eq!(decision.reason.as_deref(), Some("source edits"));
        assert_eq!(decision.rule_index, Some(0));
    }

    #[test]
    fn test_check_tool_permission_convenience() {
        let mut config = FeatureConfig::default();
        config.permissions.ceiling.clear();
        config.permissions.rules =
            vec![rule("allow", "Read", Some("src/**"), Some("source reads"))];

        let rules = RuleSet::from_config(&config);
        let direct = rules.evaluate("Read", Some("src/main.rs"));
        let convenience = check_tool_permission(&rules, "Read", Some("src/main.rs"));

        assert_eq!(convenience.action, direct.action);
        assert_eq!(convenience.reason, direct.reason);
        assert_eq!(convenience.rule_index, direct.rule_index);
    }

    fn rule(
        action: &str,
        tool: &str,
        path: Option<&str>,
        reason: Option<&str>,
    ) -> PermissionRuleConfig {
        PermissionRuleConfig {
            action: action.to_owned(),
            tool: tool.to_owned(),
            path: path.map(str::to_owned),
            reason: reason.map(str::to_owned),
        }
    }

    // Robust: when two rules have *exactly* equal specificity (same tool
    // pattern, same path pattern, both fully literal so neither has any
    // glob bytes), the Deny>Allow tiebreaker in `is_stronger_than` must
    // pick the safer (deny) verdict regardless of registration order.
    // This locks in the defense-in-depth behavior described in the
    // comment on `PermissionRule::is_stronger_than`.
    #[test]
    fn permissions_tiebreaker_prefers_deny_robust() {
        // Allow registered first, deny registered second.
        let mut config = FeatureConfig::default();
        config.permissions.ceiling.clear();
        config.permissions.rules = vec![
            rule("allow", "Edit", Some("src/lib.rs"), Some("permissive")),
            rule("deny", "Edit", Some("src/lib.rs"), Some("strict")),
        ];

        let rules = RuleSet::from_config(&config);
        let decision = rules.evaluate("Edit", Some("src/lib.rs"));

        assert_eq!(
            decision.action,
            PermissionAction::Deny,
            "deny must win on equal specificity",
        );
        assert_eq!(decision.reason.as_deref(), Some("strict"));

        // Reverse order — deny first, allow second. Same outcome.
        let mut config = FeatureConfig::default();
        config.permissions.ceiling.clear();
        config.permissions.rules = vec![
            rule("deny", "Edit", Some("src/lib.rs"), Some("strict")),
            rule("allow", "Edit", Some("src/lib.rs"), Some("permissive")),
        ];

        let rules = RuleSet::from_config(&config);
        let decision = rules.evaluate("Edit", Some("src/lib.rs"));

        assert_eq!(
            decision.action,
            PermissionAction::Deny,
            "deny must win regardless of registration order",
        );
        assert_eq!(decision.reason.as_deref(), Some("strict"));
    }
}
