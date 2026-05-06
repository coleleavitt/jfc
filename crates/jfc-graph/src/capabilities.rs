//! Modular capability tree for enabling/disabling graph features.

use std::collections::HashMap;

/// Individual capabilities that can be toggled on/off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    CallGraph,
    TypeUsage,
    PartialStruct,
    VirtualValidation,
    Persistence,
    SymbolEditing,
}

/// All known capabilities for iteration.
const ALL_CAPABILITIES: [Capability; 6] = [
    Capability::CallGraph,
    Capability::TypeUsage,
    Capability::PartialStruct,
    Capability::VirtualValidation,
    Capability::Persistence,
    Capability::SymbolEditing,
];

/// Capability tree with dependency cascading.
///
/// When a capability is disabled, any capability that depends on it
/// is also disabled (cascading).
pub struct CapabilityTree {
    enabled: HashMap<Capability, bool>,
    dependencies: HashMap<Capability, Vec<Capability>>,
}

impl CapabilityTree {
    pub fn new() -> Self {
        let mut tree = Self {
            enabled: HashMap::new(),
            dependencies: HashMap::new(),
        };

        // All enabled by default
        for cap in ALL_CAPABILITIES {
            tree.enabled.insert(cap, true);
        }

        // Dependencies: VirtualValidation requires CallGraph
        tree.dependencies
            .insert(Capability::VirtualValidation, vec![Capability::CallGraph]);
        // Dependencies: PartialStruct requires TypeUsage
        tree.dependencies
            .insert(Capability::PartialStruct, vec![Capability::TypeUsage]);

        tree
    }

    /// Check if a capability is currently enabled.
    pub fn is_enabled(&self, cap: Capability) -> bool {
        *self.enabled.get(&cap).unwrap_or(&false)
    }

    /// Disable a capability. Cascades to dependents (capabilities that require this one).
    /// Returns the list of additionally disabled capabilities.
    pub fn disable(&mut self, cap: Capability) -> Vec<Capability> {
        self.enabled.insert(cap, false);

        // Find and disable dependents
        let mut cascaded = Vec::new();
        let all_caps: Vec<Capability> = self.enabled.keys().copied().collect();
        for other_cap in all_caps {
            if let Some(deps) = self.dependencies.get(&other_cap) {
                if deps.contains(&cap) && self.is_enabled(other_cap) {
                    self.enabled.insert(other_cap, false);
                    cascaded.push(other_cap);
                }
            }
        }
        cascaded
    }

    /// Enable a capability.
    pub fn enable(&mut self, cap: Capability) {
        self.enabled.insert(cap, true);
    }
}

impl Default for CapabilityTree {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capability_all_default() {
        let tree = CapabilityTree::new();
        for cap in ALL_CAPABILITIES {
            assert!(tree.is_enabled(cap), "{cap:?} should be enabled by default");
        }
    }

    #[test]
    fn test_capability_disable_cascades() {
        let mut tree = CapabilityTree::new();

        // Disable CallGraph → VirtualValidation should also be disabled
        let cascaded = tree.disable(Capability::CallGraph);
        assert!(!tree.is_enabled(Capability::CallGraph));
        assert!(!tree.is_enabled(Capability::VirtualValidation));
        assert!(cascaded.contains(&Capability::VirtualValidation));

        // TypeUsage still enabled
        assert!(tree.is_enabled(Capability::TypeUsage));
    }

    #[test]
    fn test_capability_enable() {
        let mut tree = CapabilityTree::new();
        tree.disable(Capability::Persistence);
        assert!(!tree.is_enabled(Capability::Persistence));

        tree.enable(Capability::Persistence);
        assert!(tree.is_enabled(Capability::Persistence));
    }
}
