//! JFC voice mode — push-to-talk / hands-free speech-to-text pipeline.
//!
//! ## Live streaming architecture (CC 2.1.177 parity)
//!
//! ```text
//! User holds Space (hold) / taps (tap) / just speaks (vad)
//!   → AudioCapture (arecord/sox/ffmpeg) → raw 16kHz/mono/S16LE PCM chunks
//!   → [hold/tap] stream_record: buffer-until-ready → coalesced 32KB flush
//!                → live WS frames → interim transcripts type in place
//!                → finalize (CloseStream + endpoint/timeout) → silent-drop replay
//!   → VoiceTranscriptEvent::{Level,Interim,Final} → engine bus → TUI
//! ```
//!
//! The Anthropic path is a faithful port of 2.1.177's `connectVoiceStream` and
//! its recording hook (see [`anthropic_ws`] / [`stream_record`]): real-time
//! binary PCM frames, `KeepAlive`/`CloseStream` control, interim promotion, and
//! the silent-drop replay. The OAuth token is supplied by the embedding app via
//! a [`TokenProvider`] so this crate stays provider-neutral.
//!
//! ## STT backends (priority order)
//!
//! 1. **Anthropic WebSocket** — `wss://<api>/api/ws/speech_to_text/voice_stream`.
//!    Requires a Claude.ai OAuth token. Streams audio in real time with live
//!    interim transcripts. Used for hold/tap dictation and (batch-style, via
//!    [`anthropic_ws::transcribe_pcm`]) for VAD utterances.
//!
//! 2. **OpenAI Whisper API** — `https://api.openai.com/v1/audio/transcriptions`.
//!    Sends the full WAV after recording stops when an API key is configured.
//!
//! 3. **Local whisper.cpp** — shells out to `whisper-cpp` or `whisper` binary.
//!    Works fully offline. Sends the WAV file as stdin or a temp file.
//!
//! ## Audio capture
//!
//! 1. `arecord -f S16_LE -r 16000 -c 1` (Linux ALSA)
//! 2. `rec -r 16000 -c 1 -e signed -b 16` (SoX)
//! 3. `ffmpeg -f alsa -i default -ar 16000 -ac 1 -f s16le` (ffmpeg)

pub mod anthropic_ws;
pub mod audio;
mod auth_endpoint;
pub mod backends;
pub mod config;
pub mod conversation_session;
pub mod conversation_ws;
pub mod doctor;
#[cfg(feature = "vad-neural")]
pub mod neural_vad;
pub mod platform;
pub mod playback;
pub mod recorder;
pub mod speaker;
pub mod stream_record;
pub mod streaming_tts;
pub mod tts;
pub mod vad;

pub use audio::AudioCapture;
pub use config::{VadEngine, VoiceConfig, VoiceMode};
pub use doctor::{Verdict, VoiceDiagnostic, format_report, run_diagnostic};
#[cfg(feature = "vad-neural")]
pub use neural_vad::NeuralVad;
pub use recorder::{
    TokenProvider, VoiceRecorder, VoiceState, VoiceTranscriptEvent, default_speaker_profile_path,
    enroll_primary_speaker, enroll_self_voice, enroll_speaker, reject_profile_path,
    save_reject_profile_from_pcm,
};
pub use speaker::{AdmitDecision, MatchScore, SpeakerProfile, verify_admit};
