//! Intent-policy environment flags.
//!
//! Two runtime toggles the dispatcher consults when deciding whether to
//! surface a doc-suggestion toast or flip into plan-mode posture. Both are
//! read fresh on each call so a session can change them without a restart;
//! no LLM round-trip and no I/O beyond a single env read.

/// Whether the auto-plan-mode flip is enabled. Off by default — the
/// false-positive cost (suddenly read-only when the user wanted edits)
/// is high enough that we make this opt-in. Users set
/// `JFC_AUTO_PLAN_MODE=1` to turn it on.
pub fn auto_plan_mode_enabled() -> bool {
    matches!(
        std::env::var("JFC_AUTO_PLAN_MODE")
            .ok()
            .as_deref()
            .map(|s| s.trim().to_lowercase()),
        Some(ref v) if matches!(v.as_str(), "1" | "true" | "on" | "yes")
    )
}

/// Whether doc-suggestion toasts are enabled. On by default — non-
/// destructive (just a toast saying "press /plan"), so opting out is
/// for users who already know the slash commands.
pub fn auto_doc_suggest_enabled() -> bool {
    match std::env::var("JFC_AUTO_DOC_SUGGEST") {
        Ok(v) => {
            let v = v.trim().to_lowercase();
            !matches!(v.as_str(), "0" | "false" | "off" | "no")
        }
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Robust: env-flag helpers respect the canonical truthy/falsy
    /// values. Both helpers are read fresh each call, so a session
    /// can flip them at runtime without restart.
    #[serial_test::serial]
    #[test]
    fn auto_plan_mode_and_doc_suggest_env_flags_respect_canonical_values_robust() {
        struct Restore {
            apm: Option<String>,
            ads: Option<String>,
        }
        impl Drop for Restore {
            fn drop(&mut self) {
                unsafe {
                    match self.apm.take() {
                        Some(v) => std::env::set_var("JFC_AUTO_PLAN_MODE", v),
                        None => std::env::remove_var("JFC_AUTO_PLAN_MODE"),
                    }
                    match self.ads.take() {
                        Some(v) => std::env::set_var("JFC_AUTO_DOC_SUGGEST", v),
                        None => std::env::remove_var("JFC_AUTO_DOC_SUGGEST"),
                    }
                }
            }
        }
        let _r = Restore {
            apm: std::env::var("JFC_AUTO_PLAN_MODE").ok(),
            ads: std::env::var("JFC_AUTO_DOC_SUGGEST").ok(),
        };

        // Auto-plan-mode is opt-in: unset / 0 → off; 1/true/on → on.
        unsafe { std::env::remove_var("JFC_AUTO_PLAN_MODE") };
        assert!(!auto_plan_mode_enabled(), "default should be OFF");
        unsafe { std::env::set_var("JFC_AUTO_PLAN_MODE", "0") };
        assert!(!auto_plan_mode_enabled());
        for on in ["1", "true", "on", "yes"] {
            unsafe { std::env::set_var("JFC_AUTO_PLAN_MODE", on) };
            assert!(auto_plan_mode_enabled(), "value {on:?} should enable");
        }

        // Doc-suggest is opt-out: unset / anything-not-disabled → on.
        unsafe { std::env::remove_var("JFC_AUTO_DOC_SUGGEST") };
        assert!(auto_doc_suggest_enabled(), "default should be ON");
        for off in ["0", "false", "off", "no"] {
            unsafe { std::env::set_var("JFC_AUTO_DOC_SUGGEST", off) };
            assert!(!auto_doc_suggest_enabled(), "value {off:?} should disable");
        }
    }
}
