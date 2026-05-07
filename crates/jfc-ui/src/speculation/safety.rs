//! Path validation for the speculation overlay.
//!
//! Mirrors v132's `nA_` parent-dir-escape check and symlink rejection: any
//! candidate write path must (a) lexically resolve underneath the project
//! root after `..` collapsing and (b) not traverse a symlink that points
//! outside the project root. Writes targeting well-known sensitive system
//! paths (`/etc`, `~/.ssh`, etc.) are also refused even if a misconfigured
//! cwd would otherwise allow them.
//!
//! All checks are lexical + a bounded `read_link` walk; no canonicalisation
//! of the leaf is required so missing files (the common Write case) are
//! still validatable.
use std::io;
use std::path::{Component, Path, PathBuf};

/// Reasons a candidate path is rejected by the speculation gate.
#[derive(Debug, thiserror::Error)]
pub enum SafetyError {
    /// The path lexically escapes the project root via `..`.
    #[error("path escapes project root: {0}")]
    Escape(PathBuf),
    /// A component of the path is a symlink leaving the project root.
    #[error("symlink escapes project root: {link} -> {target}")]
    SymlinkEscape {
        /// The symlink that was followed.
        link: PathBuf,
        /// The (possibly relative) target the symlink resolved to.
        target: PathBuf,
    },
    /// The path falls under a hard-coded sensitive prefix (`/etc`, `~/.ssh`, ...).
    #[error("write to system path forbidden: {0}")]
    SystemPath(PathBuf),
    /// The candidate path is not absolute.
    #[error("path must be absolute: {0}")]
    NotAbsolute(PathBuf),
}

impl From<SafetyError> for io::Error {
    fn from(err: SafetyError) -> Self {
        io::Error::new(io::ErrorKind::PermissionDenied, err.to_string())
    }
}

/// Hard-coded denylist of host-sensitive prefixes. We refuse to redirect
/// these even into the overlay because the overlay copy-back would happily
/// move them straight to the real filesystem on commit.
fn forbidden_prefixes() -> Vec<PathBuf> {
    let mut v = vec![
        PathBuf::from("/etc"),
        PathBuf::from("/boot"),
        PathBuf::from("/sys"),
        PathBuf::from("/proc"),
        PathBuf::from("/dev"),
        PathBuf::from("/root"),
        PathBuf::from("/var/run"),
        PathBuf::from("/var/lib"),
    ];
    if let Some(home) = dirs::home_dir() {
        v.push(home.join(".ssh"));
        v.push(home.join(".gnupg"));
        v.push(home.join(".aws"));
        v.push(home.join(".config/gh"));
    }
    v
}

/// Lexically normalise a path: collapse `.` and pop on `..`. Does not touch
/// the filesystem. Returns `None` if `..` would pop above the path's root.
pub fn normalize(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Prefix(p) => out.push(p.as_os_str()),
            Component::RootDir => out.push(Component::RootDir),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::Normal(n) => out.push(n),
        }
    }
    Some(out)
}

/// True if `child` is `root` or lies underneath it (lexical comparison).
pub fn is_within(root: &Path, child: &Path) -> bool {
    child.starts_with(root)
}

/// Validate that `candidate` is safe to redirect into the speculation overlay
/// rooted at the given project `cwd`.
///
/// Returns the lexically-normalised absolute path on success.
pub fn validate(cwd: &Path, candidate: &Path) -> Result<PathBuf, SafetyError> {
    if !candidate.is_absolute() {
        return Err(SafetyError::NotAbsolute(candidate.to_owned()));
    }
    let normalized = normalize(candidate).ok_or_else(|| SafetyError::Escape(candidate.to_owned()))?;
    let cwd_norm = normalize(cwd).unwrap_or_else(|| cwd.to_owned());

    if !is_within(&cwd_norm, &normalized) {
        return Err(SafetyError::Escape(normalized));
    }

    for forbidden in forbidden_prefixes() {
        if normalized.starts_with(&forbidden) {
            return Err(SafetyError::SystemPath(normalized));
        }
    }

    // Walk every existing ancestor and reject if any component is a symlink
    // pointing outside `cwd_norm`. We stop at the first component that does
    // not yet exist (typical for Write to a brand-new file).
    let mut walk = PathBuf::new();
    for c in normalized.components() {
        match c {
            Component::Prefix(p) => walk.push(p.as_os_str()),
            Component::RootDir => walk.push(Component::RootDir),
            Component::CurDir => {}
            Component::ParentDir => {
                walk.pop();
            }
            Component::Normal(n) => walk.push(n),
        }
        let meta = match std::fs::symlink_metadata(&walk) {
            Ok(m) => m,
            Err(_) => break,
        };
        if meta.file_type().is_symlink() {
            let target = std::fs::read_link(&walk).unwrap_or_else(|_| PathBuf::new());
            let resolved = if target.is_absolute() {
                normalize(&target).unwrap_or(target.clone())
            } else {
                let parent = walk.parent().unwrap_or(Path::new("/"));
                normalize(&parent.join(&target)).unwrap_or_else(|| parent.join(&target))
            };
            if !is_within(&cwd_norm, &resolved) {
                return Err(SafetyError::SymlinkEscape {
                    link: walk.clone(),
                    target,
                });
            }
        }
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use tempfile::TempDir;

    #[test]
    fn validate_simple_path_normal() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        let p = cwd.join("foo.txt");
        let v = validate(cwd, &p).expect("simple write should pass");
        assert_eq!(v, p);
    }

    #[test]
    fn validate_nested_path_normal() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        let p = cwd.join("a/b/c/foo.txt");
        let v = validate(cwd, &p).expect("nested write should pass");
        assert_eq!(v, p);
    }

    #[test]
    fn validate_collapsed_path_normal() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        // cwd/a/../foo.txt → cwd/foo.txt (still inside cwd)
        let p = cwd.join("a").join("..").join("foo.txt");
        let v = validate(cwd, &p).expect("collapse to inside cwd should pass");
        assert_eq!(v, cwd.join("foo.txt"));
    }

    #[test]
    fn rejects_dotdot_escape_robust() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        // cwd/../../../etc/passwd should escape
        let p = cwd
            .join("..")
            .join("..")
            .join("..")
            .join("etc")
            .join("passwd");
        let err = validate(cwd, &p).expect_err("escape should be rejected");
        assert!(matches!(
            err,
            SafetyError::Escape(_) | SafetyError::SystemPath(_)
        ));
    }

    #[test]
    fn rejects_etc_passwd_robust() {
        let tmp = TempDir::new().unwrap();
        let p = PathBuf::from("/etc/passwd");
        let err = validate(tmp.path(), &p).expect_err("system path");
        // Either Escape (cwd not /etc) or SystemPath (denylist) is acceptable;
        // both forbid the operation.
        assert!(matches!(
            err,
            SafetyError::Escape(_) | SafetyError::SystemPath(_)
        ));
    }

    #[test]
    fn rejects_relative_robust() {
        let tmp = TempDir::new().unwrap();
        let err = validate(tmp.path(), Path::new("foo.txt")).expect_err("relative");
        assert!(matches!(err, SafetyError::NotAbsolute(_)));
    }

    #[test]
    fn rejects_symlink_escape_robust() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        // Create a symlink inside cwd that points to /tmp.
        let link = cwd.join("escape");
        symlink("/tmp", &link).expect("create symlink");
        let target = link.join("evil.txt");
        let err = validate(cwd, &target).expect_err("symlink escape rejected");
        assert!(matches!(err, SafetyError::SymlinkEscape { .. }));
    }

    #[test]
    fn allows_internal_symlink_normal() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path();
        std::fs::create_dir_all(cwd.join("real")).unwrap();
        let link = cwd.join("alias");
        symlink(cwd.join("real"), &link).expect("create internal symlink");
        let target = link.join("inside.txt");
        validate(cwd, &target).expect("internal symlink should pass");
    }

    #[test]
    fn normalize_pops_dotdot_normal() {
        let p = Path::new("/a/b/../c");
        assert_eq!(normalize(p).unwrap(), PathBuf::from("/a/c"));
    }

    #[test]
    fn normalize_rejects_above_root_robust() {
        let p = Path::new("/..");
        assert!(normalize(p).is_none());
    }
}
