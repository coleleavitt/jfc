use std::{path::Path, sync::Arc};

use jfc_plugin_host::cached_discovered_resource_plugin_state;
use jfc_plugin_sdk::{
    DescriptorVisibility, ProviderDescriptor, ProviderExecutorDescriptor, ProviderExecutorKind,
};
use jfc_provider::{
    EventStream, ModelId, ModelInfo, Provider, ProviderId, ProviderMessage, StreamOptions,
};

use crate::workflows::registry::plugin_discovery_options_for;

#[derive(Debug, Clone)]
pub struct DescriptorProvider {
    name: String,
    models: Vec<ModelInfo>,
    executor: ProviderExecutorDescriptor,
}

impl DescriptorProvider {
    pub fn from_descriptor(descriptor: ProviderDescriptor) -> Option<Self> {
        if descriptor.visibility == DescriptorVisibility::Internal {
            return None;
        }
        if descriptor.provider.trim().is_empty() {
            return None;
        }
        let provider_name = descriptor.provider;
        let models = descriptor
            .models
            .into_iter()
            .map(|model| {
                ModelInfo::new(
                    ModelId::new(model.id),
                    model.display_name,
                    ProviderId::new(provider_name.clone()),
                )
                .with_context_window_tokens(model.context_window_tokens)
                .with_max_output_tokens(model.max_output_tokens)
            })
            .collect();
        Some(Self {
            name: provider_name,
            models,
            executor: descriptor.executor,
        })
    }
}

impl jfc_provider::seal::Sealed for DescriptorProvider {}

#[async_trait::async_trait]
impl Provider for DescriptorProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn available_models(&self) -> Vec<ModelInfo> {
        self.models.clone()
    }

    async fn stream(
        &self,
        messages: Vec<ProviderMessage>,
        options: &StreamOptions,
    ) -> anyhow::Result<EventStream> {
        match self.executor.kind {
            ProviderExecutorKind::ProcessBridge => {
                super::provider_process_bridge::stream(
                    super::provider_process_bridge::ProviderBridgeInvocation {
                        provider_name: &self.name,
                        executor: &self.executor,
                        messages,
                        options,
                    },
                )
                .await
            }
            ProviderExecutorKind::BuiltIn => anyhow::bail!(
                "plugin provider `{}` cannot use the built-in provider executor",
                self.name
            ),
        }
    }

    async fn fetch_models(&self) -> anyhow::Result<Vec<ModelInfo>> {
        Ok(self.available_models())
    }

    async fn complete(
        &self,
        _messages: Vec<ProviderMessage>,
        _options: &StreamOptions,
    ) -> anyhow::Result<jfc_provider::CompletionResponse> {
        anyhow::bail!(
            "plugin provider `{}` does not support non-streaming completion yet",
            self.name
        )
    }
}

pub fn append_descriptor_providers<I>(
    providers: &mut Vec<Arc<dyn Provider>>,
    descriptors: I,
) -> usize
where
    I: IntoIterator<Item = ProviderDescriptor>,
{
    let mut added = 0;
    for descriptor in descriptors {
        let Some(provider) = DescriptorProvider::from_descriptor(descriptor) else {
            continue;
        };
        if providers
            .iter()
            .any(|existing| existing.name() == provider.name())
        {
            continue;
        }
        providers.push(Arc::new(provider));
        added += 1;
    }
    added
}

pub fn append_discovered_provider_plugins(
    providers: &mut Vec<Arc<dyn Provider>>,
    project_root: &Path,
) -> usize {
    let options = plugin_discovery_options_for(project_root);
    let state = match cached_discovered_resource_plugin_state(options) {
        Ok(state) => state,
        Err(error) => {
            tracing::warn!(
                target: "jfc::plugin_host",
                error = %error,
                "failed to activate provider descriptor plugins"
            );
            return 0;
        }
    };
    append_descriptor_providers(providers, state.host.provider_descriptors())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use jfc_plugin_sdk::{PluginId, ProviderDescriptor, ProviderExecutorKind};
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn descriptor_provider_advertises_declared_models_normal() {
        let descriptor = ProviderDescriptor::new(PluginId::new("plugin.local"), "local-ai")
            .with_executor(ProviderExecutorKind::ProcessBridge, "provider.sh")
            .with_model_info("local-chat", "Local Chat", Some(32_000), Some(4_096));

        let provider = DescriptorProvider::from_descriptor(descriptor).expect("provider");
        let models = provider.available_models();

        assert_eq!(provider.name(), "local-ai");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id.as_str(), "local-chat");
        assert_eq!(models[0].provider.as_str(), "local-ai");
        assert_eq!(models[0].context_window_tokens, Some(32_000));
        assert_eq!(models[0].max_output_tokens, Some(4_096));
    }

    #[tokio::test]
    async fn descriptor_provider_stream_reports_invalid_bridge_command_robust() {
        let descriptor = ProviderDescriptor::new(PluginId::new("plugin.local"), "local-ai")
            .with_executor(ProviderExecutorKind::ProcessBridge, "provider.sh")
            .with_model("local-chat");
        let provider = DescriptorProvider::from_descriptor(descriptor).expect("provider");

        let err = match provider
            .stream(Vec::new(), &StreamOptions::new("local-chat"))
            .await
        {
            Ok(_) => panic!("streaming is not implemented yet"),
            Err(error) => error,
        };

        assert!(err.to_string().contains("could not start"));
    }

    #[tokio::test]
    async fn descriptor_provider_streams_process_bridge_events_normal() {
        // Given: a plugin provider descriptor backed by an executable bridge.
        let tempdir = tempfile::tempdir().expect("tempdir");
        let script = tempdir.path().join("provider.sh");
        let mut file = std::fs::File::create(&script).expect("script create");
        writeln!(
            file,
            r#"#!/bin/sh
read line
id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
case "$line" in
  *provider_stream*local-ai*local-chat*)
    printf '%s\n' "{{\"type\":\"response\",\"id\":\"$id\",\"response\":{{\"kind\":\"provider_event\",\"event\":{{\"type\":\"text_delta\",\"index\":0,\"delta\":\"bridge hi\"}}}}}}"
    printf '%s\n' "{{\"type\":\"response\",\"id\":\"$id\",\"response\":{{\"kind\":\"provider_event\",\"event\":{{\"type\":\"done\",\"stop_reason\":{{\"type\":\"end_turn\"}}}}}}}}"
    ;;
  *)
    printf '%s\n' "{{\"type\":\"response\",\"id\":\"$id\",\"response\":{{\"kind\":\"provider_event\",\"event\":{{\"type\":\"error\",\"message\":\"bad request\"}}}}}}"
    ;;
esac"#
        )
        .expect("script write");
        drop(file);
        let mut permissions = std::fs::metadata(&script)
            .expect("script metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script, permissions).expect("script permissions");

        let descriptor = ProviderDescriptor::new(PluginId::new("plugin.local"), "local-ai")
            .with_executor(
                ProviderExecutorKind::ProcessBridge,
                script.to_string_lossy().into_owned(),
            )
            .with_model("local-chat");
        let provider = DescriptorProvider::from_descriptor(descriptor).expect("provider");

        // When: the provider stream is consumed through the normal Provider trait.
        let messages = vec![ProviderMessage {
            role: jfc_provider::ProviderRole::User,
            content: vec![jfc_provider::ProviderContent::Text("hello".to_owned())],
        }];
        let mut stream = provider
            .stream(messages, &StreamOptions::new("local-chat"))
            .await
            .expect("stream starts");
        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            events.push(event.expect("stream event"));
        }

        // Then: bridge events are exposed as provider stream events.
        assert!(matches!(
            events.first(),
            Some(jfc_provider::StreamEvent::TextDelta { delta, .. }) if delta == "bridge hi"
        ));
        assert!(matches!(
            events.last(),
            Some(jfc_provider::StreamEvent::Done {
                stop_reason: jfc_provider::StopReason::EndTurn
            })
        ));
    }

    #[test]
    fn append_descriptor_providers_skips_existing_provider_names_normal() {
        let mut providers: Vec<Arc<dyn Provider>> = vec![Arc::new(
            DescriptorProvider::from_descriptor(
                ProviderDescriptor::new(PluginId::new("plugin.first"), "local-ai")
                    .with_model("first-model"),
            )
            .expect("provider"),
        )];
        let added = append_descriptor_providers(
            &mut providers,
            [
                ProviderDescriptor::new(PluginId::new("plugin.second"), "local-ai")
                    .with_model("second-model"),
            ],
        );

        assert_eq!(added, 0);
        assert_eq!(providers.len(), 1);
    }
}
