//! LIVE integration tests against the REAL Anthropic voice WebSocket endpoints
//! (`text_to_speech/text_stream` and `speech_to_text/voice_stream`).
//!
//! These hit the network with a real Claude OAuth login, so they are
//! `#[ignore]`d by default (a normal `cargo test` skips them) and additionally
//! self-skip when no token is available. Run them explicitly:
//!
//!   cargo test -p jfc --test voice_live -- --ignored --nocapture
//!
//! They guard the two bugs found and fixed via the `voice_debug` example:
//!   * VAD batch STT returned an empty transcript because the whole utterance
//!     was dumped in one burst then closed — the server discards that. The fix
//!     paces the send ~real-time; this test asserts a full transcript comes
//!     back over the live socket.
//!   * (companion) TTS synthesis returns real audio bytes.

use std::time::Duration;

use jfc_voice::VoiceConfig;
use tokio::io::AsyncWriteExt;

fn init() {
    // The app installs this at startup; the WS TLS connect panics without it.
    let _ = rustls::crypto::ring::default_provider().install_default();
}

async fn oauth_token() -> Option<String> {
    jfc_providers::current_access_token().await
}

/// Synthesize `text` to raw PCM over the live TTS WS (no mic needed — gives a
/// deterministic speech source for the STT test).
async fn synth(cfg: &VoiceConfig, token: &str, text: &str) -> (Vec<u8>, jfc_voice::tts::TtsStats) {
    let ua = "jfc-voice-live-test";
    let path = std::env::temp_dir().join("jfc_voice_live_test.pcm");
    let mut f = tokio::fs::File::create(&path).await.unwrap();
    let stats = jfc_voice::tts::synthesize_to_writer(cfg, token, ua, text, &mut f)
        .await
        .expect("live TTS synthesize");
    f.flush().await.unwrap();
    drop(f);
    (std::fs::read(&path).unwrap(), stats)
}

#[tokio::test]
#[ignore = "live: needs Claude OAuth + network"]
async fn live_tts_returns_audio() {
    init();
    let Some(token) = oauth_token().await else {
        eprintln!("SKIP live_tts_returns_audio: no Claude OAuth token");
        return;
    };
    let cfg = VoiceConfig::default();
    let (pcm, stats) = tokio::time::timeout(
        Duration::from_secs(30),
        synth(&cfg, &token, "Testing one two three."),
    )
    .await
    .expect("TTS should not hang");
    assert!(stats.audio_bytes > 0, "TTS returned no audio bytes");
    assert!(!pcm.is_empty(), "TTS produced empty PCM");
    eprintln!(
        "live_tts: {} pcm bytes, {} chunks",
        pcm.len(),
        stats.chunks_sent
    );
}

#[tokio::test]
#[ignore = "live: needs Claude OAuth + network"]
async fn live_vad_batch_transcribes_full_utterance() {
    init();
    let Some(token) = oauth_token().await else {
        eprintln!("SKIP live_vad_batch_transcribes_full_utterance: no Claude OAuth token");
        return;
    };
    let cfg = VoiceConfig::default();
    let phrase = "The quick brown fox jumps over the lazy dog.";
    let (pcm, _) = synth(&cfg, &token, phrase).await;
    assert!(
        pcm.len() > 16_000,
        "need a few seconds of audio, got {} bytes",
        pcm.len()
    );

    // The VAD path: batch transcribe over the live voice_stream WS. Regression:
    // before the real-time pacing fix this returned None because the server
    // discarded the burst-then-close.
    let mut stt_cfg = VoiceConfig::default();
    stt_cfg.enabled = true;
    let out = tokio::time::timeout(
        Duration::from_secs(60),
        jfc_voice::backends::transcribe_with_token(&pcm, &stt_cfg, Some(&token)),
    )
    .await
    .expect("batch STT should not hang")
    .expect("live batch STT call");
    let text = out
        .expect("live VAD batch STT must return a transcript (regression: burst → empty)")
        .to_lowercase();
    eprintln!("live_vad transcript: {text:?}");
    assert!(
        text.contains("quick") && text.contains("fox"),
        "transcript should resemble the spoken phrase, got: {text:?}"
    );
}

#[tokio::test]
#[ignore = "live: needs Claude OAuth + network"]
async fn live_streaming_finalize_is_fast() {
    use jfc_voice::anthropic_ws::{StreamMsg, StreamOpts, connect_voice_stream};
    init();
    let Some(token) = oauth_token().await else {
        eprintln!("SKIP live_streaming_finalize_is_fast: no Claude OAuth token");
        return;
    };
    let cfg = VoiceConfig::default();
    let (pcm, _) = synth(&cfg, &token, "The quick brown fox jumps over the lazy dog.").await;

    let opts = StreamOpts {
        language: "en".to_owned(),
        keyterms: Vec::new(),
        forward_interims: true,
        allow_custom_auth_endpoint: false,
        allow_insecure_auth_endpoint: false,
    };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<StreamMsg>();
    let stream = connect_voice_stream(
        "wss://api.anthropic.com",
        &token,
        "jfc-voice-live-test",
        "claude_code_cli",
        &opts,
        tx,
    )
    .await
    .unwrap();

    // Stream paced ~real-time — this is what the VAD loop does WHILE you talk,
    // so it overlaps speech and is "free" wall-clock.
    for frame in pcm.chunks(640) {
        stream.send(frame).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // The metric that matters: latency AFTER speech ends. Streaming-during-capture
    // keeps the server caught up, so finalize resolves quickly.
    let t = std::time::Instant::now();
    let _ = stream.finalize().await;
    let post_stop = t.elapsed();

    let mut transcript = String::new();
    while let Ok(msg) = rx.try_recv() {
        if let StreamMsg::Transcript { text, .. } = msg {
            if !text.is_empty() {
                transcript = text;
            }
        }
    }
    eprintln!("live_streaming: post-stop finalize={post_stop:?}, transcript={transcript:?}");
    assert!(
        transcript.to_lowercase().contains("quick"),
        "streaming transcript should resemble the phrase, got: {transcript:?}"
    );
    assert!(
        post_stop < Duration::from_secs(3),
        "post-stop finalize should be fast (server caught up during capture), was {post_stop:?}"
    );
}

#[tokio::test]
#[ignore = "live: needs Claude OAuth + network + a PCM player"]
async fn live_streaming_tts_synthesizes_incrementally() {
    init();
    let Some(token) = oauth_token().await else {
        eprintln!("SKIP live_streaming_tts_synthesizes_incrementally: no Claude OAuth token");
        return;
    };
    let cfg = VoiceConfig::default();
    let session =
        match jfc_voice::streaming_tts::StreamingTts::connect(&cfg, &token, "jfc-voice-live-test")
            .await
        {
            Ok(session) => session,
            Err(err) => {
                eprintln!("SKIP live_streaming_tts: connect failed (no PCM player?): {err}");
                return;
            }
        };
    // Feed two sentences as they'd arrive during generation, then finish.
    session.feed("Hello there.").await;
    session.feed("This is the streaming read aloud path.").await;
    let stats = tokio::time::timeout(Duration::from_secs(60), session.finish())
        .await
        .expect("streaming TTS finish should not hang");
    eprintln!(
        "live_streaming_tts: audio_bytes={} chunks={}",
        stats.audio_bytes, stats.chunks
    );
    assert!(
        stats.audio_bytes > 0,
        "streaming TTS should produce audio bytes"
    );
}
