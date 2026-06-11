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
//!
//! ## Recurrent-state drift on a long idle stream (the "go quiet and it stops
//! detecting" bug)
//!
//! Silero is recurrent: each `predict()` updates a 2×1×128 LSTM state that is
//! carried into the next call (see the bundled `voice_activity_detector` crate
//! — `predict()` reads `self.state` and writes back `stateN`). The model is
//! only ever *reset* explicitly. Streaming it for minutes of continuous silence
//! without a reset lets that hidden state accumulate/saturate, which biases the
//! probability for the chunks that follow — so when you finally speak again
//! after a long quiet stretch the onset can be missed or badly delayed. This is
//! the classic RNN long-sequence hidden-state-drift / saturation failure mode,
//! and it's exactly why the Silero maintainers say the state "should be reset
//! each time the stream changes or ends" and why Pipecat's `SileroVADAnalyzer`
//! force-resets the model on a fixed wall-clock cadence
//! (`_MODEL_RESET_STATES_TIME = 5.0` s) regardless of activity.
//!
//! We adopt the same defense: while **idle** (not mid-utterance) we reset the
//! model after a run of sustained non-speech chunks
//! (`JFC_VAD_NEURAL_IDLE_RESET_MS`, default 5000 ms). The reset is gated to the
//! idle/non-speech state so it can never wipe context mid-word — resetting
//! during speech is itself a known source of mid-utterance glitches.
//!
//! ## What this model can and cannot do (background voices / "movies in the
//! background get detected")
//!
//! Silero answers one question: *"is there human speech in this 32 ms window?"*
//! It does **not** answer *"is this the primary near-field speaker, or a voice
//! from the TV behind them?"* — both are real speech and both score high. No
//! single-channel VAD (energy or neural) can separate your voice from a
//! background talker at the same loudness/pitch; the periodicity/flatness gates
//! in the energy VAD reject *non-speech* noise (fans/AC), not *competing
//! speech*. Suppressing background human voices is a different problem —
//! *background voice cancellation* (BVC) / target-speaker extraction, which is
//! what Krisp ships as a separately trained source-separation model that keeps
//! only the enrolled/primary speaker. That is out of scope for this detector;
//! the practical mitigations here are a close-talking mic, push-to-talk/hold
//! mode in noisy rooms, or raising the onset threshold.

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
    /// Minimum voiced chunks captured in a segment before SpeechEnd may fire
    /// (anti-truncation, mirrors the energy VAD's `min_speech_frames`). Stops a
    /// single stray low-probability chunk just after onset from truncating the
    /// very start of a sentence.
    min_speech_chunks: usize,
    /// While idle, reset the Silero LSTM state after this many consecutive
    /// non-speech chunks, to stop recurrent-state drift over a long quiet
    /// stream (0 disables). See the module docs.
    idle_reset_chunks: usize,

    // State
    consecutive_voiced: usize,
    consecutive_silent: usize,
    in_speech: bool,
    /// Voiced chunks accumulated in the current segment (anti-truncation).
    speech_chunks: usize,
    /// Non-speech chunks seen while idle since the last model reset; drives the
    /// idle state-reset that prevents long-idle recurrent drift.
    idle_silent_chunks: usize,
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
        //
        // NOTE: this fixed tail-silence is intentionally generous. A short
        // hangover ends the turn on any within-utterance pause — a breath, a
        // mid-thought "stanza", a hesitation — which is the well-documented
        // weakness of pure silence-based endpointing (silences *inside* a turn
        // are routinely longer than the gaps *between* turns; see Sacks/Levinson
        // turn-taking and Skantze's VAP work). The real fix for cutting that
        // latency without truncating speakers is a semantic end-of-turn model,
        // which is a separate, much larger feature; here we keep a forgiving
        // hangover and expose it via env so it can be tuned per speaking style.
        let silence_ms = env_u32("JFC_VAD_SILENCE_MS", 1000);
        let silence_chunks = (silence_ms / CHUNK_MS).max(1) as usize;
        // ~96ms of voiced audio (3 chunks) to confirm onset, rejecting clicks.
        let onset_chunks = 3;

        // Anti-truncation guard, mirrors the energy VAD's `min_speech_frames`
        // (~200ms). Override via JFC_VAD_MIN_SPEECH_MS (shared with energy VAD).
        let min_speech_ms = env_u32("JFC_VAD_MIN_SPEECH_MS", 200);
        let min_speech_chunks = (min_speech_ms / CHUNK_MS).max(1) as usize;

        // Idle recurrent-state reset cadence. Default 5000ms matches Pipecat's
        // `_MODEL_RESET_STATES_TIME`. Set to 0 to disable.
        let idle_reset_ms = env_u32("JFC_VAD_NEURAL_IDLE_RESET_MS", 5000);
        let idle_reset_chunks = (idle_reset_ms / CHUNK_MS) as usize;

        Ok(Self {
            detector,
            onset_threshold,
            offset_threshold,
            onset_chunks,
            silence_chunks,
            min_speech_chunks,
            idle_reset_chunks,
            consecutive_voiced: 0,
            consecutive_silent: 0,
            in_speech: false,
            speech_chunks: 0,
            idle_silent_chunks: 0,
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
            self.idle_silent_chunks = 0;
        } else {
            self.consecutive_silent += 1;
            self.consecutive_voiced = 0;
            // Recurrent-state-drift defense: while idle, count sustained
            // non-speech and periodically reset the model's LSTM state so a long
            // quiet stretch can't bias the next onset (the "go quiet and it
            // stops detecting" bug). Gated to idle so it never wipes context
            // mid-utterance.
            if !self.in_speech && self.idle_reset_chunks > 0 {
                self.idle_silent_chunks += 1;
                if self.idle_silent_chunks >= self.idle_reset_chunks {
                    self.detector.reset();
                    self.idle_silent_chunks = 0;
                }
            }
        }

        if !self.in_speech && self.consecutive_voiced >= self.onset_chunks {
            self.in_speech = true;
            self.speech_chunks = self.consecutive_voiced;
            self.idle_silent_chunks = 0;
            return Some(VadEvent::SpeechStart);
        }
        if self.in_speech {
            self.speech_chunks += 1;
            // Anti-truncation: require a minimum amount of captured speech
            // before honoring an end-of-utterance, so a stray low-probability
            // chunk right after onset can't truncate the start of a sentence.
            if self.consecutive_silent >= self.silence_chunks
                && self.speech_chunks >= self.min_speech_chunks
            {
                self.in_speech = false;
                self.consecutive_voiced = 0;
                self.speech_chunks = 0;
                return Some(VadEvent::SpeechEnd);
            }
        }
        None
    }

    /// Force a SpeechEnd if we're currently in speech (e.g. PTT release).
    pub fn force_end(&mut self) -> bool {
        if self.in_speech {
            self.in_speech = false;
            self.consecutive_voiced = 0;
            self.consecutive_silent = 0;
            self.speech_chunks = 0;
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
        self.speech_chunks = 0;
        self.idle_silent_chunks = 0;
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

    /// Drive the state machine with a synthetic high-confidence probability,
    /// independent of model output, so onset/endpoint logic is deterministic.
    fn voiced_prob(vad: &mut NeuralVad) -> Option<VadEvent> {
        vad.process_probability(0.95)
    }
    fn silent_prob(vad: &mut NeuralVad) -> Option<VadEvent> {
        vad.process_probability(0.01)
    }

    // REGRESSION (long-idle recurrent-state drift): while idle, the Silero LSTM
    // state must be reset after a sustained run of non-speech so a long quiet
    // stretch can't bias the next onset ("go quiet and it stops detecting").
    // We assert the idle silence counter trips the reset cadence and zeroes.
    #[test]
    fn idle_reset_fires_after_sustained_silence_robust() {
        let mut vad = NeuralVad::new().expect("Silero model should load");
        vad.idle_reset_chunks = 5; // small cadence for the test
        // Feed silence chunks up to just before the reset boundary.
        for _ in 0..4 {
            assert_eq!(silent_prob(&mut vad), None);
        }
        assert_eq!(vad.idle_silent_chunks, 4, "should accumulate idle silence");
        // The 5th non-speech chunk hits the cadence and resets the counter.
        assert_eq!(silent_prob(&mut vad), None);
        assert_eq!(
            vad.idle_silent_chunks, 0,
            "idle reset must zero the silence counter at the cadence"
        );
        assert!(!vad.is_speaking(), "idle reset must not enter speech");
    }

    // The idle reset is gated to the idle state: once we're mid-utterance the
    // model state must NOT be reset on within-utterance pauses (that would wipe
    // the recurrent context mid-word).
    #[test]
    fn idle_reset_does_not_fire_during_speech_robust() {
        let mut vad = NeuralVad::new().expect("Silero model should load");
        vad.idle_reset_chunks = 3;
        vad.min_speech_chunks = 1;
        // Enter speech (3-chunk onset).
        for _ in 0..vad.onset_chunks {
            voiced_prob(&mut vad);
        }
        assert!(vad.is_speaking());
        // A within-utterance pause shorter than the hangover: idle_silent_chunks
        // must stay at zero because we're in_speech, not idle.
        for _ in 0..2 {
            silent_prob(&mut vad);
        }
        assert_eq!(
            vad.idle_silent_chunks, 0,
            "idle reset counter must not advance while in speech"
        );
    }

    // The idle reset cadence can be disabled (idle_reset_chunks == 0).
    #[test]
    fn idle_reset_disabled_when_zero_normal() {
        let mut vad = NeuralVad::new().expect("Silero model should load");
        vad.idle_reset_chunks = 0;
        for _ in 0..50 {
            silent_prob(&mut vad);
        }
        assert_eq!(
            vad.idle_silent_chunks, 0,
            "with reset disabled, the idle counter must never advance"
        );
    }

    // REGRESSION (anti-truncation): a stray low-probability chunk immediately
    // after onset must not fire SpeechEnd before the minimum utterance length,
    // so the start of a sentence can't be truncated — parity with the energy VAD.
    #[test]
    fn min_speech_guard_blocks_immediate_end_robust() {
        let mut vad = NeuralVad::new().expect("Silero model should load");
        vad.silence_chunks = 2; // easy to end if unguarded
        vad.min_speech_chunks = 10; // ~320ms guard
        // Just enough voiced chunks to start speech.
        for _ in 0..vad.onset_chunks {
            voiced_prob(&mut vad);
        }
        assert!(vad.is_speaking());
        // Immediate silence — without the guard this ends after 2 chunks.
        let ends = (0..5)
            .filter_map(|_| silent_prob(&mut vad))
            .filter(|e| *e == VadEvent::SpeechEnd)
            .count();
        assert_eq!(
            ends, 0,
            "min-speech guard must block a truncating SpeechEnd"
        );
    }

    // A genuine end-of-utterance (past the hangover, after enough speech) still
    // fires exactly one SpeechEnd — we didn't break endpointing.
    #[test]
    fn real_end_still_fires_after_hangover_normal() {
        let mut vad = NeuralVad::new().expect("Silero model should load");
        vad.silence_chunks = 3;
        vad.min_speech_chunks = 2;
        for _ in 0..vad.onset_chunks {
            voiced_prob(&mut vad);
        }
        // A few more voiced chunks to clear the min-speech guard.
        for _ in 0..3 {
            voiced_prob(&mut vad);
        }
        assert!(vad.is_speaking());
        let ends = (0..6)
            .filter_map(|_| silent_prob(&mut vad))
            .filter(|e| *e == VadEvent::SpeechEnd)
            .count();
        assert_eq!(
            ends, 1,
            "a real end-of-utterance must fire exactly one SpeechEnd"
        );
        assert!(!vad.is_speaking());
    }

    // REGRESSION (breath/stanza pause): a within-utterance pause shorter than
    // the hangover must NOT end the turn — the user pausing for breath or a
    // mid-thought "stanza" keeps one segment.
    #[test]
    fn within_utterance_pause_stays_one_segment_robust() {
        let mut vad = NeuralVad::new().expect("Silero model should load");
        vad.silence_chunks = 30; // ~1s hangover
        vad.min_speech_chunks = 2;
        for _ in 0..vad.onset_chunks {
            voiced_prob(&mut vad);
        }
        assert!(vad.is_speaking());
        // Several short breath pauses, each well under the hangover, interleaved
        // with speech — must produce zero SpeechEnd events.
        let mut ends = 0;
        for _ in 0..6 {
            for _ in 0..5 {
                ends += voiced_prob(&mut vad)
                    .iter()
                    .filter(|e| **e == VadEvent::SpeechEnd)
                    .count();
            }
            for _ in 0..10 {
                ends += silent_prob(&mut vad)
                    .iter()
                    .filter(|e| **e == VadEvent::SpeechEnd)
                    .count();
            }
        }
        assert_eq!(
            ends, 0,
            "short within-utterance pauses must not fire SpeechEnd"
        );
        assert!(
            vad.is_speaking(),
            "should still be mid-utterance after breath pauses"
        );
    }
}
