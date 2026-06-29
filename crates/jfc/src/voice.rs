//! Voice mode integration — bridges jfc-voice into the TUI.
//!
//! Lifecycle:
//! 1. `init()` reads voice config and starts the event forwarder.
//! 2. Key handlers call `activate(pressed)` for push-to-talk.
//! 3. The event loop consumes `VoiceEvent` from the engine bus.
//! 4. `EngineEvent::Voice(Final(text))` injects text into the textarea.

use std::sync::{Mutex as StdMutex, OnceLock};
use tokio::sync::Mutex;

use jfc_engine::runtime::{EngineEvent, EventSender, StreamEvent, VoiceEvent};
use jfc_voice::{VoiceConfig, VoiceRecorder, VoiceTranscriptEvent};

/// Process-global recorder handle.
static RECORDER: OnceLock<Mutex<VoiceRecorder>> = OnceLock::new();

/// Runtime read-aloud override toggled by `/voice readaloud on|off`. 0 = unset
/// (fall back to the `read_aloud` config value), 1 = forced on, 2 = forced off.
/// Lets the user flip TTS read-aloud live without editing config.toml or
/// restarting. The decision point ([`read_aloud_on`]) consults this first.
static READ_ALOUD_OVERRIDE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Force read-aloud on/off at runtime (`/voice readaloud on|off`).
pub fn set_read_aloud_override(on: bool) {
    READ_ALOUD_OVERRIDE.store(if on { 1 } else { 2 }, std::sync::atomic::Ordering::Relaxed);
}

/// Effective read-aloud state for a resolved config: the runtime override wins,
/// otherwise the `read_aloud` config value (default off, set in config.toml).
fn read_aloud_on(cfg: &VoiceConfig) -> bool {
    match READ_ALOUD_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => cfg.read_aloud,
    }
}

pub fn init(voice_value: Option<&serde_json::Value>, engine_tx: jfc_engine::runtime::EventSender) {
    let cfg = voice_value
        .map(|value| VoiceConfig::from_settings(Some(value)))
        .unwrap_or_else(current_config);
    init_with_config(cfg, engine_tx);
}

fn init_with_config(cfg: VoiceConfig, engine_tx: jfc_engine::runtime::EventSender) {
    if !cfg.enabled {
        tracing::debug!(target: "jfc::voice", "voice mode disabled (voice.enabled=false)");
        return;
    }

    tracing::info!(
        target: "jfc::voice",
        mode = %cfg.mode.label(),
        "voice mode initializing"
    );

    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<VoiceTranscriptEvent>();

    // Token provider: resolve the real Claude.ai OAuth access token from the
    // shared accounts store on demand, so the live Anthropic voice stream is
    // wired to the same auth the rest of the app uses (not a dead env var).
    let token_provider: jfc_voice::TokenProvider =
        std::sync::Arc::new(|| Box::pin(jfc_providers::current_access_token()));
    let recorder = VoiceRecorder::new(cfg, event_tx).with_token_provider(token_provider);

    if RECORDER.set(Mutex::new(recorder)).is_err() {
        tracing::warn!(target: "jfc::voice", "init called twice — ignoring");
        return;
    }

    // Forward transcript events → engine bus as VoiceEvent
    tokio::spawn(async move {
        while let Some(ev) = event_rx.recv().await {
            let engine_ev = match ev {
                VoiceTranscriptEvent::Interim(t) => EngineEvent::Voice(VoiceEvent::Interim(t)),
                VoiceTranscriptEvent::Final(t) => EngineEvent::Voice(VoiceEvent::Final(t)),
                VoiceTranscriptEvent::Level(l) => EngineEvent::Voice(VoiceEvent::Level(l)),
                VoiceTranscriptEvent::Error(e) => EngineEvent::Voice(VoiceEvent::Error(e)),
                VoiceTranscriptEvent::StateChanged(s) => {
                    EngineEvent::Voice(VoiceEvent::StateChanged(s as u8))
                }
                VoiceTranscriptEvent::AssistantMessageStarted => {
                    EngineEvent::Voice(VoiceEvent::AssistantMessageStarted)
                }
                VoiceTranscriptEvent::AssistantTextDelta(text) => {
                    EngineEvent::Stream(StreamEvent::Chunk {
                        text: Some(text),
                        reasoning: None,
                    })
                }
                VoiceTranscriptEvent::AssistantMessageCompleted => {
                    EngineEvent::Voice(VoiceEvent::AssistantResponseCompleted)
                }
                VoiceTranscriptEvent::ReadAloudStarted { chars } => {
                    EngineEvent::Voice(VoiceEvent::ReadAloudStarted { chars })
                }
                VoiceTranscriptEvent::ReadAloudCompleted {
                    audio_bytes,
                    chunks_sent,
                } => EngineEvent::Voice(VoiceEvent::ReadAloudCompleted {
                    audio_bytes,
                    chunks_sent,
                }),
                VoiceTranscriptEvent::ReadAloudError(msg) => {
                    EngineEvent::Voice(VoiceEvent::ReadAloudError(msg))
                }
                VoiceTranscriptEvent::TtsWord { text, pts_ms } => {
                    EngineEvent::Voice(VoiceEvent::TtsWord { text, pts_ms })
                }
            };
            if engine_tx.send(engine_ev).await.is_err() {
                break; // engine bus closed
            }
        }
    });
}

pub async fn configure(
    voice_value: Option<&serde_json::Value>,
    engine_tx: jfc_engine::runtime::EventSender,
) {
    let cfg = voice_value
        .map(|value| VoiceConfig::from_settings(Some(value)))
        .unwrap_or_else(current_config);
    if let Some(rec) = RECORDER.get() {
        rec.lock().await.reconfigure(cfg);
    } else {
        init_with_config(cfg, engine_tx);
    }
}

/// Activate/deactivate push-to-talk.
///
/// - `pressed = true` → key down (start recording in hold mode, or toggle in tap mode)
/// - `pressed = false` → key up (stop recording in hold mode only)
///
/// Returns immediately if voice has not been initialized (voice is disabled or not configured).
pub async fn activate(pressed: bool) {
    if let Some(rec) = RECORDER.get() {
        rec.lock().await.activate(pressed).await;
    }
}

/// Cancel any active recording (e.g. on Esc) AND the VAD listen loop.
pub async fn cancel() {
    if let Some(rec) = RECORDER.get() {
        rec.lock().await.cancel().await;
    }
}

/// Discard an in-flight hold/tap recording without emitting a transcript — used
/// when the user submits manually (Enter) so voice doesn't auto-submit a
/// duplicate. Leaves the VAD listen loop running.
pub async fn discard_recording() {
    if let Some(rec) = RECORDER.get() {
        rec.lock().await.discard_recording().await;
    }
}

/// Current voice state for rendering.
pub async fn state() -> jfc_voice::VoiceState {
    if let Some(rec) = RECORDER.get() {
        rec.lock().await.state().await
    } else {
        jfc_voice::VoiceState::Idle
    }
}

/// Whether the continuous VAD listen loop is currently running. This is the
/// authoritative "hands-free mode" signal: `/voice vad` configures the recorder
/// for VAD but does NOT persist `mode=vad` to the config that `current_config()`
/// reads, so the auto-submit decision must consult the live loop, not just cfg.
pub async fn vad_loop_running() -> bool {
    match RECORDER.get() {
        Some(rec) => rec.lock().await.vad_loop_running(),
        None => false,
    }
}

/// Start the VAD continuous-listen loop. Called by `/voice vad`.
/// The recorder must already be initialized. The loop runs until `cancel()`.
pub async fn start_vad() {
    if let Some(rec) = RECORDER.get() {
        rec.lock().await.start_vad_loop().await;
    }
}

/// True if voice has been initialized (enabled + init() called).
pub fn is_initialized() -> bool {
    RECORDER.get().is_some()
}

// ── Incremental streaming read-aloud ────────────────────────────────────────
//
// Speaks the assistant reply sentence-by-sentence AS it streams, rather than
// synthesizing the whole finished message at the end. A single background task
// per turn owns the [`jfc_voice::streaming_tts::StreamingTts`] session; the
// event loop feeds it the growing transcript via [`read_aloud_feed`], ends the
// turn with [`read_aloud_finish`], and barges-in (stops playback immediately)
// with [`read_aloud_barge_in`] when the user starts speaking.

static READ_ALOUD_TURN: OnceLock<StdMutex<Option<ReadAloudTurn>>> = OnceLock::new();

/// Serializes read-aloud PLAYBACK so two sections of one reply (or two quick
/// turns) never play over each other. Each turn task holds this for the entire
/// time it owns the speaker — including the final drain — so the next task waits
/// for it to finish before opening its own playback. Without this, `finish()`
/// drains asynchronously while the next section's session starts immediately,
/// and you hear both at once.
static READ_ALOUD_PLAYBACK: Mutex<()> = Mutex::const_new(());

struct ReadAloudTurn {
    tx: tokio::sync::mpsc::UnboundedSender<TurnMsg>,
    /// Byte length of the streaming transcript already handed to the task.
    fed_bytes: usize,
}

enum TurnMsg {
    Text(String),
    Done,
    Cancel,
}

fn read_aloud_turn_slot() -> &'static StdMutex<Option<ReadAloudTurn>> {
    READ_ALOUD_TURN.get_or_init(|| StdMutex::new(None))
}

/// Feed the current FULL streaming-assistant transcript. The new suffix (vs.
/// what was already fed) is split into sentences and synthesized/played
/// incrementally. Starts a read-aloud turn on the first call — but only when
/// read-aloud is enabled, so it's a cheap no-op otherwise.
pub fn read_aloud_feed(full_text: &str, engine_tx: &EventSender) {
    let Ok(mut slot) = read_aloud_turn_slot().lock() else {
        return;
    };
    ensure_turn(&mut slot, engine_tx);
    let Some(turn) = slot.as_mut() else {
        return; // read-aloud off — nothing to feed
    };
    let start = floor_char_boundary(full_text, turn.fed_bytes.min(full_text.len()));
    let new = &full_text[start..];
    if new.is_empty() {
        return;
    }
    if turn.tx.send(TurnMsg::Text(new.to_owned())).is_err() {
        // The task ended (error/cancel) — reset so a later turn can restart.
        *slot = None;
        return;
    }
    turn.fed_bytes = full_text.len();
}

/// Pre-warm the read-aloud turn: open the TTS WebSocket *now* — typically while
/// the model is still thinking/streaming its first tokens — so the first spoken
/// sentence isn't blocked on connect latency. Cheap no-op when read-aloud is
/// off or a turn is already active. The opened socket idles on keepalive until
/// the first sentence arrives.
pub fn read_aloud_prewarm(engine_tx: &EventSender) {
    let Ok(mut slot) = read_aloud_turn_slot().lock() else {
        return;
    };
    ensure_turn(&mut slot, engine_tx);
}

/// Start the read-aloud task (which connects the TTS socket) if no turn is
/// active and read-aloud is enabled. The config is parsed once per turn here,
/// not per delta.
fn ensure_turn(slot: &mut Option<ReadAloudTurn>, engine_tx: &EventSender) {
    if slot.is_some() {
        return;
    }
    let cfg = current_config();
    if !read_aloud_on(&cfg) {
        return;
    }
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<TurnMsg>();
    tokio::spawn(read_aloud_turn_task(cfg, engine_tx.clone(), rx));
    *slot = Some(ReadAloudTurn { tx, fed_bytes: 0 });
}

/// End the current read-aloud turn: flush the trailing partial sentence and
/// drain playback. No-op when no turn is active.
pub fn read_aloud_finish() {
    if let Ok(mut slot) = read_aloud_turn_slot().lock()
        && let Some(turn) = slot.take()
    {
        let _ = turn.tx.send(TurnMsg::Done);
    }
}

/// Barge-in: stop any in-progress read-aloud playback immediately (called when
/// the user starts speaking). No-op when nothing is playing.
pub fn read_aloud_barge_in() {
    if let Ok(mut slot) = read_aloud_turn_slot().lock()
        && let Some(turn) = slot.take()
    {
        let _ = turn.tx.send(TurnMsg::Cancel);
    }
}

async fn read_aloud_turn_task(
    cfg: VoiceConfig,
    engine_tx: EventSender,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<TurnMsg>,
) {
    let Some(token) = resolve_read_aloud_token().await else {
        send_voice_event(
            &engine_tx,
            VoiceEvent::ReadAloudError("no Claude OAuth token available".to_owned()),
        )
        .await;
        while rx.recv().await.is_some() {} // drain so feeders don't error mid-turn
        return;
    };
    let user_agent = format!("jfc-voice/{}", env!("CARGO_PKG_VERSION"));
    let session =
        match jfc_voice::streaming_tts::StreamingTts::connect(&cfg, &token, &user_agent).await {
            Ok(session) => session,
            Err(err) => {
                send_voice_event(&engine_tx, VoiceEvent::ReadAloudError(err.to_string())).await;
                while rx.recv().await.is_some() {}
                return;
            }
        };
    // Serialize playback: wait for any in-flight read-aloud to finish draining
    // before we feed the speaker, so sections never overlap. The WS is already
    // open (idle keepalive) during the wait; held until this task fully drains.
    let _playback_guard = READ_ALOUD_PLAYBACK.lock().await;

    // Don't speak code blocks / inline code / URLs / markdown markup.
    let mut filter = TtsProseFilter::default();
    let mut pending = String::new();
    // `spoke` flips on the first real audio feed. It gates both the
    // first-clause flush (faster start) and the `ReadAloudStarted` UI signal —
    // emitting that only on real audio, never on the pre-warm connect (which
    // can happen while the model is still thinking), so the speaking indicator
    // doesn't show fake liveness.
    let mut spoke = false;
    loop {
        match rx.recv().await {
            Some(TurnMsg::Text(text)) => {
                pending.push_str(&filter.push(&text));
                loop {
                    let chunk = if spoke {
                        take_sentence(&mut pending)
                    } else {
                        take_first_chunk(&mut pending)
                    };
                    let Some(chunk) = chunk else { break };
                    if !spoke {
                        send_voice_event(&engine_tx, VoiceEvent::ReadAloudStarted { chars: 0 })
                            .await;
                        spoke = true;
                    }
                    session.feed(&chunk).await;
                }
            }
            Some(TurnMsg::Done) | None => {
                pending.push_str(&filter.flush());
                let tail = pending.trim();
                if !tail.is_empty() {
                    if !spoke {
                        send_voice_event(&engine_tx, VoiceEvent::ReadAloudStarted { chars: 0 })
                            .await;
                    }
                    session.feed(tail).await;
                }
                let stats =
                    tokio::time::timeout(std::time::Duration::from_secs(180), session.finish())
                        .await
                        .unwrap_or_default();
                send_voice_event(
                    &engine_tx,
                    VoiceEvent::ReadAloudCompleted {
                        audio_bytes: stats.audio_bytes,
                        chunks_sent: stats.chunks,
                    },
                )
                .await;
                tracing::debug!(
                    target: "jfc::voice::tts",
                    audio_bytes = stats.audio_bytes,
                    chunks = stats.chunks,
                    "assistant reply read aloud (streaming)"
                );
                break;
            }
            Some(TurnMsg::Cancel) => {
                session.cancel();
                send_voice_event(
                    &engine_tx,
                    VoiceEvent::ReadAloudCompleted {
                        audio_bytes: 0,
                        chunks_sent: 0,
                    },
                )
                .await;
                tracing::debug!(target: "jfc::voice::tts", "read-aloud barge-in (cancelled)");
                break;
            }
        }
    }
}

/// Strips content that shouldn't be spoken — fenced code blocks, inline code,
/// URLs, and markdown markup — leaving readable prose. Stateful so it tracks
/// code fences across streaming deltas (a fence opens in one delta, closes in
/// another).
#[derive(Default)]
struct TtsProseFilter {
    in_fence: bool,
    /// Incomplete trailing line carried between deltas (fence/line decisions
    /// need whole lines).
    carry: String,
}

impl TtsProseFilter {
    /// Feed a streaming delta; return speakable prose from any lines it
    /// completed. The trailing partial line is carried to the next call.
    fn push(&mut self, delta: &str) -> String {
        self.carry.push_str(delta);
        let mut out = String::new();
        while let Some(nl) = self.carry.find('\n') {
            let line = self.carry[..nl].to_owned();
            self.carry.drain(..=nl);
            self.process_line(&line, &mut out);
        }
        out
    }

    /// Flush the trailing line at end of turn.
    fn flush(&mut self) -> String {
        let line = std::mem::take(&mut self.carry);
        let mut out = String::new();
        if !line.is_empty() {
            self.process_line(&line, &mut out);
        }
        out
    }

    fn process_line(&mut self, line: &str, out: &mut String) {
        let trimmed = line.trim_start();
        // A fence line toggles code mode; never speak the fence or its language.
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            self.in_fence = !self.in_fence;
            return;
        }
        if self.in_fence {
            return; // inside a code block — skip entirely
        }
        // Table rows / delimiter rows read terribly aloud; skip them whole.
        if is_table_line(trimmed) {
            return;
        }
        // Robust inline stripping via the markdown AST (inline code, links →
        // text, URLs → "link", emphasis/heading/list markers). Fenced code is
        // already handled above by line-level fence tracking, which `pulldown`
        // can't see one line at a time.
        let prose = jfc_markdown::speakable_prose(line);
        if !prose.trim().is_empty() {
            out.push_str(prose.trim_end());
            out.push('\n');
        }
    }
}

/// A markdown table row (`| a | b |`) or delimiter (`|---|---|`) — not prose.
fn is_table_line(trimmed: &str) -> bool {
    trimmed.starts_with('|') && trimmed.matches('|').count() >= 2
}

/// Pull the next complete sentence from `buf`, leaving the incomplete tail in
/// place. Splits on a newline, on `.`/`!`/`?` *followed by whitespace* (so
/// "3.14"/"e.g." mid-token don't split), or on a hard byte cap so the first
/// audio starts promptly on a long run-on. Returns `None` until a boundary
/// exists (so a sentence isn't emitted until we've seen what follows it).
fn take_sentence(buf: &mut String) -> Option<String> {
    take_chunk(buf, false)
}

/// Like [`take_sentence`] but flushes the FIRST utterance of a turn sooner: it
/// also splits on a clause boundary (`,`/`;`/`:`/`—`/`–` + whitespace) and uses
/// a shorter byte cap, so audio starts on the first clause instead of waiting
/// for a full sentence. Later chunks fall back to sentence granularity for
/// natural prosody.
fn take_first_chunk(buf: &mut String) -> Option<String> {
    take_chunk(buf, true)
}

fn take_chunk(buf: &mut String, clause: bool) -> Option<String> {
    const HARD_CAP_BYTES: usize = 240;
    const FIRST_CAP_BYTES: usize = 96;
    let hard_cap = if clause {
        FIRST_CAP_BYTES
    } else {
        HARD_CAP_BYTES
    };
    let mut split_at: Option<usize> = None;
    let mut chars = buf.char_indices().peekable();
    while let Some((idx, ch)) = chars.next() {
        if ch == '\n' {
            split_at = Some(idx + ch.len_utf8());
            break;
        }
        // Sentence enders always split; clause enders only for the first chunk.
        // The "followed by whitespace" guard keeps "3.14"/"1,000"/"3:1" intact.
        let is_boundary =
            matches!(ch, '.' | '!' | '?') || (clause && matches!(ch, ',' | ';' | ':' | '—' | '–'));
        if is_boundary && chars.peek().is_some_and(|&(_, c)| c.is_whitespace()) {
            split_at = Some(idx + ch.len_utf8());
            break;
        }
    }
    if split_at.is_none() && buf.len() >= hard_cap {
        let cap = floor_char_boundary(buf, hard_cap);
        if let Some(sp) = buf[..cap].rfind(char::is_whitespace).filter(|&p| p > 0) {
            split_at = Some(sp + 1);
        }
    }
    let at = floor_char_boundary(buf, split_at?);
    let sentence = buf[..at].trim().to_owned();
    // Drop the leading whitespace the boundary split left on the tail (the
    // space after "sentence. " belongs to neither chunk). Keep trailing space
    // so an unfinished tail still reads as mid-stream. Without trim_start the
    // retained tail carried a leading space into the next chunk's buffer.
    let rest = buf[at..].trim_start().to_owned();
    *buf = rest;
    if sentence.is_empty() {
        return take_chunk(buf, clause);
    }
    Some(sentence)
}

/// Largest char boundary `<= i` (stable stand-in for unstable
/// `str::floor_char_boundary`).
fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

async fn resolve_read_aloud_token() -> Option<String> {
    jfc_providers::current_access_token().await
}

async fn send_voice_event(tx: &EventSender, event: VoiceEvent) {
    let _ = tx.send(EngineEvent::Voice(event)).await;
}

pub fn current_config() -> VoiceConfig {
    let cfg = jfc_engine::config::load_arc();
    config_from_loaded(&cfg)
}

pub fn config_from_loaded(cfg: &jfc_config::Config) -> VoiceConfig {
    let voice_value = cfg
        .voice
        .as_ref()
        .map(jfc_config::VoiceSettingsConfig::to_compat_json)
        .or_else(|| cfg.claude.voice.clone());
    VoiceConfig::from_settings(voice_value.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_from_loaded_prefers_voice_table_over_claude_compat_normal() {
        let mut cfg = jfc_config::Config {
            voice: Some(jfc_config::VoiceSettingsConfig {
                enabled: Some(true),
                mode: Some("tap".to_owned()),
                auto_submit: Some(true),
                read_aloud: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        };
        cfg.claude.voice = Some(serde_json::json!({
            "enabled": false,
            "mode": "hold",
            "readAloud": false
        }));

        let voice = config_from_loaded(&cfg);

        assert!(voice.enabled);
        assert_eq!(voice.mode, jfc_voice::VoiceMode::Tap);
        assert!(voice.auto_submit);
        assert!(voice.read_aloud);
    }

    #[test]
    fn read_aloud_override_wins_over_config_regression() {
        use std::sync::atomic::Ordering;
        let mut cfg = VoiceConfig::default();

        // Unset override → fall back to the config value.
        READ_ALOUD_OVERRIDE.store(0, Ordering::Relaxed);
        cfg.read_aloud = false;
        assert!(!read_aloud_on(&cfg));
        cfg.read_aloud = true;
        assert!(read_aloud_on(&cfg));

        // `/voice readaloud on` forces it on regardless of config.
        set_read_aloud_override(true);
        cfg.read_aloud = false;
        assert!(read_aloud_on(&cfg));

        // `/voice readaloud off` forces it off regardless of config.
        set_read_aloud_override(false);
        cfg.read_aloud = true;
        assert!(!read_aloud_on(&cfg));

        READ_ALOUD_OVERRIDE.store(0, Ordering::Relaxed); // reset process-global
    }

    #[test]
    fn take_sentence_emits_on_boundary_and_keeps_tail_regression() {
        let mut buf = String::from("Hello world. This is ");
        // First sentence is complete (period + following whitespace).
        assert_eq!(take_sentence(&mut buf).as_deref(), Some("Hello world."));
        // The remainder has no terminal boundary yet → withheld until more text.
        assert_eq!(take_sentence(&mut buf), None);
        assert_eq!(buf, "This is ");
        buf.push_str("a test! And then more");
        assert_eq!(take_sentence(&mut buf).as_deref(), Some("This is a test!"));
        assert_eq!(take_sentence(&mut buf), None); // "And then more" — no boundary
    }

    #[test]
    fn take_sentence_does_not_split_inside_decimals_regression() {
        // "3." is followed by a digit, not whitespace → no split there.
        let mut buf = String::from("Pi is about 3.14 today. Done");
        assert_eq!(
            take_sentence(&mut buf).as_deref(),
            Some("Pi is about 3.14 today.")
        );
    }

    #[test]
    fn take_sentence_splits_on_newline_normal() {
        let mut buf = String::from("first line\nsecond");
        assert_eq!(take_sentence(&mut buf).as_deref(), Some("first line"));
        assert_eq!(take_sentence(&mut buf), None);
    }

    #[test]
    fn take_sentence_hard_caps_long_runons_normal() {
        // No punctuation, well over the cap → flush a bounded chunk so the first
        // audio doesn't wait for the whole run-on.
        let mut buf = "word ".repeat(60); // 300 bytes
        let chunk = take_sentence(&mut buf).expect("hard cap should flush a chunk");
        assert!(!chunk.is_empty());
        assert!(chunk.len() <= 240, "chunk too long: {}", chunk.len());
        assert!(!buf.is_empty(), "remainder should be retained");
    }

    #[test]
    fn tts_prose_filter_skips_code_and_markup_robust() {
        let mut f = TtsProseFilter::default();
        let mut out = String::new();
        out.push_str(&f.push(
            "Here is the fix.\n```rust\nlet x = 5;\nfn foo() {}\n```\nDone — see `foo()` at https://x.com\n",
        ));
        out.push_str(&f.flush());
        assert!(out.contains("Here is the fix"), "prose dropped: {out:?}");
        assert!(out.contains("Done"), "prose dropped: {out:?}");
        assert!(!out.contains("let x"), "code line spoken: {out:?}");
        assert!(!out.contains("fn foo"), "code line spoken: {out:?}");
        assert!(!out.contains("```"), "fence spoken: {out:?}");
        assert!(!out.contains("foo()"), "inline code spoken: {out:?}");
        assert!(!out.contains("x.com"), "url spoken: {out:?}");
        assert!(out.contains("link"), "url not replaced: {out:?}");
    }

    #[test]
    fn tts_prose_filter_handles_fence_split_across_deltas_robust() {
        // Fence opens in one delta and closes in another — state must persist.
        let mut f = TtsProseFilter::default();
        let mut out = String::new();
        out.push_str(&f.push("Intro line.\n``"));
        out.push_str(&f.push("`\ncode_here();\n```\nOutro."));
        out.push_str(&f.flush());
        assert!(out.contains("Intro line"), "{out:?}");
        assert!(out.contains("Outro"), "{out:?}");
        assert!(!out.contains("code_here"), "code leaked: {out:?}");
    }
}
