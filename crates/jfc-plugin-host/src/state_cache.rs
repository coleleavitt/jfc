use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use crate::{
    DiscoveredPluginReload, PluginDiscoveryOptions, PluginHost, PluginHostError,
    PluginReloadReport, reload_discovered_resource_plugin_host,
};

#[derive(Clone)]
pub struct CachedDiscoveredPluginState {
    pub host: Arc<PluginHost>,
    pub report: PluginReloadReport,
}

fn cache() -> &'static RwLock<HashMap<String, Arc<CachedDiscoveredPluginState>>> {
    static CACHE: OnceLock<RwLock<HashMap<String, Arc<CachedDiscoveredPluginState>>>> =
        OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

pub fn cached_discovered_resource_plugin_state(
    options: PluginDiscoveryOptions,
) -> Result<Arc<CachedDiscoveredPluginState>, PluginHostError> {
    let key = options.cache_key();
    if let Some(state) = cache()
        .read()
        .map(|cache| cache.get(&key).cloned())
        .unwrap_or_default()
    {
        return Ok(state);
    }

    let state = build_cached_state(options, None)?;
    if let Ok(mut cache) = cache().write() {
        Ok(cache.entry(key).or_insert_with(|| state.clone()).clone())
    } else {
        Ok(state)
    }
}

pub fn reload_cached_discovered_resource_plugin_state(
    options: PluginDiscoveryOptions,
    previous_digest: Option<&str>,
) -> Result<Arc<CachedDiscoveredPluginState>, PluginHostError> {
    let key = options.cache_key();
    let state = build_cached_state(options, previous_digest)?;
    if let Ok(mut cache) = cache().write() {
        cache.insert(key, state.clone());
    }
    Ok(state)
}

#[doc(hidden)]
pub fn clear_discovered_plugin_state_cache_for_tests() {
    if let Ok(mut cache) = cache().write() {
        cache.clear();
    }
}

fn build_cached_state(
    options: PluginDiscoveryOptions,
    previous_digest: Option<&str>,
) -> Result<Arc<CachedDiscoveredPluginState>, PluginHostError> {
    let DiscoveredPluginReload { host, report } =
        reload_discovered_resource_plugin_host(options, previous_digest)?;
    Ok(Arc::new(CachedDiscoveredPluginState {
        host: Arc::new(host),
        report,
    }))
}
