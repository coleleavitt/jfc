//! Platform utilities shared across the voice module.

/// Returns `true` if `bin` exists as a file on `$PATH`.
pub fn which(bin: &str) -> bool {
    let Some(path_val) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path_val)
        .map(|dir| dir.join(bin))
        .any(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn which_finds_sh_normal() {
        // /bin/sh should exist on any POSIX system
        assert!(which("sh"), "sh should be on PATH");
    }

    #[test]
    fn which_missing_binary_robust() {
        assert!(!which("this-binary-does-not-exist-jfc-voice-test"));
    }
}
