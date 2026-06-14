//! Developer-handoff package generator.
//!
//! Scaffolds `design_handoff_<feature>/` next to the design files: copies the
//! referenced files in and writes a `README.md` skeleton the agent then fills with
//! precise tokens, screens, and interactions (see the `handoff-to-claude-code` skill).

use std::path::{Path, PathBuf};

use crate::{Result, io_err};

/// Result of scaffolding a handoff package.
#[derive(Debug, Clone)]
pub struct HandoffPackage {
    pub dir: PathBuf,
    pub readme: PathBuf,
    pub copied: Vec<String>,
}

/// Create `design_handoff_<feature>/` under `project_dir`, copy `files` into it, and
/// write a README skeleton. `files` are paths relative to `project_dir` (or absolute).
pub fn scaffold(
    project_dir: impl AsRef<Path>,
    feature: &str,
    files: &[String],
) -> Result<HandoffPackage> {
    let project_dir = project_dir.as_ref();
    let slug = slugify(feature);
    let dir = project_dir.join(format!("design_handoff_{slug}"));
    std::fs::create_dir_all(&dir).map_err(|e| io_err(&dir, e))?;

    let mut copied = Vec::new();
    for f in files {
        let src = if Path::new(f).is_absolute() {
            PathBuf::from(f)
        } else {
            project_dir.join(f)
        };
        let Some(name) = src.file_name() else {
            continue;
        };
        let dst = dir.join(name);
        if std::fs::copy(&src, &dst).is_ok() {
            copied.push(name.to_string_lossy().into_owned());
        }
    }

    let readme = dir.join("README.md");
    std::fs::write(&readme, readme_skeleton(feature, &copied)).map_err(|e| io_err(&readme, e))?;

    Ok(HandoffPackage {
        dir,
        readme,
        copied,
    })
}

fn slugify(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    out.split('_')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

fn readme_skeleton(feature: &str, files: &[String]) -> String {
    let files_list = if files.is_empty() {
        "- _(add the design files this package documents)_".to_owned()
    } else {
        files
            .iter()
            .map(|f| format!("- `{f}`"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        r#"# Handoff: {feature}

## Overview
<!-- What this design is for and what it accomplishes. -->

## About the design files
The files in this bundle are **design references created in HTML** — prototypes
showing the intended look and behavior, **not** production code to copy directly.
The task is to **recreate these designs in the target codebase's existing
environment** (React, Vue, SwiftUI, native, …) using its established patterns and
libraries — or, if no environment exists yet, to choose the most appropriate
framework and implement them there.

## Fidelity
<!-- State hi-fi (pixel-perfect; recreate exactly using the codebase's libraries) or
     lo-fi (wireframe; use as a guide for layout/flow, apply the codebase's design
     system for styling). -->

## Screens / Views
<!-- Per screen: Name · Purpose · Layout (grid/flex, widths, heights, margins,
     padding) · Components (position/size, exact hex colors, typography
     family/size/weight/line-height/letter-spacing, radius/shadow/borders,
     hover/active/focus states, exact copy). -->

## Interactions & Behavior
<!-- Click handlers, navigation flows, animations (duration/easing/properties),
     hover/loading/error states, form validation, responsive behavior. -->

## State Management
<!-- State variables, transitions and triggers, data fetching. -->

## Design Tokens
<!-- Colors (hex), spacing scale, type scale, radii, shadows. -->

## Assets
<!-- Images/icons used and where they came from. -->

## Files
{files_list}
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaffolds_package_normal() {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("jfc_handoff_{n}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Onboarding.html"), b"<h1>x</h1>").unwrap();
        let pkg = scaffold(&dir, "Onboarding Flow", &["Onboarding.html".to_owned()]).unwrap();
        assert!(pkg.dir.ends_with("design_handoff_onboarding_flow"));
        assert!(pkg.readme.exists());
        assert_eq!(pkg.copied, vec!["Onboarding.html".to_owned()]);
        let readme = std::fs::read_to_string(&pkg.readme).unwrap();
        assert!(readme.contains("# Handoff: Onboarding Flow"));
        assert!(readme.contains("`Onboarding.html`"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
