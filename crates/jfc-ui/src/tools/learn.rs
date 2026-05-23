//! /learn slash command — status, historize, dream, key-files, user-profile.

use super::ExecutionResult;

/// `/learn status` — report learning subsystem state.
pub(super) fn execute_learn_status() -> ExecutionResult {
    ExecutionResult::success("Learning subsystem: enabled")
}

/// `/learn historize` — trigger historian extraction (stub).
pub(super) fn execute_learn_historize() -> ExecutionResult {
    ExecutionResult::success("Historian extraction: not yet wired (stub)")
}

/// `/learn dream` — trigger dreamer maintenance cycle (stub).
pub(super) fn execute_learn_dream() -> ExecutionResult {
    ExecutionResult::success("Dreamer cycle: not yet wired (stub)")
}

/// `/learn key-files list` — list pinned key files.
pub(super) fn execute_learn_key_files_list(project_root: &std::path::Path) -> ExecutionResult {
    match jfc_learn::KeyFileStore::open(project_root) {
        Ok(store) => match store.list_pinned() {
            Ok(pinned) => {
                if pinned.is_empty() {
                    ExecutionResult::success("No pinned key files.")
                } else {
                    let mut out = String::from("Pinned key files:\n");
                    for pf in &pinned {
                        out.push_str(&format!("  {} — {}\n", pf.file_path, pf.reason));
                    }
                    ExecutionResult::success(out)
                }
            }
            Err(e) => ExecutionResult::failure(format!("Failed to list pinned files: {e}")),
        },
        Err(e) => ExecutionResult::failure(format!("Failed to open key-file store: {e}")),
    }
}

/// `/learn user-profile show` — show promoted user profile (stub).
pub(super) fn execute_learn_user_profile_show() -> ExecutionResult {
    ExecutionResult::success("User profile: not yet populated (stub)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learn_status_returns_ok_normal() {
        let result = execute_learn_status();
        assert!(!result.is_error());
        assert!(result.output.contains("Learning subsystem"));
    }
}
