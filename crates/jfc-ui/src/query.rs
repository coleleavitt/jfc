#![allow(dead_code)]

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

/// Stable fingerprint for query inputs. This is intentionally smaller than
/// rustc's `Fingerprint`: jfc only needs a deterministic key for in-process
/// UI/data queries, not cross-version incremental compilation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint(u64);

impl Fingerprint {
    pub fn new(value: impl Hash) -> Self {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        value.hash(&mut hasher);
        Self(hasher.finish())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum QueryKey {
    ModelPickerModels(Fingerprint),
}

#[derive(Debug, Default)]
pub struct QueryCache<V> {
    values: Mutex<HashMap<QueryKey, V>>,
}

impl<V: Clone> QueryCache<V> {
    pub fn get_or_insert_with(&self, key: QueryKey, f: impl FnOnce() -> V) -> V {
        if let Some(value) = self.values.lock().unwrap().get(&key).cloned() {
            return value;
        }

        let value = f();
        self.values.lock().unwrap().insert(key, value.clone());
        value
    }

    pub fn clear(&self) {
        self.values.lock().unwrap().clear();
    }
}

/// Tiny arena/interner for stable handles inside a process. This gives jfc a
/// rustc-inspired typed-handle substrate without introducing global lifetimes or
/// a dependency while the call sites are still small.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InternId(usize);

#[derive(Debug, Default)]
pub struct Interner<T> {
    values: Vec<T>,
}

impl<T: Eq> Interner<T> {
    pub fn intern(&mut self, value: T) -> InternId {
        if let Some(index) = self.values.iter().position(|existing| existing == &value) {
            return InternId(index);
        }
        let id = InternId(self.values.len());
        self.values.push(value);
        id
    }

    pub fn get(&self, id: InternId) -> Option<&T> {
        self.values.get(id.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_cache_reuses_stable_fingerprint_entry() {
        let cache = QueryCache::default();
        let key = QueryKey::ModelPickerModels(Fingerprint::new(("provider", "model")));

        let first = cache.get_or_insert_with(key.clone(), || vec!["computed".to_string()]);
        let second = cache.get_or_insert_with(key, || vec!["wrong".to_string()]);

        assert_eq!(first, vec!["computed"]);
        assert_eq!(second, vec!["computed"]);
    }

    #[test]
    fn interner_returns_stable_handles_for_equal_values() {
        let mut interner = Interner::default();
        let a = interner.intern("anthropic".to_string());
        let b = interner.intern("anthropic".to_string());

        assert_eq!(a, b);
        assert_eq!(interner.get(a).map(String::as_str), Some("anthropic"));
    }
}
