# JFC Tool Call Issues: Complete Analysis

## Executive Summary

Analysis of the latest session logs and codebase reveals **5 major tool call issues** and **3 parameter aliasing gaps**. All issues prevent tools from working correctly or limit compatibility with Claude Code patterns.

---

## Issue #1: TaskUpdate Missing `depends_on` Alias ⚠️ HIGH PRIORITY

**Status**: CONFIRMED ISSUE

**Location**: `crates/jfc-core/src/tool_input.rs:249` (macro-generated parser)

**Problem**: 
- `TaskCreate` supports both `blocked_by` and `depends_on` field names
- `TaskUpdate` only accepts `blocked_by`
- Code using `depends_on` will fail validation

**Code Path**:
```rust
// TaskCreate (working with alias)
TaskCreate => { 
    ..., 
    blocked_by: str_vec @ "blocked_by", 
    ... 
}
// Line 1198-1200: Hand-written fallback parser
let blocked_by = obj
    .and_then(|map| map.get("blocked_by").or_else(|| map.get("depends_on")))
    .and_then(|value| value.as_array())

// TaskUpdate (NO FALLBACK - uses macro only)
TaskUpdate => { 
    ..., 
    blocked_by: str_vec @ "blocked_by", 
    ... 
}
// No hand-written parser!
```

**Example Failure**:
```json
{
    "task_id": "t1",
    "depends_on": ["t2", "t3"]  // ← Will fail
}
```
Error:
```
ToolInputError::MissingField { 
    tool: "TaskUpdate", 
    field: "blocked_by" 
}
```

**Root Cause**: 
- TaskUpdate uses macro-generated `str_vec @ "blocked_by"` parser
- Macro rules don't support fallback aliases yet
- TaskCreate has hand-written exception code (lines 1198-1207)

**Fix**:
Add hand-written parser for TaskUpdate in `from_value()` match, parallel to TaskCreate.

```rust
ToolKind::TaskUpdate => {
    // depends_on is an alias for blocked_by
    let blocked_by = obj
        .and_then(|map| map.get("blocked_by").or_else(|| map.get("depends_on")))
        .and_then(|value| value.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    // ... parse other fields ...
}
```

---

## Issue #2: SendUserFile Parameter Validation Too Strict ⚠️ MEDIUM PRIORITY

**Status**: OBSERVED IN LOGS (04:32:29 session)

**Logged**:
```
2026-06-01T04:32:29.709491Z  INFO jfc::scheduler: tool completed tool_id=toolu_0126k2JDprAJGRbi7MPopmjY 
                             kind=SendUserFile outcome=Failed output_len=166
2026-06-01T04:32:29.711751Z  INFO jfc::stream: tool_result received tool_id=toolu_0126k2JDprAJGRbi7MPopmjY 
                             is_error=true output_len=166
```

**Location**: `crates/jfc/src/tools/dispatch.rs:xxx` (SendUserFile handler)

**Problem**:
```rust
if paths.is_empty() {
    ExecutionResult::failure(
        "SendUserFile requires a non-empty `files` array of paths.".to_string(),
    )
} else {
    let mut delivered = Vec::new();
    let mut errors = Vec::new();
    for p in &paths {
        let abs = if path.is_absolute() { 
            path.to_path_buf() 
        } else { 
            cwd.join(path) 
        };
        match std::fs::metadata(&abs) {
            Ok(meta) if meta.is_file() => {
                delivered.push(format!("{} ({} bytes)", abs.display(), meta.len()));
            }
            Ok(_) => errors.push(format!("{}: not a regular file", abs.display())),
            Err(e) => errors.push(format!("{}: {e}", abs.display())),
        }
    }
    // ...
    if errors.is_empty() {
        ExecutionResult::success(out)
    } else {
        ExecutionResult::failure(out)  // ← FAILS if ANY error
    }
}
```

**Failure Modes**:
1. **File not found**: Path doesn't exist
2. **Not a regular file**: Path is a directory or symlink
3. **Permission denied**: Can't read file metadata
4. **Mixed delivery**: Some files exist, others don't → treated as failure

**Error from logs** (166 bytes output):
```
Errors:
  <missing-file-or-path>: No such file or directory
```

**Recommendation**:
- Allow partial success: return `success()` if at least one file delivered
- Log warnings for failed paths instead of failing entire call
- Or: require `all_or_nothing` parameter

---

## Issue #3: Scratchpad Lock Contention on Concurrent Access ⚠️ MEDIUM PRIORITY

**Status**: POTENTIAL ISSUE (not observed but design concern)

**Location**: `crates/jfc/src/tools/scratchpad.rs:14-47`

**Problem**:
```rust
#[cfg(unix)]
fn lock_scratchpad(lock_path: &Path) -> std::io::Result<std::fs::File> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .read(true)
        .open(lock_path)?;
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(file)
}

pub(crate) fn execute_scratchpad_read(key: &str) -> ExecutionResult {
    let path = scratchpad_path();
    let lock_path = path.with_extension("json.lock");
    let _guard = match lock_scratchpad(&lock_path) {
        Ok(guard) => guard,
        Err(e) => return ExecutionResult::failure(format!("Scratchpad lock failed: {e}")),
    };
    // ...
}
```

**Issues**:
1. **Blocking lock**: `libc::flock(LOCK_EX)` blocks indefinitely
2. **No timeout**: If sibling agent holds lock, reader may hang forever
3. **Synchronous I/O**: Blocks tokio reactor when spawned from async context
4. **Windows fallback**: Non-Unix uses exclusive file open (not a lock)

**Scenario**:
- Agent A writes to scratchpad, holds lock
- Agent B tries to read, waits for lock
- Agent A crashes or freezes → Agent B deadlocked

**Evidence**:
```rust
// Current spawn pattern (dispatch.rs:964-966)
tokio::task::spawn_blocking(move || execute_scratchpad_read(&key))
    .await
    .unwrap_or_else(|e| ExecutionResult::failure(...))
```

Blocking task will block the blocking thread pool if contention is high.

**Fix Options**:
1. Add timeout: `flock()` → `flock()` with LOCK_NB + retry loop
2. Use `parking_lot::Mutex` for in-memory locking
3. Add backoff + jitter for retries
4. Document: "scratchpad is not suitable for real-time coordination between agents"

---

## Issue #4: Parameter Type Coercion Strict (Numeric Fields) ⚠️ LOW PRIORITY

**Status**: DESIGN ISSUE (not a bug)

**Location**: `crates/jfc-core/src/tool_input.rs:61-78` (parse macros)

**Problem**:
```rust
($obj:ident, $tool:ident, opt_u64, $k:literal) => {
    $obj.and_then(|m| m.get($k)).and_then(|v| v.as_u64())
};
```

If `offset: "50"` (string) is sent instead of `offset: 50` (number):
```
ToolInputError::WrongType { 
    field: "offset", 
    expected: "number", 
    got: "string" 
}
```

**Why This Matters**:
- HTML form data sends `"50"` as string
- Some clients convert parameters to strings
- Claude sometimes outputs `"offset": "50"` instead of `"offset": 50`

**Evidence**:
- Function `opt_u64_loose_field()` exists but unused:
  ```rust
  let opt_u64_loose_field = |key: &str| -> Option<u64> {
      obj.and_then(|map| map.get(key)).and_then(|value| {
          value
              .as_u64()
              .or_else(|| value.as_str().and_then(|s| s.trim().parse::<u64>().ok()))
      })
  };
  ```

**Fix**: Replace macro rules to use coercion function for numeric fields:
```rust
($obj:ident, $tool:ident, opt_u64_loose, $k:literal) => {
    $obj.and_then(|m| m.get($k)).and_then(|v| {
        v.as_u64()
            .or_else(|| v.as_str().and_then(|s| s.trim().parse::<u64>().ok()))
    })
};
```

---

## Issue #5: Tool Alias Macro Needs `str_vec_alias` Rule ⚠️ LOW PRIORITY

**Status**: FEATURE GAP

**Location**: `crates/jfc-core/src/tool_input.rs:127-137`

**Existing Rule**:
```rust
($obj:ident, $tool:ident, str_vec_alias, $k:literal, $alias:literal) => {
    $obj.and_then(|m| m.get($k).or_else(|| m.get($alias)))
        .and_then(|v| v.as_array())
        // ...
}
```

**Status**: Rule exists but **NOT USED anywhere** in macro tables!

**Use Case**:
```rust
// Could be added to TaskUpdate table:
TaskUpdate => { 
    ..., 
    blocked_by: str_vec_alias @ "blocked_by", "depends_on",
    ... 
}
```

**Problem**: Macro table declaration doesn't support passing multiple keys. 

**Why**: Macro is designed for single-field-per-directive. Adding multi-key support requires redesign.

**Workaround**: Use hand-written parser (as TaskCreate does).

---

## Issue #6: SendUserMessage Parameter Handling Incomplete ⚠️ LOW PRIORITY

**Status**: DESIGN ISSUE

**Location**: `crates/jfc-core/src/tool_input.rs` (hand-written, not in macro)

**Missing Fields**:
```rust
pub enum ToolInput {
    SendUserMessage {
        message: String,
        #[serde(default)]
        summary: Option<String>,
        #[serde(default)]
        status: Option<String>,  // "normal" | "proactive"
        // MISSING: escalation_level, priority, timeout, etc.
    }
}
```

**Limitation**: 
- No way to specify urgency or escalation
- No timeout for response
- No categorization (alert vs. info vs. debug)

**Not blocking**, but limits use cases.

---

## Issue #7: Graph Tool Names Need Alias Support ⚠️ LOW PRIORITY

**Status**: POTENTIAL CONFUSION

**Current**:
- `GraphSearch` ← tool definition in defs
- `graph_search` ← Claude prefers lowercase

**Resolution**: Already works via `tool_name_eq()` case-insensitivity.

**No action needed**. Included for completeness.

---

## Issue #8: EnterWorktree Parameter Validation Missing

**Status**: DESIGN ISSUE

**Location**: `crates/jfc-core/src/tool_input.rs:302`

```rust
EnterWorktree => { 
    name: req_str @ "name", 
    branch: opt_str @ "branch" 
}
```

**Problems**:
1. No validation of `name` format (should be alphanumeric + `-_`)
2. No validation of `branch` format (git refs can be malicious)
3. No check for reserved names (`..`, `/root`, etc.)

**Risk**: Low (worktree creation itself validates), but input sanitization is missing.

---

## Tool Alias System: WORKING CORRECTLY ✅

**Confirmed Working**:
```rust
pub fn from_name(name: &str) -> Self {
    return_tool_kind!(name,
        Self::Edit => ["edit", "str_replace_based_edit_tool"],
        Self::Read => ["read", "read_file"],
        Self::TaskCreate => ["task_create"],
        // ... all 68 tools
    );
}

fn tool_name_eq(candidate: &str, alias: &str) -> bool {
    let mut lhs = candidate.bytes().filter(|b| *b != b'_');
    let mut rhs = alias.bytes().filter(|b| *b != b'_');
    loop {
        match (lhs.next(), rhs.next()) {
            (Some(a), Some(b)) if a.eq_ignore_ascii_case(&b) => {}
            (None, None) => return true,
            _ => return false,
        }
    }
}
```

**All work**:
- `TaskCreate` ≈ `task_create` ≈ `taskcreate` ≈ `TASK_CREATE` ✓
- `GraphSearch` ≈ `graph_search` ✓
- `Read` ≈ `read_file` ✓
- `skill` ← alias for `Skill` (hand-written exception) ✓

---

## Session Error Summary

**From logs (04:32:29)**:

| Tool | ID | Status | Error | Cause |
|------|----|----|-------|-------|
| SendUserFile | toolu_0126k... | Failed | "file not found" or "not a regular file" | Missing file or symlink/dir |
| TaskUpdate | N/A | N/A | (untested) | Would fail if using `depends_on` |

---

## Recommendations (Priority Order)

### 🔴 CRITICAL (Fix immediately)

1. **TaskUpdate `depends_on` alias** — Line 1249+
   - Add hand-written parser similar to TaskCreate (lines 1197-1248)
   - Support both `blocked_by` and `depends_on`
   - Test: `TaskUpdate` with `depends_on: ["t1"]`

### 🟡 IMPORTANT (Next sprint)

2. **Macro `str_vec_alias` rule adoption** — Consider redesigning macro to support multi-key aliases
   - Would prevent future TaskUpdate-like issues
   - High effort, benefits TaskUpdate + future tools

3. **Numeric parameter coercion** — Add `opt_u64_loose` rule variant
   - Replace all `opt_u64` usages that can tolerate string input
   - Low-risk change, improves robustness

4. **SendUserFile partial success** — Change failure logic
   - Return `success()` if ANY file delivered
   - Move failed paths to warning output
   - Or add `all_or_nothing: bool` parameter

### 🟢 NICE-TO-HAVE (Future)

5. **Scratchpad timeout** — Add configurable lock timeout
   - Prevents deadlocks with frozen agents
   - Default: 5s, configurable via env

6. **Graph tool description improvements** — Add more aliases
   - `graph_context` ← `context` (too generic?)
   - `graph_query` ← `query` (ditto)

---

## Testing Plan

### Unit Tests to Add

1. **test_task_update_depends_on_alias()**
   ```rust
   #[test]
   fn test_task_update_depends_on_alias() {
       let input = json!({
           "task_id": "t1",
           "depends_on": ["t2", "t3"]
       });
       let result = ToolInput::from_value("TaskUpdate", input);
       assert!(result.is_ok());
       if let ToolInput::TaskUpdate { blocked_by, .. } = result.unwrap() {
           assert_eq!(blocked_by, vec!["t2", "t3"]);
       }
   }
   ```

2. **test_send_user_file_partial_success()**
   - Mixed existing/missing files → success if ANY delivered

3. **test_numeric_parameter_string_coercion()**
   - `Read` with `"offset": "50"` (string) → parsed as u64

### Integration Tests

1. Stream a response that calls `TaskUpdate` with `depends_on`
2. Stream a response that calls `SendUserFile` with partial paths
3. Concurrent scratchpad read/write (no lock timeout)

---

## Code Locations Summary

| Issue | File | Line | Type |
|-------|------|------|------|
| TaskUpdate alias | `tool_input.rs` | 1249+ | Enhancement |
| SendUserFile validation | `dispatch.rs` | ~950 | Enhancement |
| Scratchpad locking | `scratchpad.rs` | 14-47 | Enhancement |
| Parameter coercion | `tool_input.rs` | 60-80 | Enhancement |
| str_vec_alias rule | `tool_input.rs` | 127-137 | Infrastructure |
| EnterWorktree sanitization | `tool_input.rs` | 302 | Enhancement |

---

## References

- **Streaming flow**: `/docs/STREAMING_TOOL_CALLS.md`
- **Tool dispatch**: `crates/jfc/src/tools/dispatch.rs`
- **Parameter parsing**: `crates/jfc-core/src/tool_input.rs`
- **Session logs**: `~/.config/jfc/logs/latest.log`
- **Session file**: `~/.config/jfc/sessions/ses_20260601_042950.json`
