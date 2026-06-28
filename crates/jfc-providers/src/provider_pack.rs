use jfc_plugin_sdk::{PluginId, ProviderDescriptor, ProviderExecutorKind, ProviderModelDescriptor};
use jfc_provider::ModelInfo;

pub const BUILTIN_PROVIDER_PACK_ID: &str = "builtin.providers";

pub fn first_party_provider_descriptors() -> Vec<ProviderDescriptor> {
    vec![
        anthropic_provider_descriptor(),
        openai_provider_descriptor(),
        openrouter_provider_descriptor(),
        litellm_provider_descriptor(),
    ]
}

fn anthropic_provider_descriptor() -> ProviderDescriptor {
    let mut descriptor =
        ProviderDescriptor::new(PluginId::new(BUILTIN_PROVIDER_PACK_ID), "anthropic")
            .with_executor(ProviderExecutorKind::BuiltIn, "anthropic");
    descriptor.models = model_descriptors(super::anthropic_models::anthropic_first_party_models(
        "anthropic",
    ));
    descriptor
}

fn openai_provider_descriptor() -> ProviderDescriptor {
    let mut descriptor = ProviderDescriptor::new(PluginId::new(BUILTIN_PROVIDER_PACK_ID), "openai")
        .with_executor(ProviderExecutorKind::BuiltIn, "openai");
    descriptor.models = model_descriptors(super::openai::OpenAIProvider::fallback_models());
    descriptor
}

fn openrouter_provider_descriptor() -> ProviderDescriptor {
    ProviderDescriptor::new(PluginId::new(BUILTIN_PROVIDER_PACK_ID), "openrouter")
        .with_executor(ProviderExecutorKind::BuiltIn, "openrouter")
}

fn litellm_provider_descriptor() -> ProviderDescriptor {
    ProviderDescriptor::new(PluginId::new(BUILTIN_PROVIDER_PACK_ID), "litellm")
        .with_executor(ProviderExecutorKind::BuiltIn, "litellm")
}

fn model_descriptors(models: Vec<ModelInfo>) -> Vec<ProviderModelDescriptor> {
    models
        .into_iter()
        .map(|model| {
            ProviderModelDescriptor::new(model.id.as_str().to_owned())
                .with_display_name(model.display_name)
                .with_context_window_tokens(model.context_window_tokens)
                .with_max_output_tokens(model.max_output_tokens)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_party_provider_descriptors_list_anthropic_catalog_normal() {
        let descriptors = first_party_provider_descriptors();

        let descriptor = descriptors
            .iter()
            .find(|descriptor| descriptor.provider == "anthropic")
            .expect("anthropic descriptor");
        assert_eq!(descriptor.plugin_id.as_str(), BUILTIN_PROVIDER_PACK_ID);
        assert!(
            descriptor
                .models
                .iter()
                .any(|model| model.id == "claude-opus-4-8")
        );
    }

    #[test]
    fn first_party_provider_descriptors_list_openai_compatible_family_normal() {
        let descriptors = first_party_provider_descriptors();
        let providers = descriptors
            .iter()
            .map(|descriptor| descriptor.provider.as_str())
            .collect::<Vec<_>>();

        assert!(providers.contains(&"openai"));
        assert!(providers.contains(&"openrouter"));
        assert!(providers.contains(&"litellm"));

        let openai = descriptors
            .iter()
            .find(|descriptor| descriptor.provider == "openai")
            .expect("openai descriptor");
        assert!(openai.models.iter().any(|model| model.id == "gpt-5.1"));
    }
}
