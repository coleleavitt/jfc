# ProcessBridge ABI

ProcessBridge is JFC's v1 external plugin ABI. It lets plugins run as normal
executables that exchange newline-delimited JSON frames with the host over
stdin/stdout. The contract lives in `jfc-plugin-sdk`; plugin authors should use
the SDK DTOs instead of hand-writing JSON structs.

This ABI is intentionally narrow:

- plugins do not receive `EngineState`, `App`, ratatui types, provider internals,
  or raw session stores
- the host owns discovery, policy, permissions, safe mode, snapshots, refresh
  cadence, and UI rendering
- plugin processes receive typed requests and return typed responses
- host-owned descriptor ids provide the stable join points

## Command Handler

Every process bridge handler is either:

```text
my-plugin-helper
```

or a serialized `ProcessBridgeCommand`:

```json
{"command":"cargo","args":["run","-p","my-plugin","--quiet"]}
```

The host starts that command with piped stdin, stdout, and stderr. Bridge frames
are one JSON object per line. Stderr is diagnostic text only; it is never parsed
as bridge data.

## Frame Envelope

All lanes use the same `BridgeEnvelope`.

```json
{"type":"request","id":"tool-123","request":{"kind":"tool_call","tool":"echo","input":{"text":"hi"}}}
```

```json
{"type":"response","id":"tool-123","response":{"kind":"tool_result","output":"hi","is_error":false}}
```

Rules:

- `type` is `request` or `response`
- `id` correlates a response to a request
- `request.kind` and `response.kind` select the lane payload
- response ids must match the triggering host request unless the plugin is
  sending a teammate host request
- unsupported request kinds should return `BridgeResponse::Error`

## Process Lifetimes

Different lanes use the same frames with different lifetimes.

| Lane | Lifetime | Host input | Plugin output |
| --- | --- | --- | --- |
| Descriptor discovery | one process per discovery call | one `describe` request | first non-empty `descriptors` response line |
| Tool call | one process per tool call | one `tool_call` request | first non-empty `tool_result` or `error` response line |
| Provider stream | one process per provider stream | one `provider_stream` request | many `provider_event` response lines until process exit |
| Foreground agent launch | one process per launch | one `agent_launch` request | first non-empty `agent_launch_result` or `error` response line |
| Teammate launch | persistent until terminal event/cancel | one `agent_launch` request, then host replies | many `teammate_event` responses and optional host requests |
| UI widget refresh | one process per refresh | one `ui_widget_refresh` request | first non-empty `ui_widget_refresh` or `error` response line |
| UI panel refresh | one process per refresh | one `ui_panel_refresh` request | first non-empty `ui_panel_refresh` or `error` response line |
| Prompt-context refresh | one process per refresh | one `prompt_context_refresh` request | first non-empty `prompt_context_refresh` or `error` response line |

## Descriptor Discovery

Project plugins can declare descriptors in `.jfc-plugin.toml`. Tool plugins may
also answer a bridge `describe` request.

```json
{"type":"request","id":"describe-tools","request":{"kind":"describe"}}
```

The response is either an array of `ToolDescriptor` values or an object with a
`tools` array.

```json
{"type":"response","id":"describe-tools","response":{"kind":"descriptors","descriptors":{"tools":[{"plugin_id":"placeholder","name":"external_echo","description":"Echo text through a plugin process.","input_schema":{"type":"object","properties":{"text":{"type":"string"}}},"executor":{"kind":"process_bridge","handler":""},"approval_policy":"read_only","visibility":"model_visible"}]}}}
```

During discovery the host normalizes the plugin id and fills an empty
process-bridge tool handler with the command that answered `describe`.

## Tool Calls

Model-visible process-bridge tools receive `tool_call`.

```json
{"type":"request","id":"tool-123","request":{"kind":"tool_call","tool":"external_echo","tool_id":"toolu_1","input":{"text":"hi"}}}
```

Return `tool_result`.

```json
{"type":"response","id":"tool-123","response":{"kind":"tool_result","output":"hi","is_error":false}}
```

If `is_error` is true, JFC treats `output` as a business/tool error. Optional
`payload` is preserved for structured callers, but plain `output` is the normal
surface.

## Providers

Process-bridge providers receive one `provider_stream` request containing
provider messages and stream options.

```json
{"type":"request","id":"provider-123","request":{"kind":"provider_stream","provider":"local-ai","messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}],"options":{"model":"local-chat","max_tokens":128}}}
```

They stream `provider_event` responses on stdout.

```json
{"type":"response","id":"provider-123","response":{"kind":"provider_event","event":{"type":"text_delta","index":0,"delta":"hi"}}}
```

```json
{"type":"response","id":"provider-123","response":{"kind":"provider_event","event":{"type":"done","stop_reason":{"type":"end_turn"}}}}
```

Common event types include `text_delta`, `text_done`, `thinking_delta`,
`thinking_done`, `tool_delta`, `tool_done`, `usage`, `response_metadata`,
`keepalive`, `fallback_triggered`, `error`, and `done`.

## Agent Launchers

Foreground process-bridge launchers receive `agent_launch`.

```json
{"type":"request","id":"agent-123","request":{"kind":"agent_launch","launch":{"launcher":"variant-agent","task":{"description":"inspect code","prompt":"find the sharp edges","run_in_background":false,"allowed_tools":[],"disallowed_tools":[]},"cwd":"/workspace/project","model":"local-model","provider":"plugin-provider"}}}
```

Return `agent_launch_result`.

```json
{"type":"response","id":"agent-123","response":{"kind":"agent_launch_result","result":{"output":"agent finished","is_error":false}}}
```

For one-shot launches, the first response line is the result. Background task
launchers may execute through the same request shape inside the detached worker.

## Teammates

Teammate launchers start with the same `agent_launch` request, but the process
stays alive. The plugin can emit teammate events:

```json
{"type":"response","id":"teammate-123","response":{"kind":"teammate_event","event":{"kind":"text_delta","delta":"reading files"}}}
```

```json
{"type":"response","id":"teammate-123","response":{"kind":"teammate_event","event":{"kind":"completed"}}}
```

Terminal teammate events are `completed`, `cancelled`, and `failed`.

Persistent teammate processes can also ask the host for mailbox and ready/idle
operations by writing request frames with their own ids:

```json
{"type":"request","id":"mailbox-1","request":{"kind":"teammate_mailbox_poll","request":{"unread_only":true,"mark_read":true}}}
```

The host writes the matching response back to the teammate process:

```json
{"type":"response","id":"mailbox-1","response":{"kind":"teammate_mailbox_messages","messages":[]}}
```

Supported teammate host requests are `teammate_mailbox_poll`,
`teammate_mailbox_send`, and `teammate_ready`.

## UI Refresh

Widgets and panels are rendered by the host from descriptors. ProcessBridge only
refreshes their body/state snapshots.

Widget request:

```json
{"type":"request","id":"ui-widget-123","request":{"kind":"ui_widget_refresh","refresh":{"widget_id":"diagnostics.counter","scope":"info_sidebar","state":{"count":4}}}}
```

Widget response:

```json
{"type":"response","id":"ui-widget-123","response":{"kind":"ui_widget_refresh","result":{"body":"refresh #5","state":{"count":5}}}}
```

Panel refresh uses the same shape with `ui_panel_refresh`, `panel_id`, and
`BridgeUiPanelRefreshResult`.

The host owns `min_interval_ms`, `auto_refresh_ms`, persisted snapshots, focus,
and rendering. Plugins return body text plus optional JSON state.

## Prompt Context

Prompt-context extensions are runtime extensions with
`target = "prompt_context"` and `executor.kind = "process_bridge"`.

Request:

```json
{"type":"request","id":"prompt-context-123","request":{"kind":"prompt_context_refresh","refresh":{"extension_id":"context.cached-note","cwd":"/workspace/project","max_chars":12000,"state":{"count":4}}}}
```

Response:

```json
{"type":"response","id":"prompt-context-123","response":{"kind":"prompt_context_refresh","result":{"body":"Cached prompt context refresh #5","state":{"count":5}}}}
```

The host applies body length caps, owns refresh cadence, saves snapshots, and
passes the previous returned state back on refresh. Bridge failures warn and
fail soft so prompt assembly can continue.

## Errors

Any lane can return a structured bridge error.

```json
{"type":"response","id":"tool-123","response":{"kind":"error","code":"invalid_input","message":"missing text"}}
```

Prefer stable `code` strings that plugin authors can document and test. Put
human-readable details in `message`.

## Compatibility Rules

- Depend on `jfc-plugin-sdk` DTOs and serde names as the wire source of truth.
- Treat unknown request kinds as recoverable errors, not panics.
- Keep stdout reserved for JSONL frames; write logs to stderr.
- Return exactly one first non-empty response line for one-shot lanes.
- For streaming lanes, keep every stdout line a valid bridge response frame.
- Match host request ids on responses.
- Do not assume raw JFC paths, TUI state, session databases, or provider
  internals are stable.
- Expect the ABI to grow by adding optional fields and new enum variants.

