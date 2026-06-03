# Claude Design → JFC parity roadmap

JFC ports the Claude Design product layer. This doc tracks what's landed and what
remains, and specifies the architecture for the remaining browser-host + rich
frontend parity phases. The status table below is generated from
`jfc-design::capabilities::matrix()` — keep that the single source of truth
(`jfc-design capabilities --md`).

## What's a port of *what*

Claude Design is a browser artifact workspace: a server, a React frontend, a live
preview iframe, browser-side tools (eval-js, screenshot, PPTX capture), design-
specific file APIs, direct DOM editing, design systems, and exporters. JFC already
had the lower-level pieces (skills, agents, tool dispatch, MCP, GitHub, file/Bash/
edit tools). This work adds the **design product layer** on top.

Four layers, four states:

1. **Behavior layer (DONE).** The base design-agent prompt + the full skill registry
   + the starter components. This makes JFC *act* like the design agent today, using
   its existing filesystem/web tools to execute. Lives in `.claude/agents/designer.md`
   and `.claude/skills/jfc-design/**`.
2. **Native core (DONE).** Pure-Rust foundation in the `jfc-design` crate: design
   projects + sandboxed file store + asset registry, the standalone-HTML inliner
   (`super_inline_html`), the dev-handoff generator, the design-system indexer
   (`check_design_system`), the parity matrix, and a dependency-light localhost
   preview server. Driven via the `jfc-design` CLI.
3. **Native tool surface + API layer (MOSTLY DONE).** First-class JFC
   `ToolKind`/`ToolInput` dispatch is wired for the native core tools, and
   `jfc-design-server` exposes the Axum routes for projects, file IO, preview
   serving, SSE events, bundling, handoff, design-system checks, browser-host
   tools, direct edit, signed public shares, Tweaks, `.dc.html` writes, and local
   media generation.
4. **Browser-host + frontend (MOSTLY DONE).** The React/Vite workspace in
   `apps/design-web` now covers projects, file IO, source editing, preview iframe,
   asset registry, bundling, design-system checks, handoff, capabilities, eval,
   screenshots, multi-screenshots, editable/screenshot PPTX, verifier checks,
   direct-edit inspect/apply, downloads, signed public-token files, Tweaks state,
   `.dc.html` streaming writes, PDF/print, verifier orchestration, chat, and
   local/external-hook generated image/sound assets. Remaining differences are
   operational rather than local-runtime gaps: deploying a hosted multi-user share
   service, providing real external media credentials, and matching any private
   Claude service policy that is not present in the extracted runtime.

## Status matrix

<!-- generated: jfc-design capabilities --md -->

| Feature | JFC surface | Status |
|---|---|---|
| base design-agent prompt | .claude/agents/designer.md | ✅ done |
| skill registry (deck, hi-fi, wireframe, prototype, animation, mobile, …) | .claude/skills/jfc-design/* | ✅ done |
| starter components (deck_stage, tweaks_panel, animations, frames, canvas, image_slot) | .claude/skills/jfc-design/starters/* | ✅ done |
| design projects + sandboxed file store + asset registry | jfc-design::project + DesignProject*/Design*File/Design*Asset tools + API | ✅ done |
| super_inline_html (standalone offline HTML) | jfc-design::inline + `jfc-design bundle` + DesignBundleHtml + API | ✅ done |
| handoff package (Handoff to Claude Code) | jfc-design::handoff + `jfc-design handoff` + DesignHandoff + API | ✅ done |
| design-system indexer / check_design_system (discovery + validation) | jfc-design::design_system + `jfc-design ds` + DesignCheckSystem + API | ✅ done |
| save_as_pdf (print-ready HTML) | design server /tools/save-pdf + Playwright PDF + browser print shell | ✅ done |
| read_file / write_file / list_files / copy_files / view_image | DesignReadFile/DesignWriteFile/DesignListFiles/DesignCopyFile + existing JFC tools | ✅ done |
| web_fetch / web_search | JFC web tools | 🔗 via existing |
| questions_v2 (ask the user) | JFC AskUserQuestion tool | 🔗 via existing |
| invoke_skill / tool_search | JFC Skill tool + progressive tool disclosure | 🔗 via existing |
| GitHub import (github_get_tree / import_files) | JFC GitHub CLI + MCP | 🔗 via existing |
| Figma MCP (connect_figma, get_design_context, generate_figma_design) | Figma MCP server + send-to-figma skill | 🔗 via existing |
| Canva import (create-design-import-job) | Canva MCP server + send-to-canva skill | 🔗 via existing |
| local preview / serve project files | jfc-design::server + `jfc-design serve` + DesignServe + API static routes | ✅ done |
| show_to_user / show_html (live preview pane) | design server iframe host + frontend iframe + injected __om/__DM preview bridge | ✅ done |
| get_webview_logs / eval_js_user_view (browser console + JS eval) | design server /tools/eval-js + Playwright host | ✅ done |
| save_screenshot / multi_screenshot | design server /tools/screenshot + /tools/multi-screenshot + Playwright host | ✅ done |
| done / fork_verifier_agent | design server /tools/verify + /tools/verify-orchestrate + /tools/done multi-viewport reports | ✅ done |
| gen_pptx (editable + screenshot PPTX export) | design server /tools/gen-pptx + Playwright DOM reconstruction + screenshot fallback | ✅ done |
| get_public_file_url | signed expiring share tokens + scoped /design/public/:token host + share manifest/revoke + optional API token auth | ✅ done |
| present_fs_item_for_download / open_for_print | design server download endpoint + browser print path + save-pdf | ✅ done |
| Tweaks transport (postMessage + on-disk persistence) | preview runtime postMessage bridge + design server tweaks persistence | ✅ done |
| direct edit / __om-edit-overrides (DOM→source mapping) | design server inspect/apply endpoints + static HTML, Source Map v3, unique project-text rewrites + overlay fallback | ✅ done |
| Design Components (.dc.html streaming runtime + dc_write) | preview runtime __dcUpdate/dc_html_str_replace bridge + dc_write + dc-stream SSE | ✅ done |
| generate_image (Gemini) / generate_sound (ElevenLabs) | jfc-design::media local provider + external provider command hooks + API/frontend controls | ✅ done |
| React/Vite project workspace frontend (projects, file tree, editor, preview, exports, chat) | apps/design-web | ✅ done |

## Phase plan for the remaining work

### Phase A — Design server (Axum, async)

Status: **implemented for local parity / deployment policy separate.**
`jfc-design-server` is feature-gated behind `server-api` and provides the stable
HTTP shape for the frontend. Native handlers are live for project/file/asset APIs,
static preview serving with preview-runtime injection, SSE events,
`super_inline_html`, handoff, design-system checks, browser-host calls,
direct-edit static/Source Map v3/unique-text source rewrites and overlay fallback,
Tweaks persistence, `.dc.html` streaming writes, downloads, print/PDF, signed
scoped public tokens with manifest/revoke, optional API-token auth, chat history,
and local/external-hook media assets. Production multi-user account management and
remote hosting remain deployment work.

Promote `jfc-design::server` from the std static slice to a full async service. A new
`jfc-design-server` binary (or feature) on Axum:

```
GET   /design/projects                      list
POST  /design/projects                      create
GET   /design/projects/:id                  metadata
GET   /design/projects/:id/files            file tree
GET   /design/projects/:id/file?path=       read
PUT   /design/projects/:id/file?path=       write
GET   /design/projects/:id/serve/*path      static preview (relative assets resolve)
GET   /design/projects/:id/events           SSE: file changes, asset registry, status
POST  /design/projects/:id/tools/eval-js    run JS in the preview, return result/logs
POST  /design/projects/:id/tools/screenshot capture PNG
POST  /design/projects/:id/tools/multi-screenshot capture selector/page PNG set
POST  /design/projects/:id/tools/gen-pptx   capture → .pptx (editable | screenshots)
POST  /design/projects/:id/tools/save-pdf   capture → .pdf
POST  /design/projects/:id/tools/direct-edit-inspect
POST  /design/projects/:id/tools/direct-edit-apply
POST  /design/projects/:id/tools/verify
POST  /design/projects/:id/tools/verify-orchestrate
POST  /design/projects/:id/tools/done
POST  /design/projects/:id/tools/super-inline-html   bundle (already native)
PUT   /design/projects/:id/tools/direct-edit-overrides
PUT   /design/projects/:id/tools/tweaks
POST  /design/projects/:id/tools/dc-write
GET   /design/projects/:id/tools/dc-stream
GET   /design/projects/:id/tools/chat
POST  /design/projects/:id/tools/chat
POST  /design/projects/:id/tools/generate-image
POST  /design/projects/:id/tools/generate-sound
GET   /design/projects/:id/download?path=
GET   /design/projects/:id/print?path=
POST  /design/projects/:id/public-token
GET   /design/projects/:id/public-shares
DELETE /design/projects/:id/public-shares/:token
GET   /design/public/:token/*path           signed scoped public file URL
```

The `project`/`inline`/`handoff`/`design_system` modules are the handlers' core.
The localhost shell exists now; remote multi-user hosting policy remains separate.

### Phase B — Browser host (headless Chromium)

Status: **implemented / Playwright host landed.** The server calls
`apps/design-web/scripts/browser-host.mjs` as a JSON subprocess. The host uses
Playwright Chromium for JS eval/log capture, screenshots, multi-screenshots,
direct-edit inspection, and verifier checks. It uses `pptxgenjs` for editable DOM
reconstruction with screenshot fallback. `JFC_DESIGN_BROWSER_HOST` can override the
script path, and `JFC_DESIGN_NODE` can override the Node binary.

- **eval-js / webview-logs:** live via `/tools/eval-js`, including console and page
  errors.
- **screenshot/multi-screenshot:** live via `/tools/screenshot` and
  `/tools/multi-screenshot` for full-page, selector, and slide-set captures.
- **gen-pptx:** live via `/tools/gen-pptx` as editable DOM reconstruction with
  screenshot fallback.
- **direct inspect / verifier:** live via `/tools/direct-edit-inspect`,
  `/tools/verify`, `/tools/verify-orchestrate`, and `/tools/done`, returning
  selectors, source hints, screenshots, browser errors, duplicate-slide warnings,
  failed-image checks, layout overflow checks, and multi-viewport reports.
- **print/PDF:** live via `/tools/save-pdf` and `/print`.
- **preview bridge:** HTML previews inject a Claude-compatible `__om`/`__DM` bridge
  with `__OM_EVT__`, selector scoring, ref registry attributes, hover/selected
  overlays, edit/comment/structural modes, serialization cleanup, Tweaks,
  `dc_html_str_replace`, and dc-write/stream messages; the Playwright host also
  injects a matching init script for file-backed captures.
- **window.claude:** inject the helper for `claude-in-prototypes` and proxy to the
  provider with the fast model + token cap.

### Phase C — Frontend (`apps/design-web`, React + Vite)

Status: **implemented / workspace shell landed.** The app talks to the Phase-A API and
ships project list/create, project metadata editing, file create/read/write/delete,
source editing, static preview iframe (`show_to_user` substrate), asset registry,
`super_inline_html`, design-system checks, handoff, browser eval, screenshots,
multi-screenshots, editable/screenshot PPTX export, verifier checks,
download/signed-public links, direct-edit inspect/apply, direct-edit override
persistence, Tweaks persistence, `.dc.html` writes, local media generation,
capabilities, chat, PDF/print, share management, and SSE status.

Remaining frontend work is deployment-specific: production account UI and hosted
share administration. React/Vite is the right tool here — the hard parts
(iframe, postMessage, canvas/image/PPTX, direct edit) are browser-native and
awkward in Rust UI frameworks.

### Phase D — Design Components runtime (.dc.html)

Status: **implemented locally.** The server can persist `.dc.html` content via
`/tools/dc-write`, emits structured SSE write metadata including `name`, `kind`,
`streaming`, `bytes`, `appended`, and stream content, exposes `/tools/dc-stream`,
and the preview bridge exposes `window.__dcUpdate(...)`,
`window.dc_html_str_replace(...)`, `window.__dcAppend(...)`, and
`window.__dcReplace(...)`.

### Phase E — Native tool surface

Status: **native core tools landed.** JFC now has first-class `ToolKind`/`ToolInput`
variants, schemas, dispatch, permissions, scheduling, serialization, and changeset
details for project creation/listing/metadata, file read/write/delete/copy, asset
register/unregister, bundling, handoff, design-system checks, capabilities, and
preview serving.

Remaining native work is deployment-specific: provider credentials, production
account policy, and remote share hosting. Local host/runtime parity is implemented.

## Reference material

- Claude Design extracted prompts: `/home/cole/VulnerabilityResearch/claude-design/extracted-prompts/`
- Figma SDK (cloned): `/tmp/design-sources/plugin-typings`, `/tmp/design-sources/rest-api-spec`
- Canva SDK (cloned): `/tmp/design-sources/canva-apps-sdk-starter-kit`, `/tmp/design-sources/canva-connect-api-starter-kit`
  (Figma and Canva are proprietary — there is no app source to clone; these are the
  official public SDK/API surfaces the send-to-figma / send-to-canva skills target.)
- Mindemon runtime dumps copied from `~/Downloads/` to `/tmp/claude-design/`;
  beautified JS lives in `/tmp/claude-design/pretty/`, and CodeGraph is initialized
  for `/tmp/claude-design`.
