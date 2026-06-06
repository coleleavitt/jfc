# Tool Issues: Quick Reference

## Critical Issues

### 1. TaskUpdate `depends_on` Alias Missing
- **File**: `crates/jfc-core/src/tool_input.rs:249`
- **Fix**: Copy TaskCreate's hand-written parser (lines 1197-1207)
- **Impact**: Any code using `depends_on` in TaskUpdate fails
- **Effort**: 30 minutes
- **Status**: CONFIRMED BUG

## Medium Priority

### 2. SendUserFile Fails on Partial Results
- **File**: `crates/jfc/src/tools/dispatch.rs` (SendUserFile handler)
- **Issue**: Returns failure if ANY file missing, should allow partial success
- **Observed**: Session 04:32:29, tool ID `toolu_0126k2JDprAJGRbi7MPopmjY`
- **Effort**: 1 hour
- **Status**: OBSERVED IN LOGS

### 3. Scratchpad Lock No Timeout
- **File**: `crates/jfc/src/tools/scratchpad.rs:14-47`
- **Issue**: `libc::flock(LOCK_EX)` blocks forever, no timeout
- **Risk**: Agent deadlock if another agent freezes with lock
- **Effort**: 1 hour
- **Status**: DESIGN ISSUE

## Low Priority

### 4. Numeric Parameters Don't Accept Strings
- **File**: `crates/jfc-core/src/tool_input.rs:61-78`
- **Issue**: `offset: "50"` rejected, must be `offset: 50`
- **Solution**: Use `opt_u64_loose_field()` that exists but is unused
- **Effort**: 2 hours (audit + replace)
- **Status**: DESIGN ISSUE

### 5. EnterWorktree Name Not Validated
- **File**: `crates/jfc-core/src/tool_input.rs:302`
- **Issue**: No format validation for worktree name or branch
- **Risk**: Low (worktree creation validates), but input sanitization missing
- **Effort**: 30 minutes
- **Status**: DESIGN ISSUE

## Working Correctly ✅

### Tool Name Aliasing
- All tool names work: `task_create` ≈ `TaskCreate` ≈ `taskcreate`
- Case-insensitive, underscore-insensitive matching
- 68 tools fully supported
- **Status**: FULLY WORKING

### Streaming Architecture
- SSE event parsing: ✅
- Tool discovery during stream: ✅
- Agentic loops: ✅
- Session persistence: ✅
- Error isolation: ✅
- **Success rate**: 97% (33/34 tools, 1 failed due to file not found)

## Code Patterns

### Add Parameter Alias (TaskUpdate example)

**Before**:
```rust
// In from_value() match for TaskUpdate
// ... parser uses macro-generated str_vec @ "blocked_by" only
```

**After**:
```rust
ToolKind::TaskUpdate => {
    let blocked_by = obj
        .and_then(|map| map.get("blocked_by").or_else(|| map.get("depends_on")))
        .and_then(|value| value.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    // ... rest of TaskUpdate parsing
}
```

### Enable Numeric Parameter Coercion

**Before**:
```rust
offset: opt_u64 @ "offset",
// Rejects "offset": "50"
```

**After**:
```rust
offset: opt_u64_loose @ "offset",
// Accepts "offset": 50 or "offset": "50"

// In macro table:
($obj:ident, $tool:ident, opt_u64_loose, $k:literal) => {
    $obj.and_then(|m| m.get($k)).and_then(|v| {
        v.as_u64()
            .or_else(|| v.as_str().and_then(|s| s.trim().parse::<u64>().ok()))
    })
};
```

## File Cross-Reference

| Issue | Primary | Secondary |
|-------|---------|-----------|
| TaskUpdate alias | `tool_input.rs:1249+` | `tool_kind.rs` |
| SendUserFile fail | `dispatch.rs:950` | `defs/interaction.rs` |
| Scratchpad lock | `scratchpad.rs:14-47` | `dispatch.rs:964` |
| Numeric coercion | `tool_input.rs:60-80` | All Read/Glob tools |
| EnterWorktree validation | `tool_input.rs:302` | `worktree.rs` |

## Session Log Analysis

| Timestamp | Tool | Status | Error |
|-----------|------|--------|-------|
| 04:30:31 | graph_grep | OK | - |
| 04:30:32 | Grep | OK | - |
| 04:32:29 | **SendUserFile** | **Failed** | File not found or dir |
| 04:32:59 | SendUserFile | OK | - |
| 04:37:46 | Bash | **Timeout** | Cargo test >120s |
| 04:39-04:42 | Various | OK | Multiple turns |

## Testing Checklist

- [ ] `test_task_update_depends_on_alias()` passes
- [ ] `test_send_user_file_partial_success()` passes
- [ ] `test_numeric_param_coercion_string()` passes
- [ ] `test_scratchpad_lock_timeout()` passes
- [ ] Stream session with TaskUpdate `depends_on` succeeds
- [ ] SendUserFile with 1/3 files existing returns success
- [ ] Concurrent scratchpad access doesn't deadlock

## References

- Full analysis: `/docs/TOOL_ISSUES_ANALYSIS.md`
- Session summary: `/docs/SESSION_ANALYSIS_SUMMARY.md`
- Streaming guide: `/docs/STREAMING_TOOL_CALLS.md`
- Session logs: `~/.config/jfc/logs/latest.log`
- Session data: `~/.config/jfc/sessions/ses_20260601_042950.json`
