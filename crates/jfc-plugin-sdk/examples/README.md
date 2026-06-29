# JFC Plugin SDK Examples

See `../PROCESS_BRIDGE.md` for the shared JSONL ABI used by all process-bridge
examples.

## `teammate_helper_agent.rs`

This example is a minimal process-bridge teammate launcher. It shows the external-agent pattern JFC expects:

1. Read the initial `agent_launch` frame from stdin and keep its request id.
2. Send child-originated host requests for mailbox polling, mailbox sending, and ready/idle state.
3. Read host responses from stdin.
4. Emit normal `teammate_event` responses on stdout using the initial launch id.

Example plugin manifest:

```toml
[plugin]
name = "example-teammate-plugin"

[[agent_launches]]
name = "helper-agent"
label = "Helper Agent"
description = "Mailbox-aware process-bridge teammate."

[agent_launches.executor]
kind = "process_bridge"
handler = '{"command":"cargo","args":["run","-p","jfc-plugin-sdk","--example","teammate_helper_agent","--quiet"]}'
```

The runtime fills missing mailbox/ready agent and team fields from the teammate task, so simple plugins can use `BridgeMailboxPollRequest::unread()` and `BridgeTeammateReady::new()` without knowing JFC's swarm storage layout.

## `prompt_context_provider.rs`

This example is a minimal process-bridge prompt-context contributor. It reads a `prompt_context_refresh` frame from stdin, returns prompt context text, and persists a small refresh counter in host-owned prompt-context snapshot state.

Example plugin manifest:

```toml
[plugin]
name = "example-prompt-context-plugin"

[[runtime_extensions]]
target = "prompt_context"
id = "context.cached-note"
label = "Cached Note"
priority = 60
refresh = { kind = "process_bridge", min_interval_ms = 1000, auto_refresh_ms = 60000 }

[runtime_extensions.executor]
kind = "process_bridge"
handler = '{"command":"cargo","args":["run","-p","jfc-plugin-sdk","--example","prompt_context_provider","--quiet"]}'
```

JFC owns the refresh cadence and persisted snapshot file. The plugin only receives the previous `state` value and returns the next body/state pair.

## `process_bridge_tool.rs`

This example is a minimal process-bridge tool. It reads a `tool_call` frame from stdin and returns a `tool_result` frame that JFC can expose as a model-visible tool descriptor.

Example plugin manifest:

```toml
[plugin]
name = "example-process-tool-plugin"

[[tools]]
name = "external_echo"
description = "External Echo"
visibility = "model_visible"
approval_policy = "read_only"
input_schema = { type = "object", properties = { message = { type = "string", description = "Message to echo." } }, required = ["message"], additionalProperties = false }

[tools.executor]
kind = "process_bridge"
handler = '{"command":"cargo","args":["run","-p","jfc-plugin-sdk","--example","process_bridge_tool","--quiet"]}'
```

JFC owns descriptor discovery, visibility, and approval policy. The plugin only handles the bridge request and response.

## `process_bridge_provider.rs`

This example is a minimal process-bridge provider. It reads a `provider_stream` frame from stdin and emits provider stream events as JSONL response frames.

Example plugin manifest:

```toml
[plugin]
name = "example-process-provider-plugin"

[[providers]]
provider = "external-demo"
visibility = "host_visible"
models = [{ id = "external-demo-chat", display_name = "External Demo Chat", context_window_tokens = 8192, max_output_tokens = 1024 }]

[providers.executor]
kind = "process_bridge"
handler = '{"command":"cargo","args":["run","-p","jfc-plugin-sdk","--example","process_bridge_provider","--quiet"]}'
```

JFC owns provider registration and model advertising. The plugin only translates stream requests into `provider_event` frames.

## `ui_diagnostics_panel.rs`

This example is a minimal process-bridge refresh handler for plugin-owned UI. It reads a `ui_widget_refresh` frame from stdin, returns a widget body, and persists a small refresh counter in the host-owned widget snapshot state.

Example plugin manifest:

```toml
[plugin]
name = "example-ui-diagnostics-plugin"

[[runtime_actions]]
id = "diagnostics.refresh"
label = "Refresh Diagnostics"
description = "Refresh plugin diagnostics descriptors."
kind = "refresh_metrics"
priority = 20

[[ui_panels]]
scope = "info_sidebar"
id = "diagnostics.summary"
title = "Diagnostics Summary"
body = "Refreshable widget and panel descriptors are active."
runtime_action_id = "diagnostics.refresh"
priority = 50

[[ui_widgets]]
scope = "info_sidebar"
id = "diagnostics.counter"
label = "Refresh Counter"
kind = "text"
body = "not refreshed yet"
runtime_action_id = "diagnostics.refresh"
refresh = { kind = "process_bridge", handler = '{"command":"cargo","args":["run","-p","jfc-plugin-sdk","--example","ui_diagnostics_panel","--quiet"]}', min_interval_ms = 1000, auto_refresh_ms = 60000 }
priority = 40
```
