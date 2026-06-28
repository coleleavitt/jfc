# Compatibility Re-Export Ledger

Task 30 of `.omo/plans/jfc-pi-opencode-kernel-overhaul.md` records the temporary compatibility surfaces introduced or preserved while ownership moves toward the short-root kernel architecture. These entries are deletion candidates, not destination APIs, unless the deletion criterion says the surface has become canonical.

## Deletion Rule

Delete a compatibility shim only after all internal call sites and tests import the destination owner directly, workspace dependency guards allow the destination path, and the relevant focused tests pass without the shim. If the shim touches persisted transcript/config/session data, keep old-format fixtures green until task 33/34 records the replacement public API.

## Ledger

| ID | Shim path | Compatibility surface | Destination owner | Delete when |
| --- | --- | --- | --- | --- |
| CRL-001 | `crates/jfc-engine/src/auth/mod.rs` | `crate::auth::{device_flow, sts}` re-export | `jfc-auth` | Engine callers import `jfc_auth::{device_flow, sts}` or auth is behind a provider/plugin service, and no `crate::auth::` imports remain. |
| CRL-002 | `crates/jfc-engine/src/bash_processes.rs` | `crate::bash_processes::*` re-export | `jfc-tools::bash_processes` | Bash process users move to tool-pack/service-owned imports and engine no longer exposes tool implementation helpers. |
| CRL-003 | `crates/jfc-engine/src/config.rs` | `jfc_config::*` re-export plus engine-specific permission/slate wrappers | `jfc-config` plus runtime policy service | UI/runtime callers depend on `jfc-config` for schema and on a policy service for runtime conversions; no `crate::config::*` compatibility imports remain. |
| CRL-004 | `crates/jfc-engine/src/context.rs` | Historical `crate::context::*` path for `ToolContext`, `ReadDedupCache`, and context loaders | `jfc-core::context` now, later `crates/context` for context pack ownership | Prompt/tool paths import the destination context APIs directly and context pack integration owns cache/layout loaders. |
| CRL-005 | `crates/jfc-engine/src/cost.rs` | `jfc_economy::cost::*` re-export and `cost_for_background_task` wrapper | `jfc-economy::cost` plus future task/runtime state owner | Background task cost accounting moves behind a runtime/economy service and no engine caller imports cost tables through `crate::cost`. |
| CRL-006 | `crates/jfc-engine/src/daemon/mod.rs` | `jfc_daemon::*` re-export while worker execution remains engine-bound | `jfc-daemon` plus runtime worker boundary | Daemon CLI/runtime callers import `jfc-daemon` directly and worker spawn no longer depends on full engine internals. |
| CRL-007 | `crates/jfc-engine/src/daemon_services.rs` | Engine aliases and global install hook for scheduled-task service | `jfc-daemon::scheduled_tasks` service seam | Scheduled-task management is injected through `RuntimeServices` or daemon pack ownership and no caller relies on `DaemonScheduledTask*` aliases. |
| CRL-008 | `crates/jfc-engine/src/learn_lifecycle.rs` | `crate::learn_lifecycle::*` re-export | `jfc-learn::lifecycle` | Learn/dreamer lifecycle call sites move to the learn/context service owner and engine stops exporting learn lifecycle internals. |
| CRL-009 | `crates/jfc-engine/src/mcp/mod.rs` | `crate::mcp::*` re-export | `jfc-mcp` | MCP callers import `jfc-mcp` or MCP registry services directly and no engine module path is used for MCP APIs. |
| CRL-010 | `crates/jfc-engine/src/providers/mod.rs` | `crate::providers::*` re-export | `jfc-providers` and provider descriptor registry | Provider construction/selection is through provider packs/registry and no caller imports concrete providers via engine. |
| CRL-011 | `crates/jfc-engine/src/runtime/execution.rs` | Runtime `ExecutionResult`, `ToolErrorCategory`, `ToolProvenance`, `ToolSource` re-export | `jfc-core` | Runtime/event callers import execution DTOs from `jfc-core` or a protocol crate and no `crate::runtime::ExecutionResult` path remains. |
| CRL-012 | `crates/jfc-engine/src/scaffold_detector.rs` | `crate::scaffold_detector::*` re-export | `jfc-learn::scaffold_detector` | Scaffold detection is invoked from learn/context pack code and not through engine. |
| CRL-013 | `crates/jfc-engine/src/session/serialization.rs` | `crate::session::serialization::Serialized*` transcript re-export | `jfc-session::transcript` | Engine session serialization callers import `jfc-session::transcript` directly and old session fixtures still load. |
| CRL-014 | `crates/jfc-engine/src/session/store.rs` and `crates/jfc-engine/src/session/mod.rs` | Engine `SessionStore`, request DTO, and save/load wrapper facade | `jfc-session::store` | Engine `ChatMessage` conversion is owned by session/runtime services, callers use `jfc-session` store DTOs directly, and autosave/load tests pass without engine store wrappers. |
| CRL-015 | `crates/jfc-engine/src/web_cache.rs` | `crate::web_cache::*` re-export | `jfc-web::cache` | Web cache callers import `jfc-web` or a tool/context pack service directly. |
| CRL-016 | `crates/jfc-engine/src/web_search.rs` | `crate::web_search::search` re-export | `jfc-web::search` | Web search is registered through a tool pack/service and no engine web-search path remains. |
| CRL-017 | `crates/jfc-engine/src/lib.rs` | `index_session` compatibility facade and `knowledge_maintain` thin re-export | `jfc-knowledge` plus session/context services | UI/binary crates depend on a session/context service instead of engine helpers, and knowledge maintenance/backfill tests cover direct destination calls. |
| CRL-018 | `crates/jfc-engine/src/lib.rs` | `jfc_engine::types` domain-type facade mirroring historical `crate::types` | `jfc-core` or future protocol crate | Internal and external callers import DTOs from the protocol/core owner and task 34 maps any public API deprecation. |
| CRL-019 | `crates/jfc-session/src/session_entry.rs` | Old flat `jfc_session::session_entry::*` path re-export | `jfc-session::entry` module tree | Callers import through the curated `jfc_session::*` surface or `entry` owner and old flat module path is not referenced. |
| CRL-020 | `crates/jfc/src/app/mod.rs` | Engine-side app/runtime type re-exports for historical `crate::app::X` paths | `jfc-engine::app`, `jfc-engine::runtime`, and `ui-model` | TUI owns only frontend state/view models and all engine state/runtime imports use destination crates directly. |
| CRL-021 | `crates/jfc/src/attachments.rs` | `jfc_engine::attachments::*` re-export from frontend attachments module | Attachment DTO owner in engine/core plus TUI clipboard adapter | Clipboard-only frontend code is split from attachment DTOs and no TUI code imports engine attachment types through `crate::attachments::*`. |
| CRL-022 | `crates/jfc/src/lib.rs` | Test-only library facet re-exporting `atomic_write`, `plan`, `plan_dreamer`, `plan_recall` | `jfc-engine` or future CLI/domain crates | Integration tests import destination crates directly or move to the owning crate, removing the binary crate library facade. |
| CRL-023 | `crates/jfc/src/runtime/mod.rs` | Engine runtime surface re-export for historical `crate::runtime::X` paths | `jfc-engine::runtime` and future runtime/protocol crates | TUI event loop imports runtime DTOs/services from the destination owner and no `crate::runtime::X` compatibility import remains. |
| CRL-024 | `crates/jfc/src/runtime/event_loop/handlers/mod.rs` | Engine handler glob re-export so pump paths keep compiling | `jfc-engine::runtime::event_loop::handlers` or frontend command adapters | TUI pump calls destination handlers directly or owns frontend-only adapters; no `handlers::x::y` shim import remains. |
| CRL-025 | `crates/jfc/src/theme.rs` | `jfc_theme::*` thin re-export from TUI crate | `jfc-theme` | TUI callers import `jfc-theme` directly and color utilities are either moved to `jfc-theme` or kept as explicit TUI-only helpers. |
| CRL-026 | `crates/jfc-providers/src/openwebui/mod.rs` | Test-only `load_account(path)` compatibility shim | `openwebui::store::{load_store, get_current}` | Existing tests are rewritten to exercise store APIs directly and the helper has no non-test callers. |

## Grep Proof Commands

Use these checks to keep the ledger synchronized with source shims:

```bash
rg -n "Re-export shim|Re-export of|re-export|backwards compatibility|Backwards-compat shim|historical .*paths|compatibility facade|Everything else re-exports|Thin re-export" crates/jfc-engine/src crates/jfc/src crates/jfc-session/src crates/jfc-providers/src
rg -n "CRL-00[1-9]|CRL-01[0-9]|CRL-02[0-6]" docs/compatibility-reexport-ledger.md
```
