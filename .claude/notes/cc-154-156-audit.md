# Claude Code 2.1.154 / 2.1.156 — what's actually new

> Comparing the beautified `cli.js` from CC 2.1.153 → 2.1.154 → 2.1.156.
> Methodology: bun_binary_extractor + js-beautify -d on the linux-x64 binaries.
> Findings cross-checked: anything claimed "new" was confirmed `grep -c` = 0 in 153.

## Corrections vs first pass

A loose `"/[a-z][a-z0-9_:-]+"` regex produced three false positives — these are NOT new slash commands:

| String           | Actually                                                         |
| ---------------- | ---------------------------------------------------------------- |
| `/emcc`          | `process.env.CC` endswith check (Emscripten compiler detection)  |
| `/ld-linux-`     | dynamic-linker path detection (glibc)                            |
| `/ld-musl-`      | dynamic-linker path detection (musl)                             |
| `/v1/oauthtoken` | Google STS URL inside `googleapis` lib — not an Anthropic route  |

Also: **`claude-in-chrome` and `voice_stream` are NOT new in 154**. 153 already had 39 chrome-skill and 30 voice_stream references; 154 only refines them. The genuinely new browser-tool work is the bare `computer` + `navigate` + `browser_batch` JSON tool schemas (see Capability bucket below).

## 153 → 154 — confirmed deltas

### 🔐 Security / Privacy (review before porting)

| Area                                                         | What                                                                                              |
| ------------------------------------------------------------ | ------------------------------------------------------------------------------------------------- |
| `mid-conversation-system-2026-04-07` beta usage              | 1 → 9 hits. New: gating function `XH8` returns `true` for `claude-opus-4-8` (opt-in for opus-4-8) |
| `CLAUDE_CODE_FORCE_MID_CONVERSATION_SYSTEM` env              | NEW. Forces the beta on regardless of model gating — prompt-injection-adjacent capability         |
| `ANTHROPIC_BETAS` env var                                    | Already existed, but now explicitly read in `G36` to append arbitrary betas. Verify trust         |
| `CLAUDE_CODE_DISABLE_CLAUDE_CODE_SKILL`                      | NEW kill-switch for the `claude-code-docs` skill                                                  |
| `CLAUDE_CODE_SKILL_NAME` / `CLAUDE_CODE_SKILL_DESCRIPTION`   | NEW. Exposed via `aj9` module — lets callers rename/redescribe the built-in skill                 |
| `VOICE_STREAM_BASE_URL` env override                         | Already existed; lets users redirect Deepgram WS connection                                       |

### 🚀 Capability

| Feature                                  | Details                                                                                                                                                                                                                                       |
| ---------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **`claude-opus-4-8`**                    | Already done (commit c797ea0). New first-party default, 1M context, 128k output                                                                                                                                                               |
| **`claude-sonnet-4-6-20251114`**         | Already done (commit c797ea0). Dated sonnet 4.6 pin                                                                                                                                                                                          |
| **`ultracode` setting** (NEW)            | Boolean. Enables: xhigh effort + standing dynamic-workflow orchestration. Session-scoped (`--settings` or `apply_flag_settings` control req). Requires workflows + xhigh-capable model. Routed via `i6().ultracode === !0` (`zP6` / `e$7`)    |
| **Bare browser tools** (NEW)             | `computer`, `navigate`, `browser_batch`, `tabs_context_mcp`, `tabs_create_mcp`, `tabs_close_mcp`, `screenshot`, `zoom`, `resize_window`, `read_console_messages`, `read_network_requests`, `read_page`, `read_clipboard`, `list_connected_browsers`, `select_browser`, `switch_browser`. Used internally by the `claude-in-chrome` MCP. Schemas in the bundle |
| **`browser_batch`** (NEW)                | Tool that executes N browser-tool calls in one round-trip; stops on first error. Permission check runs per item                                                                                                                              |
| **`list_connected_browsers` semantics**  | Returns devices "on this computer" — implies cross-device chrome-extension visibility. Worth a privacy note                                                                                                                                  |
| **`claude-code-docs` skill** (NEW)       | Built-in skill (`zRz` registers it). Answers questions about CC itself, gated on `tengu_birch_kettle` LD flag. Allowed tools: `Read`, `Grep`, `Glob`, `WebFetch`                                                                              |
| **`anthropic-version` for opus-4-8**     | Bundle now includes example `curl https://api.anthropic.com/v1/models/claude-opus-4-8 -H "anthropic-version: 2023-06-01"` — confirms public API exposes the model                                                                              |

### 📊 Telemetry (`tengu_*` events new in 154)

| Event                                          | Purpose                                                              |
| ---------------------------------------------- | -------------------------------------------------------------------- |
| `tengu_basalt_meadow`                          | LD/feature flag (drives `pA9` xterm reset path)                      |
| `tengu_birch_kettle`                           | LD flag gating `claude-code-docs` skill                              |
| `tengu_cobalt_thicket`                         | LD flag (use TBD)                                                    |
| `tengu_xterm_atlas_reset`                      | LD flag triggering renderer atlas reset                              |
| `tengu_render_glyph_cardinality`               | Reports stylepool + atlas-glyph stats, terminal program, isXtermjs   |
| `tengu_ultra_effort`                           | Referenced in fields list — actual event sites in `_R_` / `ultra_effort_enter` |
| `tengu_claude_code_skill_loaded`               | Fires when claude-code-docs skill answers (with `has_args` boolean)  |
| `tengu_post_compact_survey` (+ `_event`)       | 20% sample (`iIz = 0.2`), 5s delay (`lIz = 5000`). Records `appeared` / `responded` + the user's response                                            |
| `tengu_hook_prompt_too_long_retry`             | Retry telemetry for oversized hook prompts                           |
| `tengu_org_penguin_mode_fetch_failed`          | Org-level "penguin mode" (fast-mode credit gating) fetch failure     |
| `tengu_wif_implicit_profile_skipped_stored_login` | OAuth/WIF flow                                                       |
| `tengu_bg_daemon_spawn_versions_fallback`      | Background daemon spawn fallback path                                |

### 🛠 Other notable additions

- New `ANTHROPIC_BETAS` env reader path in `G36` — splits on `,` and appends to the beta list per request.
- New `vertex-2023-10-16` anthropic-beta token.
- New `/plugin:about` slash command convention (one example; the picker reads commands from plugin manifests' `commands` map).
- `tengu_bg_daemon_spawn_versions_fallback` joins the existing daemon zoo.
- `j3` (ALL_MODEL_CONFIGS) gains `opus48: Xi$` slot; `he()` first-party default flipped from `opus47` → `opus48`.
- `isNonCustomOpusModel` whitelist extended to include `opus-4-8`.
- `eagerInputStreaming` enabled for opus-4-8 on bedrock + vertex.

## 154 → 156 — diff is tiny

Only one substantive change:

| Event                                              | What                                                                                                                                                                                                                                                                                     |
| -------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `tengu_reorder_tool_uses_skipped_for_thinking`     | New telemetry inside the tool-use reorder pass (the loop that moves all `tool_use` blocks to the end of a message's content). When a `thinking` block sits between tool_uses, the reorder is **skipped** (would otherwise corrupt thinking ordering). Logs `contentLength`, `firstToolUseIdx` |

Plus 2 commit-hash literals swapped (build-stamp).

---

## Triage for porting into jfc

Default bias: **port what affects parity with users on `latest`**, document everything else.

### Recommend porting

1. **`mid-conversation-system-2026-04-07` beta for opus-4-8** — without this, opus-4-8 requests won't get the system prompt re-injected mid-conversation that CC users get. Match `XH8`'s model gate.
2. **`ANTHROPIC_BETAS` env passthrough** — single env var → appended to the beta header. Tiny change, big debugging value.
3. **`tengu_reorder_tool_uses_skipped_for_thinking` equivalent** — we already moved tool_use blocks; check whether we have the thinking-block guard. If we don't, that's a latent bug analogous to the abandoned-tool race fixed yesterday.

### Recommend documenting (don't port yet)

4. ~~**`ultracode` setting**~~ — **ported** in follow-up. The "force xhigh effort" half maps cleanly to `[default]` / `[agents.*]` toml; CC's `e$7` "ultracode wins over effortLevel" semantics mirrored in `resolve_effort_for_model` (precedence 0). The "standing dynamic-workflow orchestration" half is jfc-orthogonal — our workflow runner is its own thing.
5. **Bare browser tools / `browser_batch`** — these are MCP-side schemas, not core runtime. If we want browser automation we should consume the `claude-in-chrome` MCP via the existing MCP plumbing, not bake schemas into jfc.
6. **`claude-code-docs` skill** — built-in skill for self-documentation. Out of scope for jfc; users have their own docs.
7. **`tengu_birch_kettle` / `tengu_basalt_meadow` / `tengu_cobalt_thicket`** — Anthropic-internal LaunchDarkly flags. No-op for us.

### Skip

8. **`tengu_post_compact_survey`** — UX survey loop tied to Anthropic's product analytics.
9. **`VOICE_STREAM_BASE_URL` / Deepgram WS** — voice already existed in 153; no behavioural change.
10. **`tengu_xterm_atlas_reset` / `tengu_render_glyph_cardinality`** — Anthropic's terminal renderer tuning; doesn't apply to our TUI.

## Source files

- Binaries: `/tmp/cc-2.1.154/package/claude`, `/tmp/cc-2.1.156/package/claude` (linux-x64, 230 MB each).
- Extracted: `/tmp/cc-2.1.{154,156}/extracted/src/entrypoints/cli.js` (15 MB minified) and `cli.beautified.js` (25 MB).
- Cached prior audit: `/home/cole/RustProjects/active/claude-code-2.1.153-audit/cli.beautified.js`.
