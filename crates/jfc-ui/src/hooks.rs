//! Lifecycle hook system with enum dispatch.
//!
//! Hooks fire at 8 defined points in the tool dispatch pipeline.
//! All dispatch is via enum match — no trait objects, no dynamic dispatch.

/// Points in the lifecycle where hooks can fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookPoint {
    BeforeToolDispatch,
    AfterToolDispatch,
    BeforeStream,
    AfterStream,
    OnError,
    OnToolApproval,
    BeforeCommit,
    OnSessionStart,
}

/// Action a hook can take.
#[derive(Debug, Clone)]
pub enum HookAction {
    /// Continue to next hook / proceed with operation.
    Continue,
    /// Skip the operation (tool not executed, no error).
    Skip,
    /// Replace the tool input with a different one.
    Replace(String),
    /// Abort with an error message.
    Abort(String),
}

/// Context passed to hooks.
#[derive(Debug, Clone)]
pub struct HookContext {
    pub tool_name: String,
    pub tool_input: String,
    pub session_id: String,
    pub intent: Option<String>,
}

/// Concrete hook handlers — enum dispatch, no dyn.
#[derive(Debug, Clone)]
pub enum HookHandler {
    /// Logs the hook invocation (for debugging).
    Logger,
    /// Permission check (delegates to permission system).
    PermissionCheck,
    /// Intent enrichment (adds intent to context).
    IntentEnricher,
    /// Comment/slop checker.
    CommentChecker,
    /// Custom function (for testing and extensibility).
    Custom { name: String, action: HookAction },
}

impl HookHandler {
    pub fn execute(&self, ctx: &HookContext) -> HookAction {
        match self {
            Self::Logger => {
                tracing::debug!(
                    tool = %ctx.tool_name,
                    "hook fired"
                );
                HookAction::Continue
            }
            Self::PermissionCheck => {
                // Placeholder — actual integration in Task 13
                HookAction::Continue
            }
            Self::IntentEnricher => {
                #[cfg(feature = "intent-gate")]
                {
                    // Classification would happen here when both features are enabled.
                    // For now, just log that intent enrichment was requested.
                    tracing::debug!("intent enricher hook fired");
                }
                HookAction::Continue
            }
            Self::CommentChecker => {
                // Placeholder — actual integration in Task 26
                HookAction::Continue
            }
            Self::Custom { action, .. } => action.clone(),
        }
    }
}

/// Registry of hooks, fired in registration order (FIFO).
pub struct HookRegistry {
    hooks: Vec<(HookPoint, HookHandler)>,
}

impl HookRegistry {
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    pub fn register(&mut self, point: HookPoint, handler: HookHandler) {
        self.hooks.push((point, handler));
    }

    /// Fire all hooks registered for the given point.
    /// Short-circuits on first Skip or Abort.
    pub fn fire(&self, point: HookPoint, ctx: &HookContext) -> HookAction {
        for (hook_point, handler) in &self.hooks {
            if *hook_point == point {
                let action = handler.execute(ctx);
                match &action {
                    HookAction::Continue => continue,
                    HookAction::Skip | HookAction::Abort(_) | HookAction::Replace(_) => {
                        return action;
                    }
                }
            }
        }
        HookAction::Continue
    }

    /// Number of registered hooks.
    pub fn len(&self) -> usize {
        self.hooks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> HookContext {
        HookContext {
            tool_name: "bash".to_string(),
            tool_input: "cargo test".to_string(),
            session_id: "session-1".to_string(),
            intent: None,
        }
    }

    fn assert_continue(action: HookAction) {
        assert!(matches!(action, HookAction::Continue));
    }

    #[test]
    fn test_hook_points_exhaustive() {
        let points = [
            HookPoint::BeforeToolDispatch,
            HookPoint::AfterToolDispatch,
            HookPoint::BeforeStream,
            HookPoint::AfterStream,
            HookPoint::OnError,
            HookPoint::OnToolApproval,
            HookPoint::BeforeCommit,
            HookPoint::OnSessionStart,
        ];

        for point in points {
            match point {
                HookPoint::BeforeToolDispatch
                | HookPoint::AfterToolDispatch
                | HookPoint::BeforeStream
                | HookPoint::AfterStream
                | HookPoint::OnError
                | HookPoint::OnToolApproval
                | HookPoint::BeforeCommit
                | HookPoint::OnSessionStart => {}
            }
        }
    }

    #[test]
    fn test_fire_continues_through_loggers() {
        let mut registry = HookRegistry::new();
        registry.register(HookPoint::BeforeToolDispatch, HookHandler::Logger);
        registry.register(HookPoint::BeforeToolDispatch, HookHandler::Logger);
        registry.register(HookPoint::BeforeToolDispatch, HookHandler::Logger);

        assert_continue(registry.fire(HookPoint::BeforeToolDispatch, &context()));
    }

    #[test]
    fn test_fire_short_circuits_on_abort() {
        let mut registry = HookRegistry::new();
        registry.register(HookPoint::BeforeToolDispatch, HookHandler::Logger);
        registry.register(
            HookPoint::BeforeToolDispatch,
            HookHandler::Custom {
                name: "abort".to_string(),
                action: HookAction::Abort("blocked".to_string()),
            },
        );
        registry.register(HookPoint::BeforeToolDispatch, HookHandler::Logger);
        registry.register(
            HookPoint::BeforeToolDispatch,
            HookHandler::Custom {
                name: "replace".to_string(),
                action: HookAction::Replace("should-not-run".to_string()),
            },
        );

        match registry.fire(HookPoint::BeforeToolDispatch, &context()) {
            HookAction::Abort(message) => assert_eq!(message, "blocked"),
            action => panic!("expected abort, got {action:?}"),
        }
    }

    #[test]
    fn test_fire_short_circuits_on_skip() {
        let mut registry = HookRegistry::new();
        registry.register(HookPoint::BeforeToolDispatch, HookHandler::Logger);
        registry.register(
            HookPoint::BeforeToolDispatch,
            HookHandler::Custom {
                name: "skip".to_string(),
                action: HookAction::Skip,
            },
        );
        registry.register(HookPoint::BeforeToolDispatch, HookHandler::Logger);
        registry.register(
            HookPoint::BeforeToolDispatch,
            HookHandler::Custom {
                name: "abort".to_string(),
                action: HookAction::Abort("should-not-run".to_string()),
            },
        );

        assert!(matches!(
            registry.fire(HookPoint::BeforeToolDispatch, &context()),
            HookAction::Skip
        ));
    }

    #[test]
    fn test_fire_only_matching_point() {
        let mut registry = HookRegistry::new();
        registry.register(
            HookPoint::AfterToolDispatch,
            HookHandler::Custom {
                name: "abort".to_string(),
                action: HookAction::Abort("wrong-point".to_string()),
            },
        );
        registry.register(HookPoint::BeforeToolDispatch, HookHandler::Logger);

        assert_continue(registry.fire(HookPoint::BeforeToolDispatch, &context()));
    }

    #[test]
    fn test_registry_fifo_order() {
        let mut registry = HookRegistry::new();
        registry.register(
            HookPoint::BeforeToolDispatch,
            HookHandler::Custom {
                name: "first".to_string(),
                action: HookAction::Replace("first".to_string()),
            },
        );
        registry.register(
            HookPoint::BeforeToolDispatch,
            HookHandler::Custom {
                name: "second".to_string(),
                action: HookAction::Replace("second".to_string()),
            },
        );

        match registry.fire(HookPoint::BeforeToolDispatch, &context()) {
            HookAction::Replace(input) => assert_eq!(input, "first"),
            action => panic!("expected replace, got {action:?}"),
        }
    }

    #[test]
    fn test_hook_context_with_intent() {
        let ctx = HookContext {
            intent: Some("Implementation".to_string()),
            ..context()
        };
        let mut registry = HookRegistry::new();
        registry.register(HookPoint::BeforeToolDispatch, HookHandler::IntentEnricher);

        assert_eq!(ctx.intent.as_deref(), Some("Implementation"));
        assert_continue(registry.fire(HookPoint::BeforeToolDispatch, &ctx));
    }

    #[test]
    fn test_hook_integration_point_compiles() {
        #[cfg(feature = "hooks")]
        {
            tracing::trace!(target: "jfc::hooks", "hook integration point: BeforeToolDispatch");
        }
    }
}
