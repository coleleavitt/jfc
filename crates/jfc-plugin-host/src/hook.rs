use std::sync::Arc;

use jfc_core::SessionId;
use jfc_plugin_sdk::{HookName, PluginId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::PluginHostError;

pub type HookCallback =
    Arc<dyn for<'a> Fn(HookInvocation<'a>) -> Result<HookValue, PluginHostError> + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct HookValue {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_id: Option<SessionId>,
    payload: Value,
}

impl HookValue {
    pub fn new(session_id: Option<SessionId>, payload: Value) -> Self {
        Self {
            session_id,
            payload,
        }
    }

    pub fn json(payload: Value) -> Self {
        Self::new(None, payload)
    }

    pub fn payload(&self) -> &Value {
        &self.payload
    }

    pub fn session_id(&self) -> Option<&SessionId> {
        self.session_id.as_ref()
    }

    pub fn into_payload(self) -> Value {
        self.payload
    }
}

#[derive(Clone)]
pub struct HookInvocation<'a> {
    plugin_id: &'a PluginId,
    name: HookName,
    value: &'a HookValue,
}

impl<'a> HookInvocation<'a> {
    pub(crate) fn new(plugin_id: &'a PluginId, name: HookName, value: &'a HookValue) -> Self {
        Self {
            plugin_id,
            name,
            value,
        }
    }

    pub fn plugin_id(&self) -> &PluginId {
        self.plugin_id
    }

    pub fn name(&self) -> HookName {
        self.name
    }

    pub fn value(&self) -> &HookValue {
        self.value
    }
}

#[derive(Clone)]
pub(crate) struct HookDefinition {
    pub name: HookName,
    pub priority: i32,
    pub callback: HookCallback,
}

impl HookDefinition {
    pub fn new(name: HookName, priority: i32, callback: HookCallback) -> Self {
        Self {
            name,
            priority,
            callback,
        }
    }
}

#[derive(Clone)]
pub(crate) struct ActivatedHook {
    pub plugin_id: PluginId,
    pub name: HookName,
    pub priority: i32,
    pub activation_order: i32,
    pub activation_sequence: u64,
    pub hook_sequence: u64,
    pub callback: HookCallback,
}
