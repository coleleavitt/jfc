use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::state::{Skill, SkillFile, parse_skill};

struct BuiltInSkill {
    name: &'static str,
    body: &'static str,
    skill_path: &'static str,
    package_root: &'static str,
}

pub fn built_in_skills() -> Vec<Skill> {
    BUILT_IN_SKILLS
        .iter()
        .filter_map(|spec| {
            let package_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(spec.package_root);
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(spec.skill_path);
            parse_skill(&path, spec.body).map(|mut skill| {
                if skill.name == "unnamed" || skill.name == "SKILL" {
                    skill.name = spec.name.to_owned();
                }
                skill.source = path;
                skill.package_root = package_root;
                skill.files = collect_builtin_files(&skill.package_root, &skill.source);
                skill
            })
        })
        .collect()
}

fn collect_builtin_files(package_root: &Path, skill_md_path: &Path) -> Vec<SkillFile> {
    const MAX_SCAN_DEPTH: usize = 8;
    const MAX_DIRS: usize = 512;
    const MAX_FILES: usize = 512;

    if !package_root.is_dir() {
        return Vec::new();
    }

    let canonical_skill = skill_md_path.canonicalize().ok();
    let mut out = Vec::new();
    let mut queue = std::collections::VecDeque::from([(package_root.to_path_buf(), 0usize)]);
    let mut seen_dirs = HashSet::new();
    if let Ok(canon) = package_root.canonicalize() {
        seen_dirs.insert(canon);
    }

    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if file_name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                if depth >= MAX_SCAN_DEPTH || seen_dirs.len() >= MAX_DIRS {
                    continue;
                }
                if let Ok(canon) = path.canonicalize()
                    && seen_dirs.insert(canon)
                {
                    queue.push_back((path, depth + 1));
                }
                continue;
            }
            if !path.is_file() {
                continue;
            }
            if canonical_skill
                .as_ref()
                .is_some_and(|skill| path.canonicalize().ok().as_ref() == Some(skill))
            {
                continue;
            }
            let Ok(metadata) = std::fs::metadata(&path) else {
                continue;
            };
            let relative_path = path
                .strip_prefix(package_root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.push(SkillFile {
                relative_path,
                path,
                bytes: metadata.len(),
            });
            if out.len() >= MAX_FILES {
                return out;
            }
        }
    }
    out.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    out
}

const BUILT_IN_SKILLS: &[BuiltInSkill] = &[
    BuiltInSkill {
        name: "catch-up",
        body: include_str!("../builtin-skills/claude-2.1.167/frontmatter-skills/catch-up/SKILL.md"),
        skill_path: "builtin-skills/claude-2.1.167/frontmatter-skills/catch-up/SKILL.md",
        package_root: "builtin-skills/claude-2.1.167/frontmatter-skills/catch-up",
    },
    BuiltInSkill {
        name: "dream",
        body: include_str!("../builtin-skills/claude-2.1.167/frontmatter-skills/dream/SKILL.md"),
        skill_path: "builtin-skills/claude-2.1.167/frontmatter-skills/dream/SKILL.md",
        package_root: "builtin-skills/claude-2.1.167/frontmatter-skills/dream",
    },
    BuiltInSkill {
        name: "morning-checkin",
        body: include_str!(
            "../builtin-skills/claude-2.1.167/frontmatter-skills/morning-checkin/SKILL.md"
        ),
        skill_path: "builtin-skills/claude-2.1.167/frontmatter-skills/morning-checkin/SKILL.md",
        package_root: "builtin-skills/claude-2.1.167/frontmatter-skills/morning-checkin",
    },
    BuiltInSkill {
        name: "pre-meeting-checkin",
        body: include_str!(
            "../builtin-skills/claude-2.1.167/frontmatter-skills/pre-meeting-checkin/SKILL.md"
        ),
        skill_path: "builtin-skills/claude-2.1.167/frontmatter-skills/pre-meeting-checkin/SKILL.md",
        package_root: "builtin-skills/claude-2.1.167/frontmatter-skills/pre-meeting-checkin",
    },
    BuiltInSkill {
        name: "run",
        body: include_str!("../builtin-skills/claude-2.1.167/frontmatter-skills/run/SKILL.md"),
        skill_path: "builtin-skills/claude-2.1.167/frontmatter-skills/run/SKILL.md",
        package_root: "builtin-skills/claude-2.1.167/frontmatter-skills/run",
    },
    BuiltInSkill {
        name: "verify",
        body: include_str!("../builtin-skills/claude-2.1.167/frontmatter-skills/verify/SKILL.md"),
        skill_path: "builtin-skills/claude-2.1.167/frontmatter-skills/verify/SKILL.md",
        package_root: "builtin-skills/claude-2.1.167/frontmatter-skills/verify",
    },
    BuiltInSkill {
        name: "run-skill-generator",
        body: include_str!(
            "../builtin-skills/claude-2.1.167/frontmatter-skills/run-skill-generator/SKILL.md"
        ),
        skill_path: "builtin-skills/claude-2.1.167/frontmatter-skills/run-skill-generator/SKILL.md",
        package_root: "builtin-skills/claude-2.1.167/frontmatter-skills/run-skill-generator",
    },
    BuiltInSkill {
        name: "run-<unit-name>",
        body: include_str!(
            "../builtin-skills/claude-2.1.167/frontmatter-skills/run--unit-name/SKILL.md"
        ),
        skill_path: "builtin-skills/claude-2.1.167/frontmatter-skills/run--unit-name/SKILL.md",
        package_root: "builtin-skills/claude-2.1.167/frontmatter-skills/run--unit-name",
    },
    BuiltInSkill {
        name: "cowork-plugin",
        body: r#"---
name: cowork-plugin
description: "Build or inspect cowork plugins: components, search strategies, and plugin-facing schemas."
---
# Cowork Plugin

Use this skill when creating or reviewing a cowork plugin. Identify the plugin manifest, exposed components, search/discovery behavior, permissions, and user-facing entry points.

Keep plugin APIs schema-driven. Validate component schemas, provide small example plugins, and include search strategies that map user intent to the right plugin surface without advertising unrelated tools.
"#,
        skill_path: "builtin-skills/claude-2.1.167/by-skill-registration/cowork-plugin/SKILL.md",
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/cowork-plugin",
    },
    BuiltInSkill {
        name: "design-sync",
        body: r#"---
name: design-sync
description: Sync component source, styles, docs, and preview metadata for design handoff workflows.
---
# Design Sync

Use this skill when packaging UI components or design assets for inspection. Detect the project framework, collect component source, CSS, type definitions, stories or examples, and build metadata.

Prefer existing project scripts. If Storybook or another preview system exists, build or validate through it; otherwise emit a minimal static preview package. Keep generated packages deterministic and verify that referenced files exist before reporting success.
"#,
        skill_path: "builtin-skills/claude-2.1.167/by-skill-registration/design-sync/SKILL.md",
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/design-sync",
    },
    BuiltInSkill {
        name: "simplify",
        body: r#"---
name: simplify
description: Reduce unnecessary complexity while preserving behavior and tests.
---
# Simplify

Use this skill for targeted simplification. First identify the behavior that must remain unchanged, then remove dead paths, collapse needless abstractions, clarify names, and reduce branching only where the code becomes easier to reason about.

Avoid broad rewrites. Keep diffs reviewable, preserve public contracts, and run the narrowest checks that cover the changed behavior.
"#,
        skill_path: "builtin-skills/claude-2.1.167/by-skill-registration/simplify/SKILL.md",
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/simplify",
    },
];

#[cfg(test)]
mod tests {
    use super::built_in_skills;

    #[test]
    fn built_in_skills_include_167_skill_pack_normal() {
        let skills = built_in_skills();
        let names: Vec<&str> = skills.iter().map(|skill| skill.name.as_str()).collect();
        for needed in [
            "catch-up",
            "dream",
            "morning-checkin",
            "pre-meeting-checkin",
            "run",
            "verify",
            "run-skill-generator",
            "cowork-plugin",
            "design-sync",
            "simplify",
        ] {
            assert!(names.contains(&needed), "missing {needed} in {names:?}");
        }
        let catch_up = skills
            .iter()
            .find(|skill| skill.name == "catch-up")
            .unwrap();
        assert!(catch_up.context.is_fork());
        assert!(catch_up.schedule.is_none());
        let verify = skills.iter().find(|skill| skill.name == "verify").unwrap();
        assert!(verify.body.contains("Verification is runtime observation"));
        let design_sync = skills
            .iter()
            .find(|skill| skill.name == "design-sync")
            .unwrap();
        assert!(
            design_sync
                .files
                .iter()
                .any(|file| file.relative_path == "package-build.mjs"),
            "design-sync should expose extracted package files"
        );
    }
}
