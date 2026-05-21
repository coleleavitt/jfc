# AI Studio / Gemini API — Complete Protocol Analysis

## Source: Burp Suite capture of `aistudio.google.com` + JS bundle deobfuscation

---

## 1. Two Distinct API Surfaces

### A. Public REST API (`generativelanguage.googleapis.com`)
- **Auth**: `?key=<GEMINI_API_KEY>` query param
- **Base**: `https://generativelanguage.googleapis.com/v1beta`
- **Endpoints**:
  - `GET /models` — list available models
  - `POST /models/{model}:generateContent` — non-streaming
  - `POST /models/{model}:streamGenerateContent?alt=sse` — SSE streaming
  - `POST /models/{model}:countTokens` — token counting
  - `POST /models/{model}:batchEmbedContents` — batch embeddings
  - `POST /models/{model}:chatCompletions` — OpenAI-compatible endpoint!
  - `POST /cachedContents` — context caching
  - `POST /files` — file upload for multimodal
  - `GET /openai/chat/completions` — OpenAI compat proxy
  - `GET /openai/embeddings` — OpenAI compat proxy
  - `POST /models/{model}:generateContentPerUserQuota` — quota-aware generation

### B. Internal gRPC-Web API (`alkalimakersuite-pa.clients6.google.com`)
- **Auth**: `X-Goog-Api-Key: AIzaSyDdP816MREB3SkjZO04QXbjsigfcI0GWOs` (AI Studio's built-in key) + Google session cookies (SAPISID auth)
- **Content-Type**: `application/json+protobuf`
- **Service**: `google.internal.alkali.applications.makersuite.v1.MakerSuiteService`
- **Body format**: JSON-encoded protobuf arrays (positional fields, `null` for empty)

---

## 2. AI Studio Internal API Key
AI Studio uses its OWN API key (not yours): `AIzaSyDdP816MREB3SkjZO04QXbjsigfcI0GWOs`
- This is hardcoded in the JS bundle
- Combined with your Google session cookies for auth
- The `X-Goog-Authuser: 0` header selects which Google account

---

## 3. Full RPC Method Catalogue (28 observed in Burp + 70+ from JS deob)

### Core Generation
- `GenerateContent` — main inference (server streaming)
- `StreamCodeAssistantOfflineGeneration` — server-streaming code assistant
- `CodeAssistantOffline` — batch code assistant
- `CountTokens` — token counting
- `GenerateTitle` — auto-title for conversations
- `GenerateImage` — image generation
- `GenerateVideo` — video generation

### Auth & Account
- `GenerateAccessToken` — body: `["users/me"]` → returns ephemeral access token
- `CheckUserStatus` — body: `[]`
- `GetAiStudioBenefitTier` — body: `[]` → returns tier (free/paid)
- `GetUserPreferences` — body: `[]`
- `GetUserRestrictions` — body: `[]`
- `AcceptTerms`
- `AcceptFirebaseTos`

### Models & Quota
- `ListModels` — body: `[]` → full model catalogue
- `ListQuotaModels` — body: `[]`
- `ListModelRateLimits` — body: `["projects/<project_id>"]`
- `GetModelQuota`

### Projects & Keys
- `ListCloudProjects` — body: `[null,null,null,1,null,["projects/<project_id>"]]`
- `ListCloudApiKeys` — body: `[100,null,1,["projects/<project_id>"]]`
- `ListImportedProjects` — body: `[]`
- `ListBillingAccounts` — body: `[]`
- `GetPrepayEligibility` — body: `["billingAccounts/<id>"]`
- `RemoveProjects`

### Conversation & Prompts
- `ListSessionTurns` — body: `["projects/<id>",25,null,null,"<api_key>","<cursor>"]`
- `ListPrompts` — body: `[100]`
- `CreatePrompt` — full conversation save
- `BulkDeleteSessionTurns`
- `RecordSessionTurnFeedback`

### Applets (AI Studio Apps)
- `ListApplets`
- `ListRecentApplets`
- `CreateApplet`
- `SaveApplet`
- `DeleteApplet`
- `UpdateApplet`
- `RemixApplet`
- `ForkApplet`
- `GetAppFolder`
- `GetAppletGalleryConfig`
- `UpdateAppletAccess`
- `UpdateAppletChatHistorySharing`
- `CreateSharedAppletDeployment`
- `DeleteSharedAppletDeployment`
- `ListSharedApplets`

### Code Assistant (Canvas)
- `LoadCodeAssistantInteractionHistory`
- `LoadCodeAssistantSnapshots`
- `StreamCodeAssistantOfflineGeneration` — **server streaming**
- `CodeAssistantOffline`
- `CancelCodeAssistantOfflineGeneration`
- `ListCodeAssistantFeatures`

### GitHub Integration
- `ListGitHubRepositories`
- `ImportGitHubRepository`
- `CreateGitHubRepository`
- `PullGitHubChanges`
- `PushNewCommit`
- `ComputeStagedGitHubDiff`
- `GenerateGitHubCommitMessage`

### Datasets & Logging
- `ListDatasets` — body: `["projects/<id>",null,null,null,"<api_key>"]`
- `CreateDataset`
- `UpdateDataset`
- `DeleteDataset`
- `ExportDataset`
- `GetTracesLoggingStatus` — body: `["projects/<id>","<api_key>"]`
- `EnableTracesLogging`
- `DisableTracesLogging`
- `ToggleInteractionsLogging`
- `UpdateTracesPreset`
- `GetLoggingContext`

### Metrics
- `FetchMetricTimeSeries` — body: `[null,null,null,null,3,null,<period>,<project>,<offset>,[20],[<metric_ids>]]`

### Cloud Deployment
- `CreateCloudRunService`
- `DeleteCloudRunService`
- `ListCloudRunServices`
- `UpdateCloudRunService`

### Figma
- `GetFigmaAuthStatus`
- `GetFigmaFileMetadata`
- `DisconnectFigmaAccount`

### Billing
- `GetProjectUsageLimit`
- `UpdateProjectUsageLimit`
- `UpgradeAndDisablePrepay`

### OAuth (for deployed apps)
- `CreateOAuthClient`
- `AddBrandTestUser`
- `GetOAuthBrand`

### Other Services (from JS)
- `google.alkali.boq.makersuite.cloudsql.proto.CloudSqlService/UpdateSchema`
- `google.alkali.boq.makersuite.makersuiteappletcontrol.proto.MakersuiteAppletControlService/*`
- `google.internal.cloud.clientapi.sdui.BillingSduiService/GetSdui`

---

## 4. GenerateContent Request Body Format (json+protobuf)

```
["models/gemini-flash-latest",                     // [0] model name
 [[[[null,"user message"]],"user"]],               // [1] contents array
 [[null,null,7,4],[null,null,8,4],...],             // [2] safety settings
 [null,null,null,65536,1,0.95,64,null,null,null,   // [3] generation config:
  null,null,null,1,null,null,[1,null,null,3]],      //     maxTokens, topK, temp, topP, ...thinkingConfig
 "!session_token_base64...",                        // [4] session/auth token
 null,                                             // [5]
 null,                                             // [6]
 null,                                             // [7]
 null,                                             // [8] system instruction?
 1,                                                // [9] streaming flag?
 null,                                             // [10]
 null,                                             // [11]
 null,                                             // [12]
 1                                                 // [13]
]
```

## 5. CountTokens Request Body Format

```
["models/gemini-flash-latest",[[[[null,"message text"]],"user"]]]
```
(model name + contents in same positional format)

---

## 6. Key Headers for Internal API

```
Authorization: SAPISIDHASH <timestamp>_<hash> SAPISID1PHASH ... SAPISID3PHASH ...
Content-Type: application/json+protobuf
X-Goog-Api-Key: AIzaSyDdP816MREB3SkjZO04QXbjsigfcI0GWOs
X-Goog-Authuser: 0
X-Aistudio-Visit-Id: <uuid>
X-User-Agent: grpc-web-javascript/0.1
X-Goog-Ext-519733851-Bin: <binary proto for request context>
```

---

## 7. OpenAI-Compatible Endpoints (Public API)

The public API also exposes OpenAI-compatible endpoints:
- `POST /v1beta/openai/chat/completions`
- `POST /v1beta/openai/embeddings`
- `POST /v1beta/openai/images/generations`
- `POST /v1beta/models/{model}:chatCompletions`

These accept OpenAI's request format and can be used with any OpenAI-compatible client!

---

## 8. Additional Endpoints from JS

- `BidiGenerateContent` — bidirectional streaming (WebSocket/gRPC)
- `BidiGenerateMusic` — bidirectional music generation
- `:predict` / `:predictLongRunning` — Vertex AI style endpoints
- `/embeddings:generate` — embeddings

---

## 9. Models Available (via public API with your key)

### Text Generation (1M context):
- gemini-3.5-flash (65K output)
- gemini-3.1-pro-preview (65K output)
- gemini-3.1-pro-preview-customtools (65K output)
- gemini-3.1-flash-lite / gemini-3.1-flash-lite-preview (65K output)
- gemini-3-pro-preview / gemini-3-flash-preview (65K output)
- gemini-2.5-pro / gemini-2.5-flash / gemini-2.5-flash-lite (65K output)
- gemini-2.0-flash / gemini-2.0-flash-lite (8K output)
- gemini-pro-latest / gemini-flash-latest / gemini-flash-lite-latest (65K aliases)

### Special Models:
- antigravity-preview-05-2026 (131K ctx, 65K out) — Antigravity Agent
- deep-research-max-preview-04-2026 (131K ctx, 65K out)
- deep-research-preview-04-2026 / deep-research-pro-preview-12-2025
- gemini-2.5-computer-use-preview-10-2025 (131K ctx)
- gemini-robotics-er-1.5-preview / gemini-robotics-er-1.6-preview

### Image/Video:
- imagen-4.0-generate-001 / imagen-4.0-ultra / imagen-4.0-fast
- veo-2.0 / veo-3.0 / veo-3.0-fast / veo-3.1 / veo-3.1-fast / veo-3.1-lite
- gemini-2.5-flash-image / gemini-3-pro-image-preview / gemini-3.1-flash-image-preview

### Audio:
- gemini-2.5-flash-preview-tts / gemini-2.5-pro-preview-tts
- gemini-2.5-flash-native-audio-*
- gemini-3.1-flash-tts-preview
- lyria-3-clip-preview / lyria-3-pro-preview

### Open Models:
- gemma-4-26b-a4b-it / gemma-4-31b-it (262K ctx)

---

## 10. API Key Analysis

### Found Keys in JS Bundles:
1. `AIzaSyDdP816MREB3SkjZO04QXbjsigfcI0GWOs` — AI Studio's primary internal key (used for alkali RPC calls)
2. `AIzaSyBGb5fGAyC-pRcRU6MUHb__b_vKha71HRE` — Feedback/survey service key
3. `AIzaSyCB6OnnfuitFnaYWu4BvtGKaoLFk4cm-GE` — Feedback service alt key

### Internal Key Behavior:
- **alkali API**: Requires OAuth2 access token (SAPISIDHASH cookies). API key alone is insufficient.
- **Public API** (`generativelanguage.googleapis.com`): The internal key is **referrer-locked** to `aistudio.google.com`. Even with the correct Referer header, Google blocks it for programmatic use ("Requests to this API ... are blocked").
- **Conclusion**: The internal key is useless outside the browser. AI Studio's internal API requires a full Google session (cookies + SAPISIDHASH auth). There is NO way to bypass this with just API keys.

### What DOES Work:
- Your personal `GEMINI_API_KEY` works against the public `generativelanguage.googleapis.com` API with NO referrer restrictions.
- Your key has access to **50 models** (the full catalogue).
- The internal key, even with correct headers, only returns 1 model (blocked).

---

## 11. Internal Corp URLs Found (Not Publicly Accessible)

- `https://omnidda-staging.corp.google.com/dda` — DDA staging
- `https://pantheon-testgaia.corp.google.com` — Test auth
- `https://protoshop.corp.google.com/embed` — Internal prototyping tool
- `https://protoshop-dev.corp.google.com/embed` — Dev version
- `https://uberproxy-pen-redirect.corp.google.com/uberproxy/pen?url=` — Internal proxy
- `https://billing-ads-qa-devel.corp.google.com/payments/v4/js/integrator.js?ss=md` — Billing QA

---

## 12. Google Workspace Scopes (for Applets/Extensions)

AI Studio applets can request access to:
- Google Calendar (read/write)
- Google Chat (messages, spaces)
- Google Contacts
- Google Docs (read/write)
- Google Drive (full access, metadata, file, scripts)
- Firebase
- Google Forms
- Gmail (compose, send, read, settings)
- Google Keep
- Google Meet
- Google Sheets
- Google Tasks

---

## 13. Googlesource Integration

AI Studio has direct integration with `*.googlesource.com` for code browsing:
- `https://{host}.googlesource.com/{repo}/+show/{ref}/{path}?format=JSON`
- `https://{host}.googlesource.com/{repo}/+show/{ref}/{path}?format=TEXT`

This powers the GitHub integration features in AI Studio's code canvas.
