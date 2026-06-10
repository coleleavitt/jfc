//! Neural voice activity detection backed by the Silero VAD v5 ONNX model.
//!
//! This is the optional `vad-neural` backend. Where the default energy VAD
//! (`crate::vad::Vad`) infers speech from hand-tuned acoustic features
//! (energy + periodicity + modulation), Silero is a small recurrent network
//! trained on 6000+ languages that outputs a per-frame *speech probability*.
//! It is far more robust to the hard cases the energy detector can't fully
//! solve — sustained tonal noise (a motor whine), babble, and low-SNR rooms —
//! which is why production stacks (LiveKit, Pipecat, Krisp) use it.
//!
//! The model has a fixed window: at 16 kHz it consumes exactly **512 samples**
//! (32 ms) per inference step and carries an LSTM state across calls. The
//! recorder feeds us arbitrary PCM, so [`NeuralVad`] re-frames the byte stream
//! into 512-sample chunks internally and applies the same onset-debounce /
//! silence-hangover / hysteresis state machine as the energy VAD, but driven by
//! the model's probability instead of an energy threshold. This keeps the
//! [`VadEvent`] contract identical so the recorder is engine-agnostic.

use voice_activity_detector::VoiceActivityDetector;

use crate::vad::VadEvent;

/// Silero's fixed window at 16 kHz: 512 samples = 1024 bytes (16-bit) = 32 ms.
const CHUNK_SAMPLES: usize = 512;
const CHUNK_BYTES: usize = CHUNK_SAMPLES * 2;
const SAMPLE_RATE: i64 = 16_000;
/// Each chunk is 32 ms.
const CHUNK_MS: u32 = (CHUNK_SAMPLES as u32 * 1000) / 16_000;

/// Neural (Silero) VAD with the same event contract as the energy VAD.
pub struct NeuralVad {
    detector: VoiceActivityDetector,
    /// Probability ≥ this starts speech (high bar — hysteresis).
    onset_threshold: f32,
    /// Probability ≥ this keeps speech going (lower bar — hysteresis).
    offset_threshold: f32,
    /// Consecutive voiced chunks required before SpeechStart (debounce).
    onset_chunks: usize,
    /// Consecutive silent chunks required before SpeechEnd (hangover).
    silence_chunks: usize,

    // State
    consecutive_voiced: usize,
    consecutive_silent: usize,
    in_speech: bool,
    last_probability: f32,
    /// Leftover bytes that didn't fill a complete 512-sample chunk.
    leftover: Vec<u8>,
}

impl NeuralVad {
    /// Build the neural VAD. Returns an error if the ONNX session can't be
    /// created (e.g. the bundled model failed to load, or the ONNX Runtime
    /// shared library is missing under `load-dynamic`).
    pub fn new() -> anyhow::Result<Self> {
        let detector = VoiceActivityDetector::builder()
            .sample_rate(SAMPLE_RATE)
            .chunk_size(CHUNK_SAMPLES)
            .build()
            .map_err(|e| anyhow::anyhow!("failed to build Silero VAD: {e}"))?;

        // Speech-probability thresholds mirror Silero's own canonical defaults:
        // `threshold = 0.5` to enter speech and `neg_threshold = threshold -
        // 0.15 = 0.35` to leave it (see silero-vad utils_vad.get_speech_timestamps).
        // The gap is the hysteresis that stops a brief mid-word confidence dip
        // from ending the utterance. Both overridable via env.
        let onset_threshold = env_f32("JFC_VAD_NEURAL_ONSET", 0.5).clamp(0.05, 0.95);
        let offset_threshold = env_f32("JFC_VAD_NEURAL_OFFSET", 0.35).clamp(0.01, onset_threshold);

        // Silence hangover before declaring end-of-utterance. Matches the
        // energy VAD's ~1s default (Deepgram utterance_end_ms), expressed in
        // 32ms chunks. Override via JFC_VAD_SILENCE_MS (shared with energy VAD).
        let silence_ms = env_u32("JFC_VAD_SILENCE_MS", 1000);
        let silence_chunks = (silence_ms / CHUNK_MS).max(1) as usize;
        // ~96ms of voiced audio (3 chunks) to confirm onset, rejecting clicks.
        let onset_chunks = 3;

        Ok(Self {
            detector,
            onset_threshold,
            offset_threshold,
            onset_chunks,
            silence_chunks,
            consecutive_voiced: 0,
            consecutive_silent: 0,
            in_speech: false,
            last_probability: 0.0,
            leftover: Vec::new(),
        })
    }

    /// Push raw 16-bit signed LE PCM bytes (16 kHz mono). Returns any VAD
    /// events detected across the complete 512-sample chunks contained in the
    /// accumulated buffer.
    pub fn push(&mut self, pcm: &[u8]) -> Vec<VadEvent> {
        let mut events = Vec::new();
        let mut buf = std::mem::take(&mut self.leftover);
        buf.extend_from_slice(pcm);

        let mut pos = 0;
        while pos + CHUNK_BYTES <= buf.len() {
            let chunk = &buf[pos..pos + CHUNK_BYTES];
            pos += CHUNK_BYTES;
            let samples: Vec<i16> = chunk
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]))
                .collect();
            let probability = self.detector.predict(samples);
            self.last_probability = probability;
            if let Some(ev) = self.process_probability(probability) {
                events.push(ev);
            }
        }

        self.leftover = buf[pos..].to_vec();
        events
    }

    /// Run the onset/hangover/hysteresis state machine for one chunk's speech
    /// probability.
    fn process_probability(&mut self, probability: f32) -> Option<VadEvent> {
        // Hysteresis: while idle require the higher onset threshold; while in
        // speech the lower offset threshold keeps us going through brief dips.
        let voiced = if self.in_speech {
            probability >= self.offset_threshold
        } else {
            probability >= self.onset_threshold
        };

        if voiced {
            self.consecutive_voiced += 1;
            self.consecutive_silent = 0;
        } else {
            self.consecutive_silent += 1;
            self.consecutive_voiced = 0;
        }

        if !self.in_speech && self.consecutive_voiced >= self.onset_chunks {
            self.in_speech = true;
            return Some(VadEvent::SpeechStart);
        }
        if self.in_speech && self.consecutive_silent >= self.silence_chunks {
            self.in_speech = false;
            self.consecutive_voiced = 0;
            return Some(VadEvent::SpeechEnd);
        }
        None
    }

    /// Force a SpeechEnd if we're currently in speech (e.g. PTT release).
    pub fn force_end(&mut self) -> bool {
        if self.in_speech {
            self.in_speech = false;
            self.consecutive_voiced = 0;
            self.consecutive_silent = 0;
            true
        } else {
            false
        }
    }

    /// Whether the detector currently considers us mid-utterance.
    pub fn is_speaking(&self) -> bool {
        self.in_speech
    }

    /// Most recent chunk's speech probability (0..1), for diagnostics / meter.
    pub fn last_probability(&self) -> f32 {
        self.last_probability
    }

    /// A 0.0..1.0 level for the UI meter — the speech probability itself.
    pub fn level(&self) -> f32 {
        self.last_probability.clamp(0.0, 1.0)
    }

    /// Reset detection state and the model's recurrent state between utterances.
    pub fn reset(&mut self) {
        self.detector.reset();
        self.consecutive_voiced = 0;
        self.consecutive_silent = 0;
        self.in_speech = false;
        self.last_probability = 0.0;
        self.leftover.clear();
    }
}

fn env_f32(key: &str, default: f32) -> f32 {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn env_u32(key: &str, default: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn silent_chunk() -> Vec<u8> {
        vec![0u8; CHUNK_BYTES]
    }

    fn tone_chunk(freq_hz: f64, amplitude: i16) -> Vec<u8> {
        (0..CHUNK_SAMPLES)
            .flat_map(|i| {
                let t = i as f64 / SAMPLE_RATE as f64;
                let v = (amplitude as f64 * (std::f64::consts::TAU * freq_hz * t).sin()) as i16;
                v.to_le_bytes()
            })
            .collect()
    }

    /// Push `n` copies of `chunk` and report whether SpeechStart ever fired.
    fn ran_speech_start(vad: &mut NeuralVad, chunk: &[u8], n: usize) -> bool {
        (0..n).any(|_| vad.push(chunk).contains(&VadEvent::SpeechStart))
    }

    #[test]
    fn silence_never_triggers_speech_normal() {
        let mut vad = NeuralVad::new().expect("Silero model should load");
        let silent = silent_chunk();
        assert!(
            !ran_speech_start(&mut vad, &silent, 40),
            "silence must not trigger neural SpeechStart"
        );
    }

    #[test]
    fn silence_probability_is_low_normal() {
        let mut vad = NeuralVad::new().expect("Silero model should load");
        let silent = silent_chunk();
        for _ in 0..10 {
            vad.push(&silent);
        }
        assert!(
            vad.last_probability() < 0.5,
            "silence probability should be low, got {}",
            vad.last_probability()
        );
    }

    #[test]
    fn reframes_partial_chunks_without_panicking_robust() {
        let mut vad = NeuralVad::new().expect("Silero model should load");
        // Feed odd-sized buffers that don't align to 512-sample boundaries.
        let tone = tone_chunk(220.0, 8000);
        for split in [101usize, 333, 640, 17] {
            let part = &tone[..split.min(tone.len())];
            let _ = vad.push(part);
        }
        // No panic = pass; also confirm leftover handling kept us coherent.
        assert!(vad.last_probability() >= 0.0);
    }

    #[test]
    fn force_end_transitions_out_of_speech_robust() {
        let mut vad = NeuralVad::new().expect("Silero model should load");
        // Manually drive into the speech state to exercise force_end without
        // depending on model output.
        vad.in_speech = true;
        assert!(vad.force_end());
        assert!(!vad.is_speaking());
        assert!(!vad.force_end());
    }

    #[test]
    fn level_tracks_probability_normal() {
        let mut vad = NeuralVad::new().expect("Silero model should load");
        vad.push(&silent_chunk());
        let lvl = vad.level();
        assert!((0.0..=1.0).contains(&lvl));
    }
}
