//! JFC voice mode — push-to-talk speech-to-text pipeline.
//!
//! ## Architecture
//!
//! ```text
//! User holds Space
//!   → AudioCapture (arecord/sox/ffmpeg) → raw PCM buffer
//!   → SttBackend::transcribe(pcm) → String
//!   → EngineEvent::VoiceTranscript(text) → inject into textarea
//! ```
//!
//! ## STT backends (priority order)
//!
//! 1. **Anthropic WebSocket** — `wss://<api>/api/ws/speech_to_text/voice_stream`
//!    Requires Claude.ai OAuth token. Streams audio in real time, returns
//!    interim + final transcripts. Identical to CC 2.1.167.
//!
//! 2. **OpenAI Whisper API** — `https://api.openai.com/v1/audio/transcriptions`
//!    Requires `OPENAI_API_KEY`. Sends the full WAV after recording stops.
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
pub mod backends;
pub mod config;
pub mod doctor;
#[cfg(feature = "vad-neural")]
pub mod neural_vad;
pub mod platform;
pub mod recorder;
pub mod speaker;
pub mod vad;

pub use audio::AudioCapture;
pub use config::{VadEngine, VoiceConfig, VoiceMode};
pub use doctor::{Verdict, VoiceDiagnostic, format_report, run_diagnostic};
#[cfg(feature = "vad-neural")]
pub use neural_vad::NeuralVad;
pub use recorder::{VoiceRecorder, VoiceState, VoiceTranscriptEvent};
pub use speaker::{MatchScore, SpeakerProfile};
