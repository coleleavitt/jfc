//! Short-lived prompt context cache for expensive static prompt sections.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};

use jfc_memory::MemoryEntry;

const TTL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
pub struct ContextHierarchySnapshot {
    pub rendered: Option<String>,
    pub disallowed_tools: Vec<String>,
}

#[derive(Debug, Clone)]
struct Timed<T> {
    created_at: Instant,
    value: T,
}

static SKILLS: OnceLock<RwLock<HashMap<PathBuf, Timed<String>>>> = OnceLock::new();
static DISPATCH: OnceLock<RwLock<HashMap<PathBuf, Timed<String>>>> = OnceLock::new();
static HIERARCHY: OnceLock<RwLock<HashMap<String, Timed<ContextHierarchySnapshot>>>> =
    OnceLock::new();
static MEMORIES: OnceLock<RwLock<HashMap<PathBuf, Timed<Vec<MemoryEntry>>>>> = OnceLock::new();

pub fn skills_listing(cwd: &Path) -> String {
    cached_path_value(SKILLS.get_or_init(Default::default), cwd, || {
        let skills = crate::agents::load_skills(cwd);
        let block = crate::agents::render_skills_section(&skills);
        if block.is_empty() {
            String::new()
        } else {
            format!(
                "{block}\nTo use a listed skill, call the Skill tool with \
                 `name` set to the listed skill name and optional `args` for \
                 extra context. On OpenAI-compatible routes the callable may \
                 be advertised as lowercase `skill`; use the exact callable \
                 name shown in the tool list."
            )
        }
    })
}

pub fn dispatch_section(cwd: &Path) -> String {
    cached_path_value(DISPATCH.get_or_init(Default::default), cwd, || {
        let agents = crate::agents::load_agents(cwd);
        crate::agents::render_dispatch_section(&agents)
    })
}

pub fn context_hierarchy(cwd: &Path, extra_dirs: &[PathBuf]) -> ContextHierarchySnapshot {
    let key = hierarchy_key(cwd, extra_dirs);
    cached_string_value(HIERARCHY.get_or_init(Default::default), key, || {
        let hierarchy = crate::context::ClaudeMdHierarchy::load_with_extra_roots(cwd, extra_dirs);
        ContextHierarchySnapshot {
            rendered: hierarchy.render(),
            disallowed_tools: hierarchy.collect_disallowed_tools(),
        }
    })
}

pub fn memories(cwd: &Path) -> Vec<MemoryEntry> {
    cached_path_value(MEMORIES.get_or_init(Default::default), cwd, || {
        jfc_knowledge::block_on_knowledge(async { crate::memory::load_all_memories(cwd).await })
    })
}

fn cached_path_value<T, F>(cache: &RwLock<HashMap<PathBuf, Timed<T>>>, path: &Path, load: F) -> T
where
    T: Clone,
    F: FnOnce() -> T,
{
    let key = path.to_path_buf();
    if let Some(value) = fresh(cache.read().ok().as_deref(), &key) {
        return value;
    }
    let value = load();
    if let Ok(mut guard) = cache.write() {
        guard.insert(
            key,
            Timed {
                created_at: Instant::now(),
                value: value.clone(),
            },
        );
    }
    value
}

fn cached_string_value<T, F>(cache: &RwLock<HashMap<String, Timed<T>>>, key: String, load: F) -> T
where
    T: Clone,
    F: FnOnce() -> T,
{
    if let Some(value) = fresh(cache.read().ok().as_deref(), &key) {
        return value;
    }
    let value = load();
    if let Ok(mut guard) = cache.write() {
        guard.insert(
            key,
            Timed {
                created_at: Instant::now(),
                value: value.clone(),
            },
        );
    }
    value
}

fn fresh<K, T>(guard: Option<&HashMap<K, Timed<T>>>, key: &K) -> Option<T>
where
    K: std::hash::Hash + Eq,
    T: Clone,
{
    guard
        .and_then(|map| map.get(key))
        .filter(|entry| entry.created_at.elapsed() <= TTL)
        .map(|entry| entry.value.clone())
}

fn hierarchy_key(cwd: &Path, extra_dirs: &[PathBuf]) -> String {
    let mut key = cwd.display().to_string();
    for extra in extra_dirs {
        key.push('\0');
        key.push_str(&extra.display().to_string());
    }
    key
}
