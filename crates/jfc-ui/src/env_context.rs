//! Environment fingerprint — toolchain versions auto-injected into the
//! system prompt so the model sees the cargo/rustc/node/go/python
//! revision live in this workspace without burning a turn on
//! `rustc --version`.
//!
//! Each command runs once per process and the result is cached for the
//! session. `--version` invocations are <50ms each on warm caches; we
//! still amortize them because the result rarely changes mid-session.

use std::process::Command;
use std::sync::OnceLock;

#[derive(Debug, Clone, Default)]
pub struct EnvContext {
    pub rustc: Option<String>,
    pub cargo: Option<String>,
    pub node: Option<String>,
    pub deno: Option<String>,
    pub bun: Option<String>,
    pub python: Option<String>,
    pub go: Option<String>,
    pub uname: Option<String>,
    pub shell: Option<String>,
}

impl EnvContext {
    pub fn to_prompt_string(&self) -> Option<String> {
        let mut lines: Vec<String> = Vec::new();
        let mut push = |label: &str, val: &Option<String>| {
            if let Some(v) = val.as_deref() {
                if !v.is_empty() {
                    lines.push(format!("- {label}: {v}"));
                }
            }
        };
        push("rustc", &self.rustc);
        push("cargo", &self.cargo);
        push("node", &self.node);
        push("deno", &self.deno);
        push("bun", &self.bun);
        push("python", &self.python);
        push("go", &self.go);
        push("uname", &self.uname);
        push("shell", &self.shell);
        if lines.is_empty() {
            return None;
        }
        let mut out = String::from("\n\n## Environment\n\n");
        for line in lines {
            out.push_str(&line);
            out.push('\n');
        }
        Some(out)
    }
}

fn first_line(args: &[&str]) -> Option<String> {
    let out = Command::new(args[0]).args(&args[1..]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_owned)
}

/// Build a fresh `EnvContext` by probing each tool. Cached at module level
/// so the second + nth call is free.
fn build() -> EnvContext {
    EnvContext {
        rustc: first_line(&["rustc", "--version"]),
        cargo: first_line(&["cargo", "--version"]),
        node: first_line(&["node", "--version"]),
        deno: first_line(&["deno", "--version"]),
        bun: first_line(&["bun", "--version"]),
        python: first_line(&["python3", "--version"]),
        go: first_line(&["go", "version"]),
        uname: first_line(&["uname", "-srm"]),
        shell: std::env::var("SHELL")
            .ok()
            .and_then(|s| s.rsplit('/').next().map(str::to_owned)),
    }
}

/// Get the cached env context. First call probes (slow-ish, ~200ms total
/// across all tools), subsequent calls are zero-cost.
pub fn get() -> &'static EnvContext {
    static CACHE: OnceLock<EnvContext> = OnceLock::new();
    CACHE.get_or_init(build)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_context_renders_none_normal() {
        let ctx = EnvContext::default();
        assert!(ctx.to_prompt_string().is_none());
    }

    #[test]
    fn populated_context_renders_lines_normal() {
        let ctx = EnvContext {
            rustc: Some("rustc 1.80.0".to_owned()),
            cargo: Some("cargo 1.80.0".to_owned()),
            ..Default::default()
        };
        let s = ctx.to_prompt_string().unwrap();
        assert!(s.contains("## Environment"));
        assert!(s.contains("rustc: rustc 1.80.0"));
        assert!(s.contains("cargo: cargo 1.80.0"));
    }

    #[test]
    fn empty_string_fields_skip_robust() {
        let ctx = EnvContext {
            rustc: Some(String::new()),
            cargo: Some("cargo 1.80".to_owned()),
            ..Default::default()
        };
        let s = ctx.to_prompt_string().unwrap();
        assert!(!s.contains("rustc:"));
        assert!(s.contains("cargo:"));
    }
}
