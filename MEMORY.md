# Project Memory

## Durable Facts

- JFC is a Rust workspace for an agentic terminal coding assistant.
- Root `CLAUDE.md` is intentionally present so JFC can dogfood context loading.
- `.claude/` is ignored by git in this repo and is suitable for local Claude Code
  runtime files.
- Prefer CodeGraph tools for structural code questions and `rg` for literal search.
- Keep generated runtime state, crash dumps, profiling output, and vendored research
  out of source changes unless the task explicitly concerns them.
- Local Claude/JFC guidance should guard against the common AI failure mode where
  many individually successful features do not work together.
- Before accepting feature velocity, check architecture, state ownership, cross-view
  integration, and scope boundaries.

## Voice Subsystem (jfc-voice + jfc TUI)

- VAD utterance flow: live frames stream to the Anthropic `voice_stream` WS;
  interims (`is_final=false`) preview into the input box; server endpointing
  (`is_final`) or client energy-VAD `SpeechEnd` ends the utterance; one `Final`
  per utterance → TUI auto-submits. There is ONE transcription source on the
  streaming path; the batch path (`backends::transcribe_with_token`) is a
  fallback used only when streaming returns empty — that is the second "method".
- Rehydration-after-Enter fix: `App.voice_suppress_input` drops late
  Interim/Final from a manually-submitted utterance, cleared on next Recording
  onset; `recorder.rs::discard_recording` covers Processing (not just Recording).
- Speaker identity ("is this our wanted input?"): `speaker.rs::verify_admit`
  decides over an accept-list (our speakers) + reject-list (own TTS voice);
  reject-list wins. `SpeakerGate` loads `<profile_dir>/speakers/*.json` +
  legacy `speaker_profile.json` (accept) and `reject/*.json` (reject). The gate
  now runs on the streaming path too (no longer forced to batch). Enroll via
  `/voice enroll [name] [secs]`. Design: `research/voice_speaker_identity/SPEC.md`.
- Self-echo defense layers: blunt half-duplex `echo_guard` (time-gate) + the
  acoustic reject-profile. Heavier future layers (WebRTC AEC3 reference-signal
  cancellation, personal-VAD/VoiceFilter) are documented, not yet implemented.

## Useful Verification Defaults

- Build: `cargo build`
- Test: `cargo test`
- Lint: `cargo clippy --workspace`
