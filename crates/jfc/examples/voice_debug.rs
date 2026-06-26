//! Live voice debug harness — exercises the REAL jfc-voice code paths so we can
//! see audio-out / VAD / STT behavior outside the TUI, with full tracing.
//!
//! Build:  cargo build -p jfc --example voice_debug
//! Run:    ./target/debug/examples/voice_debug <command>
//!
//!   tone            Play a 1s tone through JFC's PcmPlayback (proves speaker out).
//!   tts [text]      Synthesize text via Anthropic TTS and play it (proves audio out).
//!   stt [secs]      Record N s from the mic, then run the VAD *batch* transcribe
//!                   (`backends::transcribe_with_token`) — the exact path VAD uses.
//!   vad [secs]      Run the full VAD listen loop for N s and print every event.
//!
//! Tracing goes to stderr; tune with RUST_LOG (default shows jfc::voice=debug).

use std::time::{Duration, Instant};

use jfc_voice::{VoiceConfig, VoiceMode, VoiceRecorder, VoiceTranscriptEvent};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // The real app installs this at startup (main.rs); the WS TLS connect panics
    // without it. Not a JFC bug — just standalone-example setup.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| {
        "info,jfc::voice=debug,jfc::voice::vad=debug,jfc::voice::ws=debug,jfc::voice::stt=debug"
            .to_owned()
    });
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .with_writer(std::io::stderr)
        .with_target(true)
        .init();

    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_default();
    match cmd.as_str() {
        "tone" => cmd_tone().await,
        "tts" => {
            let text = args.collect::<Vec<_>>().join(" ");
            let text = if text.trim().is_empty() {
                "Hello from the J F C voice debug harness. If you can hear this, \
                 text to speech is working."
                    .to_owned()
            } else {
                text
            };
            cmd_tts(&text).await
        }
        "roundtrip" => {
            let text = args.collect::<Vec<_>>().join(" ");
            let text = if text.trim().is_empty() {
                "Testing one two three four five.".to_owned()
            } else {
                text
            };
            cmd_roundtrip(&text).await
        }
        "stream" => {
            let text = args.collect::<Vec<_>>().join(" ");
            let text = if text.trim().is_empty() {
                "The quick brown fox jumps over the lazy dog.".to_owned()
            } else {
                text
            };
            cmd_stream(&text).await
        }
        "speaktts" => {
            let text = args.collect::<Vec<_>>().join(" ");
            let text = if text.trim().is_empty() {
                "Hello there. This is the streaming text to speech path. \
                 Notice that I start talking after the first sentence, while the \
                 rest of this reply is still being generated. That is the point."
                    .to_owned()
            } else {
                text
            };
            cmd_speaktts(&text).await
        }
        "stt" => cmd_stt(args.next().and_then(|s| s.parse().ok()).unwrap_or(5)).await,
        "vad" => cmd_vad(args.next().and_then(|s| s.parse().ok()).unwrap_or(20)).await,
        other => {
            eprintln!("usage: voice_debug <tone|tts [text]|stt [secs]|vad [secs]>");
            eprintln!("unknown command: {other:?}");
            Ok(())
        }
    }
}

/// Generate `secs` seconds of a 16 kHz mono s16le sine at `hz`.
fn sine_pcm(hz: f32, secs: f32) -> Vec<u8> {
    let sr = 16_000.0_f32;
    let n = (sr * secs) as usize;
    let mut pcm = Vec::with_capacity(n * 2);
    for i in 0..n {
        let t = i as f32 / sr;
        let v = ((2.0 * std::f32::consts::PI * hz * t).sin() * 12_000.0) as i16;
        pcm.extend_from_slice(&v.to_le_bytes());
    }
    pcm
}

async fn cmd_tone() -> anyhow::Result<()> {
    let cfg = VoiceConfig::default();
    match jfc_voice::playback::detect_playback_command(cfg.tts_playback_command.as_deref(), None) {
        Some(c) => eprintln!("[tone] playback command: {c:?}"),
        None => eprintln!("[tone] WARNING: no playback command detected!"),
    }
    let mut player = jfc_voice::playback::PcmPlayback::start(&cfg)?;
    eprintln!("[tone] playing 1s 440Hz tone — you should hear a beep…");
    player.write_audio(&sine_pcm(440.0, 1.0)).await?;
    player.finish().await?;
    eprintln!("[tone] done.");
    Ok(())
}

async fn cmd_tts(text: &str) -> anyhow::Result<()> {
    let token = jfc_providers::current_access_token()
        .await
        .ok_or_else(|| anyhow::anyhow!("no Claude OAuth token — log in first"))?;
    let cfg = VoiceConfig::default();
    let ua = format!("jfc-voice-debug/{}", env!("CARGO_PKG_VERSION"));
    eprintln!(
        "[tts] voice={} speed={} fmt={} — synthesizing {} chars…",
        cfg.tts_voice,
        cfg.tts_speed,
        cfg.tts_output_format,
        text.chars().count()
    );
    let t0 = Instant::now();
    let stats = jfc_voice::playback::speak_anthropic_tts(&cfg, &token, &ua, text).await?;
    eprintln!(
        "[tts] done in {:?}: audio_bytes={} chunks_sent={}",
        t0.elapsed(),
        stats.audio_bytes,
        stats.chunks_sent
    );
    Ok(())
}

/// TTS → PCM → batch STT, with no mic. Exercises the exact VAD transcription
/// path (`backends::transcribe_with_token`) on real speech audio and times it.
async fn cmd_roundtrip(text: &str) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;
    let token = jfc_providers::current_access_token()
        .await
        .ok_or_else(|| anyhow::anyhow!("no Claude OAuth token — log in first"))?;
    let cfg = VoiceConfig::default();
    let ua = format!("jfc-voice-debug/{}", env!("CARGO_PKG_VERSION"));

    let path = std::env::temp_dir().join("jfc_voice_debug_tts.pcm");
    let mut f = tokio::fs::File::create(&path).await?;
    eprintln!("[roundtrip] 1/2 synthesizing {:?}…", text);
    let t_tts = Instant::now();
    let stats = jfc_voice::tts::synthesize_to_writer(&cfg, &token, &ua, text, &mut f).await?;
    f.flush().await?;
    drop(f);
    let pcm = std::fs::read(&path)?;
    eprintln!(
        "[roundtrip] synthesized {} pcm bytes (~{:.1}s) in {:?} (chunks={})",
        pcm.len(),
        pcm.len() as f32 / (16_000.0 * 2.0),
        t_tts.elapsed(),
        stats.chunks_sent
    );

    let mut cfg2 = VoiceConfig::default();
    cfg2.enabled = true;
    eprintln!(
        "[roundtrip] 2/2 running batch transcribe_with_token (VAD path), backend={:?}…",
        cfg2.effective_backend()
    );
    let t_stt = Instant::now();
    let result = jfc_voice::backends::transcribe_with_token(&pcm, &cfg2, Some(&token)).await;
    eprintln!(
        "[roundtrip] transcribe returned in {:?}: {result:?}",
        t_stt.elapsed()
    );
    Ok(())
}

/// Replay synthesized speech into the voice_stream paced ~real-time (20ms/frame)
/// instead of dumping it. Tests the hypothesis that the server discards a
/// burst+close but transcribes a paced stream.
async fn cmd_stream(text: &str) -> anyhow::Result<()> {
    use jfc_voice::anthropic_ws::{StreamMsg, StreamOpts, connect_voice_stream};
    use tokio::io::AsyncWriteExt;
    let token = jfc_providers::current_access_token()
        .await
        .ok_or_else(|| anyhow::anyhow!("no Claude OAuth token — log in first"))?;
    let cfg = VoiceConfig::default();
    let ua = format!("jfc-voice-debug/{}", env!("CARGO_PKG_VERSION"));

    let path = std::env::temp_dir().join("jfc_voice_debug_tts.pcm");
    let mut f = tokio::fs::File::create(&path).await?;
    jfc_voice::tts::synthesize_to_writer(&cfg, &token, &ua, text, &mut f).await?;
    f.flush().await?;
    drop(f);
    let pcm = std::fs::read(&path)?;
    eprintln!(
        "[stream] {} pcm bytes — replaying paced (real-time, 20ms/640B frame)…",
        pcm.len()
    );

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
        &ua,
        "claude_code_cli",
        &opts,
        tx,
    )
    .await?;
    // 640 bytes = 20ms of 16kHz mono s16le audio. JFC_DEBUG_PACE_MS controls the
    // inter-frame delay: 20 = real-time, 10 = 2x, 5 = 4x, 0 = burst (repro the bug).
    let pace_ms: u64 = std::env::var("JFC_DEBUG_PACE_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    eprintln!("[stream] pace_ms={pace_ms}");
    // Phase 1: send the audio paced (this overlaps with the user talking in the
    // real VAD path — it's "free" wall-clock).
    let t_send = Instant::now();
    for frame in pcm.chunks(640) {
        stream.send(frame).await;
        if pace_ms > 0 {
            tokio::time::sleep(Duration::from_millis(pace_ms)).await;
        }
    }
    let send_elapsed = t_send.elapsed();
    // Phase 2: finalize — this is the post-speech latency the VAD refactor pays
    // AFTER you stop talking. With streaming-during-capture it should be small.
    let t_fin = Instant::now();
    let reason = stream.finalize().await;
    let finalize_elapsed = t_fin.elapsed();
    let mut transcript = String::new();
    while let Ok(msg) = rx.try_recv() {
        if let StreamMsg::Transcript { text, .. } = msg {
            if !text.is_empty() {
                transcript = text;
            }
        }
    }
    eprintln!(
        "[stream] sent_in={send_elapsed:?} (overlaps speech) | POST-STOP finalize={finalize_elapsed:?} | reason={reason:?}"
    );
    eprintln!("[stream] transcript: {transcript:?}");
    Ok(())
}

/// Feed text sentence-by-sentence into the streaming TTS session with simulated
/// generation delays — proving speech starts after the FIRST sentence, while the
/// rest is still "being generated".
async fn cmd_speaktts(text: &str) -> anyhow::Result<()> {
    let token = jfc_providers::current_access_token()
        .await
        .ok_or_else(|| anyhow::anyhow!("no Claude OAuth token — log in first"))?;
    let cfg = VoiceConfig::default();
    let ua = format!("jfc-voice-debug/{}", env!("CARGO_PKG_VERSION"));
    eprintln!("[speaktts] connecting streaming TTS…");
    let session = jfc_voice::streaming_tts::StreamingTts::connect(&cfg, &token, &ua).await?;
    eprintln!(
        "[speaktts] echo guard: tts_playback_active={} (VAD mic suppressed while true)",
        jfc_voice::streaming_tts::tts_playback_active()
    );
    eprintln!("[speaktts] feeding word-by-word (~120ms/word, sentence-flushed)…");
    let t0 = Instant::now();
    let mut sentence = String::new();
    for word in text.split_whitespace() {
        sentence.push_str(word);
        sentence.push(' ');
        if word.ends_with('.') || word.ends_with('!') || word.ends_with('?') {
            eprintln!("[speaktts] +{:?} feed: {:?}", t0.elapsed(), sentence.trim());
            session.feed(sentence.trim()).await;
            sentence.clear();
        }
        tokio::time::sleep(Duration::from_millis(120)).await;
    }
    if !sentence.trim().is_empty() {
        eprintln!(
            "[speaktts] +{:?} feed tail: {:?}",
            t0.elapsed(),
            sentence.trim()
        );
        session.feed(sentence.trim()).await;
    }
    eprintln!("[speaktts] +{:?} finish (draining playback)…", t0.elapsed());
    let stats = session.finish().await;
    eprintln!(
        "[speaktts] done in {:?}: audio_bytes={} chunks={} | tts_playback_active={} (echo guard lifted)",
        t0.elapsed(),
        stats.audio_bytes,
        stats.chunks,
        jfc_voice::streaming_tts::tts_playback_active()
    );
    Ok(())
}

async fn cmd_stt(secs: u64) -> anyhow::Result<()> {
    let backend = jfc_voice::AudioCapture::detect_backend()
        .await
        .ok_or_else(|| anyhow::anyhow!("no capture backend (install arecord/sox/ffmpeg)"))?;
    eprintln!("[stt] capturing {secs}s from {backend:?} — SPEAK NOW…");
    let mut cap = jfc_voice::AudioCapture::start(backend).await?;
    let mut pcm = Vec::new();
    let mut buf = vec![0u8; 640];
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        match cap.read_chunk(&mut buf).await {
            Ok(0) => break,
            Ok(n) => pcm.extend_from_slice(&buf[..n]),
            Err(e) => {
                eprintln!("[stt] read error: {e}");
                break;
            }
        }
    }
    pcm.extend_from_slice(&cap.stop().await);

    let mut cfg = VoiceConfig::default();
    cfg.enabled = true;
    eprintln!(
        "[stt] captured {} bytes (~{:.1}s). backend={:?}. Running batch \
         transcribe_with_token (the VAD path)…",
        pcm.len(),
        pcm.len() as f32 / (16_000.0 * 2.0),
        cfg.effective_backend()
    );
    let token = jfc_providers::current_access_token().await;
    eprintln!("[stt] have_token={}", token.is_some());
    let t0 = Instant::now();
    let result = jfc_voice::backends::transcribe_with_token(&pcm, &cfg, token.as_deref()).await;
    eprintln!(
        "[stt] transcribe returned in {:?}: {result:?}",
        t0.elapsed()
    );
    Ok(())
}

async fn cmd_vad(secs: u64) -> anyhow::Result<()> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<VoiceTranscriptEvent>();
    let mut cfg = VoiceConfig::default();
    cfg.enabled = true;
    cfg.mode = VoiceMode::Vad;
    let provider: jfc_voice::TokenProvider =
        std::sync::Arc::new(|| Box::pin(jfc_providers::current_access_token()));
    let mut rec = VoiceRecorder::new(cfg, tx).with_token_provider(provider);
    eprintln!(
        "[vad] starting VAD loop for {secs}s — SPEAK a sentence, then PAUSE (~1s).\n\
         [vad] A `Final` = the SERVER detected you stopped (endpoint) and the turn ended.\n\
         [vad] In the app that `Final` auto-submits as a prompt, exactly like pressing Enter.\n\
         [vad] vad_loop_running={} (the app's auto-submit gate keys off this).",
        rec.vad_loop_running()
    );
    rec.start_vad_loop().await;

    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut speech_started: Option<Instant> = None;
    let mut finals = 0usize;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Some(VoiceTranscriptEvent::Level(_))) => {} // too noisy to print
            Ok(Some(VoiceTranscriptEvent::StateChanged(s))) => {
                if matches!(s, jfc_voice::VoiceState::Recording) && speech_started.is_none() {
                    speech_started = Some(Instant::now());
                }
                eprintln!("[vad][state] {s:?}");
            }
            Ok(Some(VoiceTranscriptEvent::Final(text))) => {
                finals += 1;
                let took = speech_started
                    .map(|t| format!("{:?} after speech began", t.elapsed()))
                    .unwrap_or_else(|| "—".to_owned());
                speech_started = None;
                eprintln!(
                    "\n[vad] ✅ FINAL #{finals} ({took}) → server endpoint ended the turn.\n\
                     [vad]    WOULD AUTO-SUBMIT AS A PROMPT (like Enter): {text:?}\n",
                );
            }
            Ok(Some(ev)) => eprintln!("[vad][event] {ev:?}"),
            Ok(None) => break,
            Err(_) => break,
        }
    }
    rec.cancel().await;
    eprintln!(
        "[vad] stopped. Received {finals} Final(s). \
         {} — each Final is a turn the app submits automatically.",
        if finals > 0 {
            "✅ auto-submit path is live"
        } else {
            "⚠️  no Final — server endpoint never fired (check mic / token / connect logs)"
        }
    );
    Ok(())
}
