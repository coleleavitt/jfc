# Voice Speaker-Identity Pipeline — Design Spec

Goal: jfc-voice should robustly answer **"is this our wanted input?"** for every
captured utterance, rejecting (a) the assistant's own TTS echoing back, (b) other
people in the room, and (c) background media (YouTube/TV/music).

## Research grounding (see `db:research` artifact)

Production voice agents use a **cascade**, not one model:

1. **Self-echo** is the easiest because *we control the reference signal* (the TTS
   we play). Two options: linear AEC (WebRTC AEC3 — needs the played reference +
   delay alignment + double-talk freeze) and/or **speaker verification against the
   TTS voice** (reject-profile). AEC3 is a heavy native dependency; the
   reject-profile reuses jfc's existing voiceprint infra with zero new deps.
2. **Other people / background TV** have no reference signal → **speaker
   verification**: embed the utterance (ECAPA-TDNN/x-vector or classical MFCC),
   accept only if it matches an enrolled speaker (cosine/Mahalanobis threshold).
   This rejects other voices, TV dialogue, and the TTS voice alike.
3. Heavier future layers: personal-VAD / target-speaker-extraction (VoiceFilter)
   for overlapping speech; linear AEC for true full-duplex barge-in.

No public detail was found on Anthropic/Claude voice echo handling.

## What already exists in jfc-voice

- `speaker.rs::SpeakerProfile` — MFCC diagonal-Gaussian + pitch + Mahalanobis
  threshold, optional neural ECAPA/x-vector embedding (`speaker-neural` feature).
  `score_with` / `accepts_with` give the accept decision.
- `recorder.rs::SpeakerGate` — single-profile gate. `admits(pcm)` drops
  non-matching utterances. Loaded from `cfg.speaker_gate` + one `speaker_profile.json`.
- `recorder.rs::enroll_primary_speaker` — one-off enrollment of a single speaker.
- `recorder.rs::echo_guard` — blunt **half-duplex time-gate**: mutes the mic while
  read-aloud plays + a 400ms tail. NOT acoustic self-recognition.
- **Gap A:** single speaker only (no "our speakers" accept-list).
- **Gap B:** no acoustic self-voice rejection (only the time-gate; leaks with
  `echo_suppression=false` / headphones, or past the tail).
- **Gap C:** the gate is *disabled on the streaming path* —
  `want_stream = token.is_some() && !cfg.speaker_gate && …` (recorder.rs:899) —
  so enabling the gate forces the slow batch path.

## Design

### Layer model (verification-based, zero new native deps)

A single decision over two profile sets:

- **accept-list** `Vec<SpeakerProfile>` — *our speakers*.
- **reject-list** `Vec<SpeakerProfile>` — voices to explicitly drop (the
  assistant's own TTS voice(s)).

`verify_admit(accept, reject, pcm, embedder) -> AdmitDecision`:

1. If a **reject** profile matches the utterance → `RejectSelf` (drop).
2. Else if accept-list is empty → `Admit` (only self-rejection active).
3. Else admit iff **any** accept profile matches → `Admit`; else `RejectUnknown`.
4. Unmeasurable audio (no voiced frames) → `Admit` (fail open — never swallow
   real speech on a measurement failure).

This one function covers all three jobs: self-voice (reject-list), other
people/TV (accept-list miss), our speakers (accept-list hit).

Ownership: the pure decision lives in `speaker.rs` (domain, unit-testable). The
`SpeakerGate` in `recorder.rs` owns *loading* the sets from config/disk and the
embedder, and delegates the decision. `VoiceConfig` owns config. The VAD loop
owns *when* to gate. No god object.

### Storage (backward compatible)

- accept profiles: `<profile_dir>/speakers/*.json` **plus** the legacy
  `speaker_profile.json` (loaded as one accept profile).
- reject profiles: `<profile_dir>/reject/*.json`.
- `<profile_dir>` derives from `cfg.speaker_profile_path`'s parent, else
  `<config>/jfc/voice/`. No new config fields required for v1.

### Self-voice reject enrollment

We control the TTS audio. `SpeakerProfile::enroll_from_pcm` already builds a
voiceprint from PCM, so a reject-profile is enrolled from a sample of the TTS
voice (captured from real read-aloud playback, or a one-off synthesized sample),
saved under `reject/<tts_voice>.json`. Keyed by `cfg.tts_voice` so each Anthropic
voice (buttery/airy/mellow/glassy/rounded) gets its own reject-profile.

### Streaming-path gating

- Remove `&& !cfg.speaker_gate` from `want_stream` so streaming + gate coexist.
- After speech-end, gate the **full captured `utterance_buf`** uniformly (both
  paths accumulate it). On reject: emit an empty `Interim` to clear the live
  preview from the box, emit **no** `Final` (no auto-submit), keep listening.
- The blunt `echo_guard` stays as the cheap first line; the reject-profile is the
  acoustic backstop.

## Implementation order

1. `speaker.rs`: `AdmitDecision` + `verify_admit` + dir-load helpers + tests.
2. `recorder.rs`: `SpeakerGate` holds accept/reject sets; `is_active()`; `admits`
   via `verify_admit`; `enroll_speaker(name)` + reject-profile save.
3. `recorder.rs` VAD loop: enable streaming with gate; gate full PCM uniformly;
   clear interim + suppress Final on reject.
4. build/test/clippy `-p jfc-voice`.

## Out of scope (heavier follow-ups)

- WebRTC AEC3 reference-signal linear echo cancellation (native dep).
- Personal-VAD / VoiceFilter target-speaker extraction for overlapping speech.
- Auto-capturing the TTS reject-profile from live playback wiring + UI command.
