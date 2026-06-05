# JFC Streaming System: Tool Calls & Event Handling

## Overview

JFC implements a real-time streaming architecture using Server-Sent Events (SSE) to deliver Claude API responses incrementally. Tool calls are discovered during streaming, queued, executed asynchronously, and their results are fed back for agentic looping.

---

## Architecture: Streaming Flow

### 1. Stream Setup Phase

```
User Input
    ↓
[stream_response spawned]
    ↓
[System prompt built with skills, agents, memory, diagnostics]
    ↓
[Provider sends POST to Anthropic API with stream=true]
    ↓
[HTTP 200 received, SSE connection opened]
    ↓
[Tracing span: `stream{model=..., messages=N, tools=M}`]
```

**Key Entry**: `jfc::stream::stream_response()` in `crates/jfc-ui/src/stream/mod.rs`

**System Prompt Composition** (`crates/jfc-ui/src/stream/mod.rs`):
- Skills index (loaded from `.claude/skills/`)
- Agent catalog (Explore, Plan, general-purpose, orchestrator, verification)
- CLAUDE.md hierarchy (project conventions)
- Memory recall block (user preferences, project knowledge)
- Output style directive (brief, detailed, etc.)
- Advisor prompt (local decision guidance)
- Estimated token budget

**Tool Registration**: 68 tools advertised to Claude:
- Filesystem: `Read`, `Write`, `Edit`, `Glob`, `Grep`
- Code graph: `graph_search`, `graph_context`, `graph_callers`, `graph_callees`, `graph_impact`, `graph_node`, `graph_explore`, `graph_outline`, `graph_grep`, `graph_query`, `graph_status`, `graph_files`, `code_index`, `symbol_edit`, `run_coverage`
- Task management: `TaskCreate`, `TaskUpdate`, `TaskList`, `TaskDone`, `TaskStop`, `TaskGet`, `TaskValidate`
- Agents: `Task`, `Workflow`, `Skill`
- Planning: `PlanCreate`, `PlanList`, `PlanShow`, `PlanAdvance`, `PlanArchive`, `PlanMaterialize`
- Teams/messaging: `TeamCreate`, `TeamDelete`, `TeamMemberMode`, `SendMessage`
- Shell/tools: `Bash`, `ApplyPatch`
- Queries: `WebSearch`, `WebFetch`, `ToolSearch`, `ToolSuggest`
- Utility: `Memory*`, `Notebook*`, `Scratchpad*`, `Lsp`, `PushNotification`, `RemoteTrigger`, `Monitor`, `Cron*`, `ScheduleWakeup`, `Advisor`
- UI/Output: `AskUserQuestion`, `SendUserFile`, `StructuredOutput`, `ExitPlanMode`
- MCP: `WaitForMcpServers`, `ListMcpResources`, `ReadMcpResource`
- Other: `EnterPlanMode`, `EnterWorktree`, `ExitWorktree`, `PostBounty`, `RunBounty`, `MarketStatus`

---

### 2. SSE Event Reception & Parsing

**Provider**: `crates/jfc-ui/src/provider/anthropic_sse.rs`

Events received:
- `message_start` → message_id, model metadata
- `content_block_start` → index, type (text/tool_use)
- `content_block_delta` → incremental text chunks
- `content_block_stop` → end of content block
- `message_delta` → usage metadata, stop_reason
- `message_stop` → finalization

**Tracing** (from `latest.log`):
```
first SSE payload received latency_ms=0 event=message_start bytes_seen=480 events_seen=1
content_block_start tool_use index=1 tool_name=Read tool_use_id=toolu_01...
content_block_delta delta="<param_chunk>"
content_block_stop index=1
content_block_start tool_use index=2 tool_name=Bash tool_use_id=toolu_02...
message_delta stop_reason=Some("tool_use") input_tokens=3 output_tokens=191 ...
```

---

### 3. Tool Call Discovery

**Location**: `crates/jfc-ui/src/stream/live_events.rs`

As tool calls arrive during streaming:

1. **Tool buffered**: `StreamEvent::ToolStart { id, kind, index }`
2. **Parameters accumulated**: Delta events with JSON parameter chunks
3. **Tool complete**: `StreamEvent::ToolDone { id, kind, index, input }`

**Example from logs**:
```
tool_done index=1 tool_name=graph_grep tool_use_id=toolu_01Ra5BdRbVGBLkU8rrZ82eGM input_len=41
tool_done index=2 tool_name=Grep tool_use_id=toolu_01P7oByC3v1jwdDAq8pT4S6i input_len=114
```

Tool calls are **not executed immediately**. Instead they are queued pending approval/deferred dispatch.

---

### 4. Tool Validation & Approval Gate

**Location**: `crates/jfc-ui/src/ui/tool.rs`

When a tool arrives during streaming:

```rust
match (streaming_tool_exec, needs_approval) {
    (true, false) => route=eager_dispatch,
    (false, false) => route=deferred_dispatch,
    (_, true) => route=pending_approval,
}
```

**Config**: `streaming-tool-exec` flag. If **OFF** (default), all tools defer.

**Logged decision**:
```
StreamTool received tool_kind="GraphGrep" tool_id=toolu_01Ra5BdRbVGBLkU8rrZ82eGM 
auto_mode=false needs_approval=false streaming_idx=Some(1)
route=deferred_dispatch (streaming-tool-exec OFF, no approval needed) 
pending_total=2
```

---

### 5. Stream Completion & Tool Dispatch

**Location**: `crates/jfc-ui/src/stream/mod.rs` (`stream_done()`)

When message_stop or tool_use stop_reason received:

```
StreamEvent::Done received stop_reason=ToolUse pending_tool_count=2
stream_done dispatching ordered pending tool batch n=2 kinds=["GraphGrep", "Grep"]
```

**Dispatch logic**:

1. Tools batched into sequential + parallel groups via scheduler
2. Parallel batch executes concurrently (up to N workers)
3. Sequential tools execute one at a time
4. Results collected as `ToolOutput`

**Scheduler split** (from logs):
```
dispatch_tools_batched: splitting tool calls task_count=0 workflow_count=0 
advisor_count=0 regular_count=2
scheduled tool calls into batches total_calls=2 batch_count=2 parallel_count=1 sequential_count=1
```

---

### 6. Tool Execution

**Location**: `crates/jfc-ui/src/tools/dispatch.rs` (`execute_tool()`)

```rust
pub async fn execute_tool(
    executor: &Executor,
    app: &mut AppState,
    call: ToolCall,
    streaming_assistant_idx: Option<usize>,
) -> Result<ToolOutput, ToolError>
```

**Steps**:

1. **Parse**: `ToolInput::from_value(tool_name, params_json)` validates parameters
2. **Dispatch**: Match on `ToolKind` and call appropriate handler
3. **Execute**: Tool-specific logic (Bash runs shell, Read loads file, etc.)
4. **Collect**: Output, elapsed_ms, errors
5. **Save**: Session updated with tool result

**Tool Kind Resolution** (`crates/jfc-core/src/tool_kind.rs`):

```rust
pub fn from_name(name: &str) -> Self {
    return_tool_kind!(name,
        Self::Edit => ["edit", "str_replace_based_edit_tool"],
        Self::Read => ["read", "read_file"],
        Self::Bash => ["bash", "run_bash"],
        Self::TaskCreate => ["task_create"],
        Self::TaskUpdate => ["task_update"],
        // ... 68 variants
    );
    Self::UnknownTool { advertised_name: name }
}
```

**Name matching** (`tool_name_eq()`):
- Case-insensitive
- Underscore-insensitive
- **Works correctly**: `taskcreate` ≈ `TaskCreate` ≈ `task_create` ≈ `TASK_CREATE`

---

## Tool Parameters: Parsing & Validation

### ToolInput Macro-Driven Parsing

**Design** (`crates/jfc-core/src/tool_input.rs`):

One declarative macro defines:
1. Enum variant definition
2. `from_value()` parse arm
3. `to_value()` serialize arm

Keeps all three synchronized.

**Example fields** (Read tool):
```rust
Read => { 
    file_path: req_str @ "file_path", 
    offset: opt_u64 @ "offset", 
    limit: opt_u64 @ "limit" 
}
```

Maps to `ToolInput::Read { file_path, offset, limit }`.

### Field Rules

| Rule | Behavior |
|------|----------|
| `req_str` | Required string, error if missing |
| `opt_str` | Optional string, None if absent |
| `opt_u64` | Optional 64-bit unsigned int |
| `opt_u64_as_usize` | Optional u64 cast to usize |
| `str_vec` | Optional string array |
| `str_vec_alias` | String array with fallback alias (see below) |
| `raw_bool_opt` | Optional boolean |
| `bool_field` | Boolean, default false |
| `replacement` | Parse `replace_all: bool` → `ReplacementMode` |
| `raw_opt` | Optional raw JSON value |

### Parameter Alias Support

**FULLY SUPPORTED** (hand-written):

1. **TaskCreate**:
   - `blocked_by` ← primary field
   - `depends_on` ← alias
   
   ```rust
   let blocked_by = obj
       .and_then(|map| map.get("blocked_by").or_else(|| map.get("depends_on")))
       .and_then(|value| value.as_array())
       // ...
   ```

2. **TaskStop**:
   - `task_id` ← primary
   - `agentId`, `bash_id` ← aliases
   
   ```rust
   task_id: opt_str_field("task_id")
       .or_else(|| opt_str_field("agentId"))
       .or_else(|| opt_str_field("bash_id"))
   ```

3. **Skill**:
   - `skill` ← alias for `name`

**MISSING** (macro-generated only):

- **TaskUpdate** uses macro-generated parser
  - `blocked_by` field declared but **NO `depends_on` alias**
  - Macro rules don't yet support fallback aliases
  - **Gap**: Code that sends `{depends_on: [...]}` to TaskUpdate will fail

---

## Tool Input Validation & Error Handling

### Validation Errors

**Type**: `ToolInputError` enum

```rust
pub enum ToolInputError {
    MissingField { tool, field },
    WrongType { tool, field, expected, got },
    InvalidShape { tool, reason },
    InvalidValue { tool, field, reason },
}
```

**Example** (from logs):
```
tool_result received tool_id=toolu_01X... is_error=true output_len=32
(Error message: "field 'file_path' was not a string")
```

**Handling**:

1. Parse fails → `ToolOutput::Error { message }`
2. Message appended to assistant's tool response
3. Stream continues (model sees error, can retry)
4. **Logged as**: `is_error=true` in scheduler

### JSON Schema Registration

Each tool registers its schema in `ToolDef`:

```rust
ToolDef {
    name: "Read".into(),
    description: "Read a file or directory...",
    input_schema: serde_json::json!({
        "type": "object",
        "properties": {
            "file_path": { "type": "string", ... },
            "offset": { "type": "number", ... },
            "limit": { "type": "number", ... }
        },
        "required": ["file_path"]
    }),
}
```

**Advertised to Claude**: These schemas guide parameter generation.

---

## Tool Output & Results

### Output Types

```rust
pub enum ToolOutput {
    Success(String),
    Error(String),
    Timeout,
    Cancelled,
}
```

### Session Persistence

After tool completes, `MessagePart::Tool` is appended:

```rust
ToolCall {
    id: tool_id,
    kind: ToolKind,
    status: ToolStatus::Complete,
    input: ToolInput,
    output: ToolOutput,
    elapsed_ms: Some(duration),
    // ...
}
```

**Session save** triggers, filtering out placeholder messages and coalescing sub-stream splits.

### Agentic Loop Continuation

If stream completes with `stop_reason=tool_use`:

1. All tools dispatched and complete
2. `build_assistant_and_tool_result_messages()` constructs next prompt
3. **New stream opened** with:
   - Previous messages (user + assistant messages up to this turn)
   - Tool results embedded as assistant tool_result blocks
   - New system prompt + memory

**Logged**:
```
agentic loop continuing — tools complete, no pending approvals
setup_new_substream_slot: staging new assistant slot assistant_idx=2 
total_messages=3 sub_stream="agentic_loop"
```

---

## Streaming Lifecycle: Complete Flow

### Example Session Trace (from logs)

```
04:30:25.793930Z  INFO handle_submit: spawning stream_response
04:30:31.274159Z  INFO jfc::provider::anthropic_sse: first SSE body bytes received
04:30:31.325316Z DEBUG jfc::stream::lifecycle: first stream byte — connection producing output
04:30:32.153489Z DEBUG jfc::stream: tool_done index=1 tool_name=graph_grep tool_use_id=...
04:30:32.153542Z  INFO jfc::ui::tool: StreamTool received tool_kind="GraphGrep" 
                  route=deferred_dispatch streaming_idx=Some(1)
04:30:32.622698Z DEBUG jfc::stream: tool_done index=2 tool_name=Grep tool_use_id=...
04:30:32.676686Z  INFO jfc::stream: server-side context management active
04:30:32.682047Z  INFO jfc::stream: stream finished — sending StreamDone stop_reason=ToolUse
04:30:32.682085Z  INFO jfc::stream: StreamEvent::Done received stop_reason=ToolUse pending_tool_count=2
04:30:32.682608Z  INFO dispatch_tools_batched: splitting tool calls task_count=0 regular_count=2
04:30:32.682825Z DEBUG jfc::scheduler: executing sequential tool tool_id=... kind=GraphGrep
04:30:36.632157Z  INFO jfc::scheduler: tool completed tool_id=... kind=GraphGrep outcome=Success output_len=4561
04:30:36.632218Z DEBUG jfc::scheduler: executing parallel batch batch_size=1 kinds=[Grep]
04:30:36.646520Z  INFO jfc::stream: ToolEvent::AllComplete message_count=2 pending_tool_calls=0
04:30:36.646811Z  INFO jfc::stream: agentic loop continuing — tools complete
04:30:36.647441Z  INFO stream_response{...messages=3}: setup_new_substream_slot: staging new assistant slot
04:30:38.433453Z  INFO stream{model=..., messages=3, tools=68}: stream opened successfully
```

---

## Known Issues & Gaps

### 1. ⚠️ TaskUpdate Missing `depends_on` Alias

**Issue**: TaskUpdate tool accepts `blocked_by` field but not `depends_on` alias.

**Location**: `crates/jfc-core/src/tool_input.rs:249` (macro-generated)

**Impact**: Code using:
```json
{ "task_id": "t1", "depends_on": ["t2", "t3"] }
```
Will fail with:
```
ToolInputError::MissingField { field: "blocked_by" }
```

**Fix Required**: Add hand-written parser for TaskUpdate (similar to TaskCreate) that supports both keys.

### 2. ✅ Tool Name Aliases Working Correctly

**Status**: Fully functional.

Examples that work:
- `TaskCreate` ≈ `task_create` ≈ `taskcreate` ≈ `TASK_CREATE`
- `GraphGrep` ≈ `graph_grep`
- Underscore/case variations all normalized

### 3. Parameter Type Coercion

**Issue**: Numeric fields strict on type.

Example: If `offset` sent as `"50"` (string) instead of `50` (number), will error:
```
WrongType { field: "offset", expected: "number", got: "string" }
```

**Possible Fix**: Add `opt_u64_loose_field()` coercion (already exists in code but not yet used everywhere).

### 4. Stream Timeout on Long Operations

**Observed**: Bash command that ran >120s logged as Failed/timeout.

```
WARN jfc::tools: bash: command timed out timeout_ms=120000
INFO jfc::scheduler: tool completed tool_id=... kind=Bash outcome=Failed output_len=32
```

**Known**: This is by design (120s is configurable), but long-running cargo/tests hit it.

### 5. Tool Execution Order

**Current**: Sequential + parallel batching via scheduler.

**Limitation**: Parallel batch size fixed; no dynamic adjustment based on load.

---

## Recommendations

### Short Term

1. **Add depends_on alias to TaskUpdate** — sync with TaskCreate behavior
2. **Enhance macro str_vec to support fallback aliases** — generalize for future tools
3. **Document parameter coercion rules** — help Claude generate stricter parameters

### Medium Term

1. **Implement loose parameter coercion** — accept `"number"` as valid for numeric fields (common in multipart form data)
2. **Add streaming telemetry** — duration breakdown by phase (setup, SSE, dispatch, execute)
3. **Timeout configurability** — expose timeout_ms per tool type

### Long Term

1. **Parameter validation on stream open** — fail fast if tools mismatch schema
2. **Tool call caching** — deduplicate identical sequential calls
3. **Partial output handling** — persist incomplete tool results for long operations

---

## References

- **Streaming entry**: `crates/jfc-ui/src/stream/mod.rs`
- **Tool execution**: `crates/jfc-ui/src/tools/dispatch.rs`
- **Tool input parsing**: `crates/jfc-core/src/tool_input.rs`
- **Tool kind resolution**: `crates/jfc-core/src/tool_kind.rs`
- **SSE events**: `crates/jfc-ui/src/provider/anthropic_sse.rs`
- **Event routing**: `crates/jfc-ui/src/stream/live_events.rs`
- **Session persistence**: `crates/jfc-ui/src/session/mod.rs`
