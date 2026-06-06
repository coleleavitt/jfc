# JFC Streaming/Messages Path: From User Enter → API Call → UI Error

## 1. ANTHROPIC MESSAGES API STREAMING CALL (POST /v1/messages with stream=true)

### Location: User Presses Enter → API Call

**File:line** | **Function** | **Details**
---|---|---
`crates/jfc/src/stream.rs:1457` | `stream_response()` | Opens initial stream with `open_stream_with_bedrock_retries(provider.as_ref(), messages, &opts)`
`crates/jfc/src/stream.rs:1057` | `open_stream_with_bedrock_retries()` | Wrapper that retries on Bedrock transient 400s
`crates/jfc/src/providers/anthropic.rs:192` | `AnthropicProvider::stream()` | Main streaming entry point
`crates/jfc/src/providers/anthropic.rs:200-210` | `send_with_retry("anthropic.messages", \|\|...)` | **THE RETRY WRAPPER** that calls:
`crates/jfc/src/providers/anthropic.rs:202` | `client.post(API_URL)` | Posts to `https://api.anthropic.com/v1/messages`
`crates/jfc/src/providers/anthropic.rs:207` | `.json(&body)` | Body includes `"stream": true`
`crates/jfc-anthropic-sdk/src/messages.rs:171` | `client.request(Method::POST, "/v1/messages", None)` | SDK-level POST (non-streaming path uses `execute_with_retry()`)

**Critical: Streaming respects HTTP-level retry in `send_with_retry()` BEFORE establishing the stream.**

---

## 2. RETRY WRAPPER: send_with_retry()

**File:line** | **Details**
---|---
`crates/jfc/src/providers/http.rs:47-102` | `pub async fn send_with_retry<F, Fut>()` — retries on **connection-level errors only** (DNS, TLS, timeouts)
`crates/jfc/src/providers/http.rs:71` | Calls `super::retry::is_retriable_error(&e)` to check error type
`crates/jfc/src/providers/retry.rs:76-78` | `pub fn is_retriable_error(err: &reqwest::Error) -> bool` — checks `.is_connect() \|\| .is_timeout() \|\| .is_request()`

### Retry Strategy
- **Max Retries:** `RetryConfig::default().max_retries = 2` (3 total attempts)
- **Backoff:** `min(0.5 * 2^attempt, 8.0) * (1 - random*0.25)` seconds (exponential with jitter)
- **Apply:** Only for network errors, **NOT** for HTTP status codes (429, 5xx handled by provider)
- **Logs:** `crates/jfc/src/providers/http.rs:74-82` — trace warnings on retry with attempt count, delay_ms, cause

**IMPORTANT: This does NOT retry on 429 or 5xx — those are passed to the provider layer.**

---

## 3. ERROR HANDLING: 429, 529, 5xx

### Where They're Handled

**File:line** | **Pattern** | **Handler**
---|---|---
`crates/jfc/src/providers/retry.rs:72` | `matches!(status, 408 \| 409 \| 425 \| 429 \| 500..=599)` | `should_retry_status()` — determines if status is retriable
`crates/jfc/src/providers/anthropic.rs:244-293` | After `send_with_retry()` succeeds, checks `!status.is_success()` | Provider-level error handler: maps status → friendly message
`crates/jfc/src/providers/anthropic.rs:276` | `Some("rate_limit_error")` | Checks error body's `error.type == "rate_limit_error"`
`crates/jfc/src/providers/anthropic.rs:277` | **Bails:** `"Rate limited — wait a moment and retry. {friendly}"` | Propagates to caller as `anyhow::Result::Err()`
`crates/jfc/src/providers/retry.rs:124-130` | `429 => { ... "Rate limited — too many requests..." }` | `friendly_error_message()` composes user-friendly error text

### All 429/529/5xx Matches

**File:line** | **Code** | **Meaning**
---|---|---
`crates/jfc-anthropic-sdk/src/error.rs:44` | `matches!(code, 408 \| 409 \| 425 \| 429) \|\| code >= 500` | SDK-level retry check (non-UI path)
`crates/jfc-anthropic-sdk/src/retry.rs:79` | Same as above | SDK's `should_retry_status()`
`crates/jfc/src/providers/retry.rs:54,72` | `408 \| 409 \| 425 \| 429 \| 500..=599` | JFC UI retry policy (matches SDK)
`crates/jfc/src/providers/retry.rs:113-145` | Match on 408, 429, 503, 504, 520-526, 529 | Per-status friendly messages
`crates/jfc/src/providers/anthropic.rs:276` | `Some("rate_limit_error")` | Anthropic API semantic type check
`crates/jfc/src/github/client.rs:210` | `429 rate limit` → `GhError::RateLimited` | GitHub-specific 429 handling (separate system)

---

## 4. StreamError & RateLimit* VARIANTS

### StreamError Construction

**File:line** | **Event Type** | **Text Content**
---|---|---
`crates/jfc/src/app.rs:36` | `AppEvent::StreamError(String)` | Carries error message to UI
`crates/jfc/src/stream.rs:1521` | `AppEvent::StreamError(e.to_string())` | From stream open failure (line 1520)
`crates/jfc/src/stream.rs:1569` | `AppEvent::StreamError(e.to_string())` | From streaming event parse error
`crates/jfc/src/stream.rs:1516` | `AppEvent::StreamError(format!("auto-compact: {e}"))` | Special prefix for prompt-too-long

**These are NOT typed as RateLimit variants — all errors use the generic String variant.**

### RateLimit* Variants in Codebase

**File:line** | **Variant** | **Usage**
---|---|---
`crates/jfc-anthropic-sdk/src/error.rs:29` | `Error::RateLimited { retry_after_ms: u64 }` | SDK error enum (non-UI)
`crates/jfc/src/github/client.rs:35` | `GhError::RateLimited { reminder: String }` | GitHub API only
`crates/jfc/src/providers/anthropic_oauth.rs:647` | `RotationDecision::RateLimited { retry_after_secs: Option<u64> }` | Account rotation on 429
`crates/jfc/src/providers/anthropic_oauth.rs:657-658` | Returned when status 429 + no retry-after | Multi-account fallback triggers

**Key insight: Anthropic provider errors DON'T use RateLimit variants — they bail with String messages.**

---

## 5. ERROR PROPAGATION INTO TUI

### AppEvent::StreamError Handler

**File:line** | **Code** | **Effect**
---|---|---
`crates/jfc/src/event_loop.rs:2067-2176` | `AppEvent::StreamError(e) => { ... }` | Main error handler in event loop
`crates/jfc/src/event_loop.rs:2141-2143` | `app.messages.push(ChatMessage::assistant(format!("**Error:** {e}\n\n_Press Ctrl+R to retry the last prompt._")))` | **Renders error as assistant message**
`crates/jfc/src/event_loop.rs:2153-2159` | `toast::push_with_cap(&mut app.toasts, toast::Toast::new(toast::ToastKind::Error, format!("Stream error: {preview}")))` | **Also surfaces as Error toast**
`crates/jfc/src/render.rs:3784` | Renders help text: `("Ctrl+R", "retry last prompt")` | UI hint for retry

### Message Rendering

**File:line** | **Code** | **Details**
---|---|---
`crates/jfc/src/render.rs:292` | `if !app.toasts.is_empty() { ... }` | Toast strip is rendered if queue non-empty
`crates/jfc/src/render.rs:3380-3493` | Full toast rendering: width, height, slide-in animation, color per kind | Transient warning strip (right side, expires in 8s for Error)
`crates/jfc/src/message_view.rs` | Renders `app.messages` transcript | Error message appears as assistant turn with markdown formatting

### "Press Ctrl+R to retry" String

**File:line** | **String**
---|---
`crates/jfc/src/event_loop.rs:2142` | `"_Press Ctrl+R to retry the last prompt._"` (rendered in error message)
`crates/jfc/src/render.rs:3784` | `("Ctrl+R", "retry last prompt")` (help text in footer)
`crates/jfc/src/input.rs:6838` | Comment: `// Ctrl+R retry`

---

## 6. TOAST / BANNER / NOTIFICATION SYSTEM

### Toast Infrastructure

**File:line** | **Component** | **Purpose**
---|---|---
`crates/jfc/src/toast.rs:1-183` | Pure data model + lifecycle | Toast struct, TTL per kind, expiry logic
`crates/jfc/src/toast.rs:12-18` | `pub enum ToastKind { Info, Success, Warning, Error }` | Four severity levels
`crates/jfc/src/toast.rs:21-45` | `pub struct Toast { kind, text, created_at, ttl }` | TTL defaults: Error 8s, Warning 6s, Info 4s, Success 3s
`crates/jfc/src/app.rs:84-87` | `AppEvent::Toast { kind, text }` | Event to push a toast
`crates/jfc/src/event_loop.rs:2084-2090` | Auto-compact signal sends `Toast::Warning("Auto-compacting...")` | Example: non-fatal recovery
`crates/jfc/src/event_loop.rs:2153-2159` | Stream error sends `Toast::Error("Stream error: ...")` | Example: fatal error

### Toast Rendering

**File:line** | **Code** | **Effect**
---|---|---
`crates/jfc/src/render.rs:3375-3494` | `render_toast_strip()` | Slide-in animation (200ms ease-out-cubic), max 5 toasts, right-aligned strip
`crates/jfc/src/render.rs:3439-3455` | Border color tracks highest severity | Error→red, Warning→yellow, Success→green, Info→default
`crates/jfc/src/render.rs:3471-3476` | Icon per kind: "ℹ", "✓", "⚠", "✘" | Visual severity cue
`crates/jfc/src/render.rs:3478-3488` | Text truncated to 60 chars max, 120 chars for preview | Readable in tight space
`crates/jfc/src/event_loop.rs:240` | `tx_guard.send(AppEvent::StreamError(msg))` at error point | Only path that sends Toast::Error

### Transient vs. Fatal Toast

**Kind** | **TTL** | **Recovery** | **Use Case**
---|---|---|---
`Toast::Warning` | 6s | User can retry/continue | Auto-compact in progress, network hiccup
`Toast::Error` | 8s | User must act (Ctrl+R or new prompt) | API error, auth failure, rate limit

**GAP: No countdown toast "Rate limited — retrying in Xs · attempt N/M" yet.** Toast is one-shot; no internal timer display.

---

## 7. MODEL SELECTION & FALLBACK

### Model Selection Logic

**File:line** | **Code** | **Details**
---|---|---
`crates/jfc/src/providers/anthropic.rs:81` | `"model": opts.model` | Directly uses StreamOptions.model (caller-selected)
`crates/jfc/src/providers/anthropic.rs:167-179` | `fetch_models()` calls `models_dev::fetch_provider_models()` | Live catalog from models.dev, falls back to embedded list
`crates/jfc/src/providers/anthropic_models.rs` | Static `anthropic_first_party_models()` | Embedded catalog includes opus-4-7, sonnet-4-6, etc.
`crates/jfc/src/slate.rs:208-231` | `pub fallback_model: Option<String>` | Per-provider fallback (experiment arm for A/B tests)
`crates/jfc/src/types.rs` | Model selection UI (picker) | User selects via `Ctrl+M`

**CRITICAL: No mid-flight model swap on 429/5xx.** If the selected model fails, user must manually retry with a different model via Ctrl+M.

### Model Fallback on 404

**File:line** | **Code** | **Effect**
---|---|---
`crates/jfc/src/providers/anthropic_oauth.rs:183-200` | `pub(crate) fn parse_model_not_found(body: &str) -> Option<String>` | Extracts model name from 404 body
`crates/jfc/src/providers/anthropic.rs:252-256` | If model 404, bail: `"{model} is not enabled on your Anthropic account..."` | User-facing error directing to Ctrl+M
`crates/jfc/src/providers/anthropic_oauth.rs:1042-1048` | Checks model availability; if not available in account rotation, try next account | Account-level fallback (not model fallback)

**There is NO fallback chain like "try opus, else sonnet" — explicit user selection only.**

---

## 8. tokio::time::sleep / tokio::time::timeout USES IN PROVIDERS

### sleep() calls (backoff / delay)

**File:line** | **Purpose** | **Duration / Formula**
---|---|---
`crates/jfc-anthropic-sdk/src/client.rs:132` | Retry backoff in SDK non-streaming path | `delay` from `retry::delay_for(attempt)`
`crates/jfc-anthropic-sdk/src/client.rs:147` | Retry backoff after transport error | Same
`crates/jfc/src/providers/http.rs:85` | Retry backoff in send_with_retry() | `config.delay_for_attempt(attempt)`
`crates/jfc/src/providers/retry.rs:178` | Same, called from `with_retry()` generic | `config.delay_for_attempt(attempt)`
`crates/jfc/src/providers/file_lock.rs:126` | Exponential backoff for file lock acquisition | `LOCK_INITIAL_RETRY_MS * factor` (10-200ms range)
`crates/jfc/src/stream.rs:1099` | Bedrock transient 400 retry backoff | `250ms * 2^attempt * jitter`
`crates/jfc/src/stream.rs:1557` | Stream interrupt polling loop | `STREAM_INTERRUPT_POLL = 50ms`
`crates/jfc/src/event_loop.rs:2108` | Delay before auto-compact requeue | 150ms

### timeout() calls (request deadlines)

**File:line** | **Purpose** | **Timeout**
---|---|---
`crates/jfc/src/providers/http.rs:16-21` | HTTP streaming client config | 600s read timeout (no hard deadline on streaming body)
`crates/jfc/src/stream.rs:3677-3688` | Wait for spawn handle with timeout | 500ms for join
`crates/jfc/src/stream.rs:3688` | Same | 500ms timeout_at on handle

### Backoff Formula Used in Providers

```
delay_ms = min(base * 2^attempt, max) * jitter
  where:
    base = 500ms (SDK) or 0.5s (UI)
    max = 8s (SDK) or 16s (UI aggressive)
    jitter = ±25% = 1.0 - (rand % 0.25)
```

**Applied at:**
1. Retry wrapper (send_with_retry) — connection errors only
2. Provider layer (should_retry_status) — status codes only
3. Bedrock wrapper (open_stream_with_bedrock_retries) — transient 400s only
4. Account rotation (anthropic_oauth) — 429 + account cooldown

---

## SUMMARY: CURRENT STATE

### (a) Retry-on-429 Path (Current)

1. **send_with_retry()** → retries **connection errors ONLY** (3 attempts, 0.5-8s backoff)
2. **Provider.stream()** → receives HTTP 429 response
3. **anthropic.rs** → checks error.type == "rate_limit_error"
4. **Bails** with user-facing message → "Rate limited — wait a moment and retry. {friendly}"
5. **Stream.rs** → catches anyhow::Err, sends **AppEvent::StreamError(msg)**
6. **Event loop** → renders error as assistant message + Error toast (8s TTL)
7. **User** → manually retries via Ctrl+R (re-submits last prompt)

**No automatic retry on 429 — user must manually re-engage.**

### (b) UI Surfacing of Rate Limit (Current)

1. **Message transcript:** `"**Error:** Rate limited — wait a moment and retry...\n\n_Press Ctrl+R to retry the last prompt._"`
2. **Toast strip:** Right-aligned, 8s TTL, red border (Error), text truncated to 120 chars
3. **Help footer:** Ctrl+R hint always visible when streaming error occurs

### (c) Gaps (Missing Features)

| Gap | Location | Impact |
|---|---|---|
| **No countdown toast** | toast.rs doesn't support animated timers | User can't see "retrying in 3s · attempt 2/5" |
| **No mid-flight retry** | anthropic.rs:276 bails immediately on 429 | User must manually Ctrl+R; no exponential backoff with status-code sleep |
| **No model fallback** | anthropic.rs has no fallback chain | 404 on "opus" → error, not fallback to "sonnet" |
| **No toast action buttons** | toast.rs only displays text | Can't "tap toast to retry" — must Ctrl+R separately |
| **No per-attempt logging in UI** | No AppEvent::RetryAttempt variant | User doesn't see "attempt 1/5" in the message view |
| **Rate-limit header parsing** | friendly_error_message() doesn't inspect retry-after headers | Toast shows generic "Rate limited", not "retry in 45s" |
| **Multi-account rotation on 429** | Works in OAuth path (anthropic_oauth.rs) but not main API path | Non-OAuth Anthropic provider doesn't rotate accounts on 429 |

---

## FILE REFERENCE: Complete Location Map

### Core Streaming
- **crates/jfc/src/stream.rs** — `open_stream_with_bedrock_retries()`, `stream_response()`, stream event loop
- **crates/jfc/src/providers/anthropic.rs** — `AnthropicProvider::stream()`, error mapping, 429 handling

### HTTP / Retry
- **crates/jfc/src/providers/http.rs** — `send_with_retry()`, connection-error retry, classify_send_error()
- **crates/jfc/src/providers/retry.rs** — `should_retry_status()`, `friendly_error_message()`, backoff formula

### SDK-Level (Non-Streaming)
- **crates/jfc-anthropic-sdk/src/client.rs** — `execute_with_retry()` 
- **crates/jfc-anthropic-sdk/src/retry.rs** — `should_retry_status()`, `delay_for()`, `parse_retry_after()`
- **crates/jfc-anthropic-sdk/src/messages.rs** — `MessageService::create()` (non-streaming only)

### UI / Event Loop
- **crates/jfc/src/event_loop.rs** — `AppEvent::StreamError` handler, toast push, message rendering
- **crates/jfc/src/app.rs** — `AppEvent` enum definition
- **crates/jfc/src/toast.rs** — Toast model, TTL, expiry logic
- **crates/jfc/src/render.rs** — `render_toast_strip()`, toast animation, color mapping

### Account Rotation (OAuth)
- **crates/jfc/src/providers/anthropic_oauth.rs** — `RotationDecision::RateLimited`, account fallback on 429
- **crates/jfc/src/providers/anthropic_accounts.rs** — Account cooldown management, retry-after parsing

### Models
- **crates/jfc/src/providers/anthropic_models.rs** — Embedded model catalog
- **crates/jfc/src/slate.rs** — fallback_model for A/B testing

