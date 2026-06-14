//! Design projects: on-disk model, sandboxed file store, and asset registry.
//!
//! A project lives under `<base>/<id>/` where `<base>` defaults to
//! `.jfc/design/projects` in the current working directory. Metadata is kept in
//! `<id>/project.json`; everything else under the directory is project content.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{DesignError, Result, io_err};

/// One registered deliverable asset (mirrors Claude Design's asset-review pane).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Asset {
    /// Display name shown to the user.
    pub name: String,
    /// Project-relative path of the asset file.
    pub path: String,
}

/// Persisted project metadata (`project.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMeta {
    pub id: String,
    pub title: String,
    /// Whether this project is itself a design system (changes agent behavior).
    #[serde(default)]
    pub is_design_system: bool,
    /// Registered deliverable assets, newest last.
    #[serde(default)]
    pub assets: Vec<Asset>,
}

/// Root store that owns the projects directory.
#[derive(Debug, Clone)]
pub struct ProjectStore {
    base: PathBuf,
}

impl ProjectStore {
    /// Open (creating if needed) the store rooted at `base`.
    pub fn new(base: impl Into<PathBuf>) -> Result<Self> {
        let base = base.into();
        std::fs::create_dir_all(&base).map_err(|e| io_err(&base, e))?;
        let base = std::fs::canonicalize(&base).map_err(|e| io_err(&base, e))?;
        Ok(Self { base })
    }

    /// The conventional store under `<cwd>/.jfc/design/projects`.
    pub fn default_in(cwd: impl AsRef<Path>) -> Result<Self> {
        Self::new(cwd.as_ref().join(".jfc/design/projects"))
    }

    pub fn base(&self) -> &Path {
        &self.base
    }

    /// Create a new project with a generated id and the given title.
    pub fn create(&self, title: impl Into<String>) -> Result<DesignProject> {
        let title = title.into();
        let id = generate_id(&title);
        let root = self.base.join(&id);
        std::fs::create_dir_all(&root).map_err(|e| io_err(&root, e))?;
        std::fs::create_dir_all(root.join("scraps")).ok();
        let meta = ProjectMeta {
            id,
            title,
            is_design_system: false,
            assets: Vec::new(),
        };
        let project = DesignProject { root, meta };
        project.save_meta()?;
        Ok(project)
    }

    /// List all projects in id order.
    pub fn list(&self) -> Result<Vec<ProjectMeta>> {
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&self.base) {
            Ok(e) => e,
            Err(_) => return Ok(out),
        };
        for entry in entries.flatten() {
            let meta_path = entry.path().join("project.json");
            if let Ok(raw) = std::fs::read_to_string(&meta_path)
                && let Ok(meta) = serde_json::from_str::<ProjectMeta>(&raw)
            {
                out.push(meta);
            }
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    /// Open an existing project by id.
    pub fn open(&self, id: &str) -> Result<DesignProject> {
        let root = self.project_root(id)?;
        let meta_path = root.join("project.json");
        let raw = std::fs::read_to_string(&meta_path)
            .map_err(|_| DesignError::ProjectNotFound(id.to_owned()))?;
        let meta =
            serde_json::from_str(&raw).map_err(|e| DesignError::BadMetadata(e.to_string()))?;
        Ok(DesignProject { root, meta })
    }

    fn project_root(&self, id: &str) -> Result<PathBuf> {
        let mut components = Path::new(id).components();
        match (components.next(), components.next()) {
            (Some(Component::Normal(_)), None) => Ok(self.base.join(id)),
            _ => Err(DesignError::PathEscape(id.to_owned())),
        }
    }
}

/// A single design project rooted at a directory.
#[derive(Debug, Clone)]
pub struct DesignProject {
    root: PathBuf,
    meta: ProjectMeta,
}

impl DesignProject {
    /// Treat an arbitrary directory as a project root (for ad-hoc, store-less use,
    /// e.g. the preview server pointed at a working directory).
    pub fn at(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let meta = std::fs::read_to_string(root.join("project.json"))
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_else(|| ProjectMeta {
                id: root
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("project")
                    .to_owned(),
                title: root
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Untitled")
                    .to_owned(),
                is_design_system: false,
                assets: Vec::new(),
            });
        Self { root, meta }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn meta(&self) -> &ProjectMeta {
        &self.meta
    }

    /// Resolve a project-relative path, rejecting any traversal outside the root.
    pub fn resolve(&self, rel: &str) -> Result<PathBuf> {
        let rel = rel.trim_start_matches('/');
        let mut out = self.root.clone();
        for comp in Path::new(rel).components() {
            match comp {
                Component::Normal(c) => out.push(c),
                Component::CurDir => {}
                // Anything that could escape the sandbox is rejected.
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(DesignError::PathEscape(rel.to_owned()));
                }
            }
        }
        Ok(out)
    }

    pub fn read_file(&self, rel: &str) -> Result<Vec<u8>> {
        let p = self.resolve(rel)?;
        std::fs::read(&p).map_err(|e| io_err(&p, e))
    }

    pub fn read_to_string(&self, rel: &str) -> Result<String> {
        let p = self.resolve(rel)?;
        std::fs::read_to_string(&p).map_err(|e| io_err(&p, e))
    }

    /// Write a file (creating parent dirs). When `asset` is set, the file is
    /// registered as a deliverable in the asset review list.
    pub fn write_file(&mut self, rel: &str, bytes: &[u8], asset: Option<&str>) -> Result<()> {
        let p = self.resolve(rel)?;
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
        }
        std::fs::write(&p, bytes).map_err(|e| io_err(&p, e))?;
        if let Some(name) = asset {
            self.register_asset(name, rel)?;
        }
        Ok(())
    }

    /// Copy a file from one project-relative path to another (both sandboxed).
    pub fn copy_file(&self, from_rel: &str, to_rel: &str) -> Result<()> {
        let from = self.resolve(from_rel)?;
        let to = self.resolve(to_rel)?;
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
        }
        std::fs::copy(&from, &to).map_err(|e| io_err(&from, e))?;
        Ok(())
    }

    /// Delete a project-relative file or directory, and unregister any matching
    /// deliverable asset. The path is sandboxed before removal.
    pub fn delete_path(&mut self, rel: &str) -> Result<()> {
        if targets_project_root(rel) {
            return Err(DesignError::PathEscape(rel.to_owned()));
        }
        let p = self.resolve(rel)?;
        if p.is_dir() {
            std::fs::remove_dir_all(&p).map_err(|e| io_err(&p, e))?;
        } else {
            std::fs::remove_file(&p).map_err(|e| io_err(&p, e))?;
        }
        self.unregister_asset(rel)?;
        Ok(())
    }

    /// List every file in the project (relative paths, sorted), skipping metadata
    /// and dotfiles.
    pub fn list_files(&self) -> Vec<String> {
        let mut out = Vec::new();
        for entry in walkdir::WalkDir::new(&self.root)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let Ok(rel) = entry.path().strip_prefix(&self.root) else {
                continue;
            };
            let rel = rel.to_string_lossy().replace('\\', "/");
            if rel == "project.json" || rel.starts_with('.') {
                continue;
            }
            out.push(rel);
        }
        out.sort();
        out
    }

    /// Register (or update) a deliverable asset and persist metadata.
    pub fn register_asset(&mut self, name: &str, rel: &str) -> Result<()> {
        let rel = rel.trim_start_matches('/').to_owned();
        self.meta.assets.retain(|a| a.path != rel);
        self.meta.assets.push(Asset {
            name: name.to_owned(),
            path: rel,
        });
        self.save_meta()
    }

    /// Remove a deliverable asset by project-relative path. Returns whether the
    /// registry changed.
    pub fn unregister_asset(&mut self, rel: &str) -> Result<bool> {
        let rel = rel.trim_start_matches('/');
        let before = self.meta.assets.len();
        self.meta.assets.retain(|a| a.path != rel);
        let changed = self.meta.assets.len() != before;
        if changed {
            self.save_meta()?;
        }
        Ok(changed)
    }

    pub fn set_title(&mut self, title: impl Into<String>) -> Result<()> {
        self.meta.title = title.into();
        self.save_meta()
    }

    pub fn set_is_design_system(&mut self, yes: bool) -> Result<()> {
        self.meta.is_design_system = yes;
        self.save_meta()
    }

    fn save_meta(&self) -> Result<()> {
        let p = self.root.join("project.json");
        let json = serde_json::to_string_pretty(&self.meta)?;
        std::fs::write(&p, json).map_err(|e| io_err(&p, e))
    }
}

fn targets_project_root(rel: &str) -> bool {
    let rel = rel.trim_start_matches('/');
    Path::new(rel)
        .components()
        .all(|comp| matches!(comp, Component::CurDir))
}

/// Generate a short, filesystem-safe project id from a title plus a time-derived
/// suffix so repeated titles don't collide.
fn generate_id(title: &str) -> String {
    let slug: String = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-');
    let slug: String = slug
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = if slug.is_empty() {
        "project".to_owned()
    } else {
        slug.chars().take(40).collect()
    };
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{slug}-{:x}", (nanos as u64) & 0xffff_ffff)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("jfc_design_test_{n}"));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn create_open_roundtrip_normal() {
        let base = tmp();
        let store = ProjectStore::new(&base).unwrap();
        let p = store.create("My Deck").unwrap();
        assert!(p.meta().id.starts_with("my-deck-"));
        let reopened = store.open(&p.meta().id).unwrap();
        assert_eq!(reopened.meta().title, "My Deck");
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn store_paths_are_absolute_normal() {
        let rel = PathBuf::from("target").join(format!(
            "jfc_design_relative_store_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = ProjectStore::new(&rel).unwrap();
        let p = store.create("Relative").unwrap();
        assert!(store.base().is_absolute());
        assert!(p.root().is_absolute());
        std::fs::remove_dir_all(store.base()).ok();
    }

    #[test]
    fn write_and_list_and_asset_normal() {
        let base = tmp();
        let store = ProjectStore::new(&base).unwrap();
        let mut p = store.create("Site").unwrap();
        p.write_file("index.html", b"<h1>hi</h1>", Some("Landing"))
            .unwrap();
        p.write_file("css/app.css", b"body{}", None).unwrap();
        let files = p.list_files();
        assert!(files.contains(&"index.html".to_owned()));
        assert!(files.contains(&"css/app.css".to_owned()));
        assert_eq!(p.meta().assets.len(), 1);
        assert_eq!(p.meta().assets[0].name, "Landing");
        // metadata persisted
        let reopened = store.open(&p.meta().id).unwrap();
        assert_eq!(reopened.meta().assets.len(), 1);
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn path_traversal_is_rejected_robust() {
        let base = tmp();
        let store = ProjectStore::new(&base).unwrap();
        let p = store.create("x").unwrap();
        assert!(matches!(
            store.open("../escape"),
            Err(DesignError::PathEscape(_))
        ));
        assert!(matches!(
            p.resolve("../escape"),
            Err(DesignError::PathEscape(_))
        ));
        assert!(matches!(
            p.resolve("a/../../escape"),
            Err(DesignError::PathEscape(_))
        ));
        // A leading slash is treated as project-relative, so this stays sandboxed
        // (root/etc/passwd) rather than reaching the real /etc/passwd.
        let abs = p.resolve("/etc/passwd").unwrap();
        assert!(abs.starts_with(p.root()));
        assert!(p.resolve("a/b/c.html").is_ok());
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn delete_path_rejects_project_root_robust() {
        let base = tmp();
        let store = ProjectStore::new(&base).unwrap();
        let mut p = store.create("x").unwrap();
        p.write_file("index.html", b"<h1>hi</h1>", None).unwrap();

        for rel in ["", "/", ".", "./", "/."] {
            assert!(matches!(
                p.delete_path(rel),
                Err(DesignError::PathEscape(_))
            ));
            assert!(p.root().exists());
            assert!(p.resolve("index.html").unwrap().exists());
        }

        std::fs::remove_dir_all(&base).ok();
    }
}
