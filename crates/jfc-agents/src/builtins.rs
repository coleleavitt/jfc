use std::path::{Path, PathBuf};

use crate::state::{Skill, SkillFile, parse_skill};

struct BuiltInSkill {
    name: &'static str,
    body: &'static str,
    skill_path: &'static str,
    package_root: &'static str,
}

struct BuiltInFile {
    package_root: &'static str,
    relative_path: &'static str,
    bytes: &'static [u8],
}

pub fn built_in_skills() -> Vec<Skill> {
    BUILT_IN_SKILLS
        .iter()
        .filter_map(|spec| {
            let source_package_root =
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(spec.package_root);
            let source_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(spec.skill_path);
            let embedded_package = materialize_embedded_package(spec);
            let package_root = embedded_package
                .as_ref()
                .map(|(root, _)| root.clone())
                .unwrap_or(source_package_root);
            let path = embedded_package
                .as_ref()
                .map(|(root, _)| root.join("SKILL.md"))
                .unwrap_or(source_path);
            parse_skill(&path, spec.body).map(|mut skill| {
                if skill.name == "unnamed" || skill.name == "SKILL" {
                    skill.name = spec.name.to_owned();
                }
                skill.source = path;
                skill.package_root = package_root;
                skill.files = embedded_package.map(|(_, files)| files).unwrap_or_default();
                skill
            })
        })
        .collect()
}

fn materialize_embedded_package(spec: &BuiltInSkill) -> Option<(PathBuf, Vec<SkillFile>)> {
    let files: Vec<&BuiltInFile> = BUILT_IN_FILES
        .iter()
        .filter(|file| file.package_root == spec.package_root)
        .collect();
    if files.is_empty() {
        return None;
    }
    let root = builtin_skill_cache_root().join(spec.package_root);
    if write_embedded_file(&root.join("SKILL.md"), spec.body.as_bytes()).is_err() {
        return None;
    }
    let mut out = Vec::new();
    for file in files {
        let path = root.join(file.relative_path);
        if write_embedded_file(&path, file.bytes).is_ok() {
            out.push(SkillFile {
                relative_path: file.relative_path.to_owned(),
                path,
                bytes: file.bytes.len() as u64,
            });
        }
    }
    out.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Some((root, out))
}

fn builtin_skill_cache_root() -> PathBuf {
    std::env::var_os("JFC_BUILTIN_SKILL_CACHE")
        .map(PathBuf::from)
        .or_else(|| dirs::cache_dir().map(|dir| dir.join("jfc").join("builtin-skills")))
        .unwrap_or_else(|| std::env::temp_dir().join("jfc-builtin-skills"))
}

fn write_embedded_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    match std::fs::read(path) {
        Ok(existing) if existing == bytes => return Ok(()),
        Ok(_) => {}
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(contextual_io_error("read embedded cache file", path, error));
        }
        Err(_) => {}
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            contextual_io_error("create embedded cache directory", parent, error)
        })?;
    }
    std::fs::write(path, bytes)
        .map_err(|error| contextual_io_error("write embedded cache file", path, error))
}

fn contextual_io_error(action: &str, path: &Path, source: std::io::Error) -> std::io::Error {
    std::io::Error::new(
        source.kind(),
        format!("{action} `{}` failed: {source}", path.display()),
    )
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

const BUILT_IN_FILES: &[BuiltInFile] = &[
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/frontmatter-skills/run",
        relative_path: "examples/cli.md",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/frontmatter-skills/run/examples/cli.md"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/frontmatter-skills/run",
        relative_path: "examples/server.md",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/frontmatter-skills/run/examples/server.md"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/frontmatter-skills/run",
        relative_path: "examples/tui.md",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/frontmatter-skills/run/examples/tui.md"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/frontmatter-skills/run",
        relative_path: "examples/electron.md",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/frontmatter-skills/run/examples/electron.md"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/frontmatter-skills/run",
        relative_path: "examples/playwright.md",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/frontmatter-skills/run/examples/playwright.md"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/frontmatter-skills/run",
        relative_path: "examples/library.md",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/frontmatter-skills/run/examples/library.md"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/frontmatter-skills/verify",
        relative_path: "examples/cli.md",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/frontmatter-skills/verify/examples/cli.md"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/frontmatter-skills/verify",
        relative_path: "examples/server.md",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/frontmatter-skills/verify/examples/server.md"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/frontmatter-skills/verify",
        relative_path: "examples/tui.md",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/frontmatter-skills/verify/examples/tui.md"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/frontmatter-skills/verify",
        relative_path: "examples/electron.md",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/frontmatter-skills/verify/examples/electron.md"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/frontmatter-skills/verify",
        relative_path: "examples/playwright.md",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/frontmatter-skills/verify/examples/playwright.md"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/frontmatter-skills/verify",
        relative_path: "examples/library.md",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/frontmatter-skills/verify/examples/library.md"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/cowork-plugin",
        relative_path: "references/component-schemas.md",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/by-skill-registration/cowork-plugin/references/component-schemas.md"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/cowork-plugin",
        relative_path: "references/example-plugins.md",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/by-skill-registration/cowork-plugin/references/example-plugins.md"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/cowork-plugin",
        relative_path: "references/search-strategies.md",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/by-skill-registration/cowork-plugin/references/search-strategies.md"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/design-sync",
        relative_path: "lib/bundle.mjs",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/by-skill-registration/design-sync/lib/bundle.mjs"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/design-sync",
        relative_path: "lib/common.mjs",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/by-skill-registration/design-sync/lib/common.mjs"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/design-sync",
        relative_path: "lib/css-fallback.mjs",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/by-skill-registration/design-sync/lib/css-fallback.mjs"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/design-sync",
        relative_path: "lib/css.mjs",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/by-skill-registration/design-sync/lib/css.mjs"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/design-sync",
        relative_path: "lib/detect.mjs",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/by-skill-registration/design-sync/lib/detect.mjs"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/design-sync",
        relative_path: "lib/docs.mjs",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/by-skill-registration/design-sync/lib/docs.mjs"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/design-sync",
        relative_path: "lib/dts.mjs",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/by-skill-registration/design-sync/lib/dts.mjs"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/design-sync",
        relative_path: "lib/emit.mjs",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/by-skill-registration/design-sync/lib/emit.mjs"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/design-sync",
        relative_path: "lib/preview-gen-package.mjs",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/by-skill-registration/design-sync/lib/preview-gen-package.mjs"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/design-sync",
        relative_path: "lib/preview-gen-storybook.mjs",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/by-skill-registration/design-sync/lib/preview-gen-storybook.mjs"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/design-sync",
        relative_path: "lib/previews.mjs",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/by-skill-registration/design-sync/lib/previews.mjs"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/design-sync",
        relative_path: "lib/source-kit.mjs",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/by-skill-registration/design-sync/lib/source-kit.mjs"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/design-sync",
        relative_path: "lib/source-storybook.mjs",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/by-skill-registration/design-sync/lib/source-storybook.mjs"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/design-sync",
        relative_path: "lib/stories-static.mjs",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/by-skill-registration/design-sync/lib/stories-static.mjs"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/design-sync",
        relative_path: "lib/stories.mjs",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/by-skill-registration/design-sync/lib/stories.mjs"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/design-sync",
        relative_path: "package-build.mjs",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/by-skill-registration/design-sync/package-build.mjs"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/design-sync",
        relative_path: "package-validate.mjs",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/by-skill-registration/design-sync/package-validate.mjs"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/design-sync",
        relative_path: "storybook/build.mjs",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/by-skill-registration/design-sync/storybook/build.mjs"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/design-sync",
        relative_path: "storybook/emit.mjs",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/by-skill-registration/design-sync/storybook/emit.mjs"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/design-sync",
        relative_path: "storybook/http-serve.mjs",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/by-skill-registration/design-sync/storybook/http-serve.mjs"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/design-sync",
        relative_path: "storybook/probe.mjs",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/by-skill-registration/design-sync/storybook/probe.mjs"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/design-sync",
        relative_path: "storybook/validate.mjs",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/by-skill-registration/design-sync/storybook/validate.mjs"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/simplify",
        relative_path: "examples/cli.md",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/by-skill-registration/simplify/examples/cli.md"
        ),
    },
    BuiltInFile {
        package_root: "builtin-skills/claude-2.1.167/by-skill-registration/simplify",
        relative_path: "examples/server.md",
        bytes: include_bytes!(
            "../builtin-skills/claude-2.1.167/by-skill-registration/simplify/examples/server.md"
        ),
    },
];

#[cfg(test)]
mod tests {
    use super::{built_in_skills, write_embedded_file};

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
        let run = skills.iter().find(|skill| skill.name == "run").unwrap();
        assert!(
            run.files
                .iter()
                .any(|file| file.relative_path == "examples/cli.md"),
            "run should expose fallback example files"
        );
        let verify = skills.iter().find(|skill| skill.name == "verify").unwrap();
        assert!(
            verify
                .files
                .iter()
                .any(|file| file.relative_path == "examples/server.md"),
            "verify should expose fallback example files"
        );
    }

    #[test]
    fn built_in_skill_files_materialize_from_embedded_bytes_regression() {
        let dir = tempfile::TempDir::new().unwrap();
        let prior = std::env::var_os("JFC_BUILTIN_SKILL_CACHE");
        unsafe { std::env::set_var("JFC_BUILTIN_SKILL_CACHE", dir.path()) };

        let skills = built_in_skills();

        unsafe {
            match prior {
                Some(value) => std::env::set_var("JFC_BUILTIN_SKILL_CACHE", value),
                None => std::env::remove_var("JFC_BUILTIN_SKILL_CACHE"),
            }
        }
        let design_sync = skills
            .iter()
            .find(|skill| skill.name == "design-sync")
            .unwrap();
        let package_build = design_sync
            .files
            .iter()
            .find(|file| file.relative_path == "package-build.mjs")
            .unwrap();

        assert!(design_sync.package_root.starts_with(dir.path()));
        assert!(package_build.path.starts_with(dir.path()));
        assert!(package_build.path.is_file());
        assert_eq!(
            std::fs::metadata(&package_build.path).unwrap().len(),
            package_build.bytes
        );
    }

    #[test]
    fn write_embedded_file_rewrites_same_length_stale_content_regression() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("skill").join("SKILL.md");
        write_embedded_file(&path, b"aaaa").unwrap();

        write_embedded_file(&path, b"bbbb").unwrap();

        assert_eq!(std::fs::read(path).unwrap(), b"bbbb");
    }
}
