//! Adaptive energy-based voice activity detection (VAD) with noise rejection.
//!
//! Watches an audio stream and emits [`VadEvent::SpeechStart`] /
//! [`VadEvent::SpeechEnd`] as you start and stop talking. The design mirrors
//! the techniques production VADs (WebRTC, Deepgram endpointing, fast-vad)
//! converge on, implemented dependency-free over raw PCM frames.
//!
//! ## Why more than an energy threshold
//!
//! A fixed energy threshold has two failure modes we hit in practice:
//!
//! - It needs hand-tuning per mic/room (too low → noise triggers it; too high
//!   → quiet speech is missed).
//! - Energy alone *cannot* tell your voice from a laptop fan or AC at the same
//!   loudness, so it keeps "recording" while the fan runs and never ends the
//!   utterance.
//!
//! ## The four layers
//!
//! 1. **Adaptive noise floor** — an exponential moving average of frame energy
//!    that only updates on non-speech frames, so the detector self-calibrates
//!    to the room instead of using a fixed number.
//! 2. **Hysteresis (double threshold)** — a high `onset` threshold to *start*
//!    speech, a lower `offset` threshold to *stay in* speech. The gap stops
//!    mid-word flicker.
//! 3. **Speech-vs-noise gates** (the fan fix) — to *start* a segment a loud
//!    frame must also (a) be **energy-modulated** (speech pulses at the ~4 Hz
//!    syllabic rate; a fan is flat) and (b) be **periodic** (voiced speech has
//!    a pitch; broadband noise does not).
//! 4. **Hangover** — speech only ends after `silence_frames` of sustained
//!    silence, so natural pauses ("um, like…") don't cut you off.
//!
//! ## Env overrides
//!
//! - `JFC_VAD_THRESHOLD` — fixed onset threshold; disables the adaptive floor
//!   and the speech-vs-noise gates (simple energy mode, for tests/back-compat).
//! - `JFC_VAD_SILENCE_MS` — silence before speech ends (default 1000).
//! - `JFC_VAD_NOISE_MARGIN` — onset = noise_floor × margin (default 3.0).
//! - `JFC_VAD_MIN_MODULATION` — min energy coefficient-of-variation (default 0.10; 0 disables).
//! - `JFC_VAD_MIN_PERIODICITY` — min autocorrelation peak to start (default 0.35; 0 disables).

use std::collections::VecDeque;

/// VAD events emitted during monitoring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VadEvent {
    /// Speech has started (enough consecutive voiced frames).
    SpeechStart,
    /// Speech has ended (enough consecutive silent frames after speech).
    SpeechEnd,
}

/// Frames of recent energy kept for the modulation test (~400ms @ 20ms).
const MODULATION_WINDOW: usize = 20;
/// Minimum frames of history before the modulation statistic is trusted.
/// Until then `energy_is_modulated` returns false, so a loud steady source
/// (a fan) can't sneak a false start through in the first few frames.
const MODULATION_MIN_FRAMES: usize = 6;

/// Adaptive energy-based VAD state machine.
pub struct Vad {
    /// Frames that must be voiced before triggering SpeechStart.
    speech_onset_frames: usize,
    /// Frames that must be silent before triggering SpeechEnd (the hangover).
    silence_frames: usize,
    /// Frame duration in samples (frame_bytes / 2 for 16-bit).
    frame_samples: usize,

    // ── Adaptive thresholding ──────────────────────────────────────────────
    /// Running noise-floor estimate (RMS), updated on non-speech frames.
    noise_floor: f64,
    /// onset = noise_floor × onset_margin (or fixed_threshold if set).
    onset_margin: f64,
    /// offset = noise_floor × offset_margin (continue-speech threshold).
    offset_margin: f64,
    /// EMA smoothing factor for the noise floor (0..1, smaller = slower).
    noise_alpha: f64,
    /// When `Some`, use this fixed onset threshold and skip adaptive logic +
    /// speech-vs-noise gates (simple energy mode; used by tests / manual tune).
    fixed_threshold: Option<u32>,
    /// True until the floor has been seeded by the first few frames.
    floor_seeded: bool,
    frames_seen: usize,

    // ── Speech-vs-noise discrimination ─────────────────────────────────────
    /// Recent per-frame RMS, for the energy-modulation test.
    rms_history: VecDeque<u32>,
    /// Minimum energy coefficient-of-variation to count as speech (0 = off).
    min_modulation: f64,
    /// Minimum autocorrelation-peak periodicity to *start* speech (0 = off).
    min_periodicity: f64,
    /// Maximum spectral flatness to *start* speech (0 = off, the default).
    /// Speech is spectrally peaky (flatness ≈ 0.1–0.4); broadband noise is flat
    /// (≈ 0.6–1.0). Opt-in via JFC_VAD_MAX_FLATNESS for environments where a
    /// loud *tonal* noise (a motor whine) defeats the periodicity gate.
    max_flatness: f64,
    /// Most recent frame periodicity score (for diagnostics).
    last_periodicity: f64,
    /// Most recent frame spectral flatness (for diagnostics).
    last_flatness: f64,
    /// Most recent frame zero-crossing rate (for diagnostics).
    last_zcr: f32,

    // ── Detection state ────────────────────────────────────────────────────
    consecutive_voiced: usize,
    consecutive_silent: usize,
    in_speech: bool,
    /// Voiced frames accumulated in the current segment. Used to enforce a
    /// minimum utterance length so a single stray quiet frame just after onset
    /// can't fire SpeechEnd before any real speech was captured.
    speech_frames: usize,
    /// Minimum voiced frames before SpeechEnd may fire (anti-truncation).
    min_speech_frames: usize,
    /// Most recent frame RMS — exposed for the UI level meter.
    last_rms: u32,
    /// Leftover bytes from the last push that didn't fill a complete frame.
    leftover: Vec<u8>,
}

impl Vad {
    /// Create with adaptive parameters (or a fixed threshold via env).
    pub fn new() -> Self {
        let frame_ms = 20; // 20ms frames
        let sample_rate = 16_000u32;
        let frame_samples = (sample_rate * frame_ms / 1000) as usize; // 320

        // Silence window before declaring end-of-utterance. 1000ms matches
        // Deepgram's `utterance_end_ms` default and rides through natural
        // between-word pauses. Override via JFC_VAD_SILENCE_MS.
        let silence_ms = env_u32("JFC_VAD_SILENCE_MS", 1000);
        let silence_frames =
            (silence_ms * sample_rate / 1000 / frame_samples as u32).max(1) as usize;

        // Optional fixed threshold (disables adaptive floor + noise gates).
        let fixed_threshold = std::env::var("JFC_VAD_THRESHOLD")
            .ok()
            .and_then(|s| s.parse::<u32>().ok());

        let onset_margin = env_f64("JFC_VAD_NOISE_MARGIN", 3.0).max(1.1);

        // Speech-vs-noise gates are on by default in adaptive mode, off when a
        // fixed threshold is pinned (back-compat / deterministic tests).
        let min_modulation = env_f64(
            "JFC_VAD_MIN_MODULATION",
            if fixed_threshold.is_some() { 0.0 } else { 0.10 },
        );
        // Real-speech calibration: measured single-frame (20ms, 320-sample)
        // autocorrelation peaks for actual conversational speech run ~0.19–0.65
        // (median ≈ 0.45), while broadband noise (fans, AC, chair scrapes) sits
        // near ~0.10. 0.35 catches ~95% of voice frames while staying well
        // above the noise band. (An earlier 0.55 was miscalibrated against
        // synthetic tones and rejected most real speech.)
        let min_periodicity = env_f64(
            "JFC_VAD_MIN_PERIODICITY",
            if fixed_threshold.is_some() { 0.0 } else { 0.35 },
        );

        // Minimum voiced frames before SpeechEnd may fire — anti-truncation
        // guard so a single stray quiet frame just after onset can't end the
        // utterance before any real speech is captured. ~200ms (10 × 20ms),
        // override via JFC_VAD_MIN_SPEECH_MS.
        let min_speech_ms = env_u32("JFC_VAD_MIN_SPEECH_MS", 200);
        let min_speech_frames =
            (min_speech_ms * sample_rate / 1000 / frame_samples as u32).max(1) as usize;

        Self {
            speech_onset_frames: 3,
            silence_frames,
            frame_samples,
            speech_frames: 0,
            min_speech_frames,
            // Seed the floor non-zero so the first frames don't all read as
            // speech before the EMA adapts.
            noise_floor: 150.0,
            onset_margin,
            offset_margin: onset_margin * 0.6, // hysteresis gap
            noise_alpha: 0.05,
            fixed_threshold,
            floor_seeded: false,
            frames_seen: 0,
            rms_history: VecDeque::with_capacity(MODULATION_WINDOW),
            min_modulation,
            min_periodicity,
            // Spectral-flatness gate is OFF by default (0.0): the periodicity +
            // modulation gates already handle fans/AC/chair noise, and a
            // miscalibrated flatness bar could reject real speech. Opt in via
            // JFC_VAD_MAX_FLATNESS (e.g. 0.5) for stubborn tonal noise.
            max_flatness: env_f64("JFC_VAD_MAX_FLATNESS", 0.0),
            last_periodicity: 0.0,
            last_flatness: 0.0,
            last_zcr: 0.0,
            consecutive_voiced: 0,
            consecutive_silent: 0,
            in_speech: false,
            last_rms: 0,
            leftover: Vec::new(),
        }
    }

    /// Push raw 16-bit signed PCM bytes. Returns any VAD events detected.
    pub fn push(&mut self, pcm: &[u8]) -> Vec<VadEvent> {
        let mut events = Vec::new();
        let mut buf = std::mem::take(&mut self.leftover);
        buf.extend_from_slice(pcm);

        let frame_bytes = self.frame_samples * 2;
        let mut pos = 0;
        while pos + frame_bytes <= buf.len() {
            let frame = &buf[pos..pos + frame_bytes];
            pos += frame_bytes;
            if let Some(ev) = self.process_frame(frame) {
                events.push(ev);
            }
        }

        self.leftover = buf[pos..].to_vec();
        events
    }

    /// Process one complete frame and return a state-transition event, if any.
    fn process_frame(&mut self, frame: &[u8]) -> Option<VadEvent> {
        let rms = rms_energy(frame);
        self.last_rms = rms;
        self.last_zcr = zero_crossing_rate(frame);
        self.frames_seen += 1;

        self.rms_history.push_back(rms);
        while self.rms_history.len() > MODULATION_WINDOW {
            self.rms_history.pop_front();
        }

        let (onset, offset) = self.thresholds();
        // Hysteresis: in-speech uses the lower offset threshold; idle uses the
        // higher onset threshold.
        let loud_enough = if self.in_speech {
            rms as f64 >= offset
        } else {
            rms as f64 >= onset
        };

        // Speech-vs-noise gates — the fan fix. Only applied to *starting*
        // speech (and skipped entirely in fixed-threshold mode). Once we're in
        // a segment the silence hangover handles ending, so quiet vowels /
        // unvoiced consonants inside a word don't get cut.
        let speech_like = if self.fixed_threshold.is_some() || self.in_speech {
            true
        } else {
            let modulated = self.min_modulation <= 0.0 || self.energy_is_modulated();
            let periodic = if self.min_periodicity <= 0.0 {
                true
            } else {
                self.last_periodicity = frame_periodicity(frame);
                self.last_periodicity >= self.min_periodicity
            };
            // Spectral-flatness gate (opt-in): a low flatness means a tonal,
            // speech-like spectrum; a high flatness means broadband/tonal noise.
            let tonal = if self.max_flatness <= 0.0 {
                true
            } else {
                self.last_flatness = spectral_flatness(frame);
                self.last_flatness <= self.max_flatness
            };
            modulated && periodic && tonal
        };

        let voiced = loud_enough && speech_like;

        if voiced {
            self.consecutive_voiced += 1;
            self.consecutive_silent = 0;
        } else {
            self.consecutive_silent += 1;
            self.consecutive_voiced = 0;
            // Adapt the noise floor only while IDLE (not mid-utterance). During
            // an active segment the natural between-word pauses are non-voiced;
            // adapting on them ratchets the floor up, which raises the `offset`
            // (stay-in-speech) threshold, which makes the next normal-volume
            // words read as silence and fires SpeechEnd mid-sentence — the
            // "cut off, transcribe, then catch up when I keep talking" bug.
            // Freezing the floor during speech matches the documented invariant
            // ("updates on non-speech frames") and how production VADs behave.
            if !self.in_speech {
                self.update_noise_floor(rms);
            }
        }

        if !self.in_speech && self.consecutive_voiced >= self.speech_onset_frames {
            self.in_speech = true;
            self.speech_frames = self.consecutive_voiced;
            return Some(VadEvent::SpeechStart);
        }
        if self.in_speech {
            self.speech_frames += 1;
            // Anti-truncation: require a minimum amount of captured speech
            // before honoring an end-of-utterance, so a stray quiet frame right
            // after onset can't truncate the very start of a sentence.
            if self.consecutive_silent >= self.silence_frames
                && self.speech_frames >= self.min_speech_frames
            {
                self.in_speech = false;
                self.consecutive_voiced = 0;
                self.speech_frames = 0;
                return Some(VadEvent::SpeechEnd);
            }
        }
        None
    }

    /// Whether the recent energy window fluctuates enough to be speech rather
    /// than steady noise. Coefficient of variation (stddev / mean): speech
    /// ≈ 0.3–1.0 (syllabic pulsing), a steady fan/AC ≈ < 0.1.
    ///
    /// Returns `false` until at least `MODULATION_MIN_FRAMES` of history exist,
    /// so a loud steady source (a fan) can't sneak a false start through in the
    /// first few frames before the statistic is meaningful. This adds ~100ms of
    /// onset latency, which is imperceptible.
    fn energy_is_modulated(&self) -> bool {
        let n = self.rms_history.len();
        if n < MODULATION_MIN_FRAMES {
            return false;
        }
        let mean = self.rms_history.iter().map(|&r| r as f64).sum::<f64>() / n as f64;
        if mean < 1.0 {
            return false;
        }
        let var = self
            .rms_history
            .iter()
            .map(|&r| {
                let d = r as f64 - mean;
                d * d
            })
            .sum::<f64>()
            / n as f64;
        (var.sqrt() / mean) >= self.min_modulation
    }

    /// Current (onset, offset) thresholds.
    fn thresholds(&self) -> (f64, f64) {
        if let Some(fixed) = self.fixed_threshold {
            // Manual mode: onset = fixed, offset = 60% of fixed (hysteresis).
            (fixed as f64, fixed as f64 * 0.6)
        } else {
            let onset = self.noise_floor * self.onset_margin;
            let offset = self.noise_floor * self.offset_margin;
            // Floor the onset so a near-silent room doesn't make faint sounds
            // count as speech.
            (onset.max(120.0), offset.max(72.0))
        }
    }

    /// Update the adaptive noise floor with an EMA over silent frames.
    fn update_noise_floor(&mut self, rms: u32) {
        let x = rms as f64;
        if !self.floor_seeded {
            // Seed quickly from the first ~10 silent frames, then settle into
            // the slow EMA so the floor tracks the room without lag.
            self.noise_floor = 0.7 * self.noise_floor + 0.3 * x;
            if self.frames_seen >= 10 {
                self.floor_seeded = true;
            }
        } else {
            self.noise_floor = (1.0 - self.noise_alpha) * self.noise_floor + self.noise_alpha * x;
        }
        self.noise_floor = self.noise_floor.clamp(20.0, 8000.0);
    }

    /// Force a SpeechEnd if we're currently in speech (e.g. on PTT release).
    pub fn force_end(&mut self) -> bool {
        if self.in_speech {
            self.in_speech = false;
            self.consecutive_voiced = 0;
            self.consecutive_silent = 0;
            self.speech_frames = 0;
            true
        } else {
            false
        }
    }

    /// Whether VAD currently considers us in a speech segment.
    pub fn is_speaking(&self) -> bool {
        self.in_speech
    }

    /// Most recent frame RMS energy (for a live level meter).
    pub fn last_rms(&self) -> u32 {
        self.last_rms
    }

    /// Most recent frame zero-crossing rate (0..1), for diagnostics.
    pub fn last_zcr(&self) -> f32 {
        self.last_zcr
    }

    /// Most recent frame periodicity score (0..1), for diagnostics.
    pub fn last_periodicity(&self) -> f64 {
        self.last_periodicity
    }

    /// Most recent frame spectral flatness (0..1), for diagnostics. Only
    /// updated when the flatness gate is enabled (JFC_VAD_MAX_FLATNESS).
    pub fn last_flatness(&self) -> f64 {
        self.last_flatness
    }

    /// Current adaptive noise floor estimate (for diagnostics / meter scaling).
    pub fn noise_floor(&self) -> u32 {
        self.noise_floor as u32
    }

    /// A 0.0..1.0 voice level for the UI meter, scaled between the noise floor
    /// and a typical speech ceiling so quiet voices still move the bar.
    pub fn level(&self) -> f32 {
        let (onset, _) = self.thresholds();
        let floor = onset.min(self.last_rms as f64);
        let ceil = (onset * 8.0).max(floor + 1.0);
        (((self.last_rms as f64 - floor) / (ceil - floor)).clamp(0.0, 1.0)) as f32
    }

    /// Reset detection state (e.g. between utterances). Keeps the learned
    /// noise floor so the next utterance benefits from prior adaptation.
    pub fn reset(&mut self) {
        self.consecutive_voiced = 0;
        self.consecutive_silent = 0;
        self.in_speech = false;
        self.speech_frames = 0;
        self.last_rms = 0;
        self.last_zcr = 0.0;
        self.last_periodicity = 0.0;
        self.rms_history.clear();
        self.leftover.clear();
    }
}

impl Default for Vad {
    fn default() -> Self {
        Self::new()
    }
}

fn env_u32(key: &str, default: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

/// Zero-crossing rate of a 16-bit LE PCM frame: fraction of adjacent sample
/// pairs whose sign changes. Returns 0.0..=1.0. Low for hum, high for hiss.
pub fn zero_crossing_rate(pcm_bytes: &[u8]) -> f32 {
    let samples: Vec<i16> = pcm_bytes
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    if samples.len() < 2 {
        return 0.0;
    }
    let crossings = samples
        .windows(2)
        .filter(|w| (w[0] >= 0) != (w[1] >= 0))
        .count();
    crossings as f32 / (samples.len() - 1) as f32
}

/// Periodicity of a 16-bit LE PCM frame, in `0.0..=1.0`.
///
/// Peak of the **normalized** autocorrelation over lags for the human pitch
/// range (80–400 Hz at 16 kHz → lags 40–200). Voiced speech is quasi-periodic
/// and scores high; broadband noise (fans, AC, hiss) is aperiodic and scores
/// low. Because the autocorrelation is normalized by the frame's own energy,
/// this score is **amplitude-invariant** — the same voiced vowel scores the
/// same whether spoken loudly or softly, which is the property that lets the
/// detector accept a quiet/changing-tone voice without retuning a hardcoded
/// energy level. See [`harmonics_to_noise_ratio`] for the dB form.
pub fn frame_periodicity(pcm_bytes: &[u8]) -> f64 {
    let samples: Vec<f64> = pcm_bytes
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f64)
        .collect();
    let n = samples.len();
    const MIN_LAG: usize = 40; // 16000 / 400 Hz
    const MAX_LAG: usize = 200; // 16000 / 80 Hz
    if n <= MAX_LAG + 1 {
        return 0.0;
    }
    let energy0: f64 = samples.iter().map(|&s| s * s).sum();
    if energy0 <= f64::EPSILON {
        return 0.0;
    }
    let mut best = 0.0f64;
    for lag in MIN_LAG..=MAX_LAG {
        let mut acc = 0.0f64;
        for i in 0..(n - lag) {
            acc += samples[i] * samples[i + lag];
        }
        let norm = acc / energy0;
        if norm > best {
            best = norm;
        }
    }
    best.clamp(0.0, 1.0)
}

/// Harmonics-to-noise ratio (HNR) of a 16-bit LE PCM frame, in decibels.
///
/// This is the Boersma (1993) autocorrelation estimator used by Praat. Given
/// the *normalized* autocorrelation peak `r = frame_periodicity(frame)` (the
/// fraction of the frame that is periodic), the harmonic and noise energy split
/// as `r` and `1 − r`, so
///
/// ```text
/// HNR_dB = 10 · log10( r / (1 − r) )
/// ```
///
/// Why it matters here: because `r` is normalized by the frame's own energy,
/// **HNR is independent of how loud or quiet the voice is** — it measures signal
/// *structure* (harmonic vs. aperiodic), not level. That is exactly the
/// intensity-invariant "is this a legitimate voice at any tone/volume?" property
/// a raw energy threshold lacks: a soft vowel and a loud vowel both yield high
/// HNR, while white/broadband noise (fans, hiss) yields a very low or negative
/// HNR regardless of loudness. Clean voiced speech typically sits around
/// ~7–20 dB; broadband noise sits well below 0 dB.
///
/// Returned as a diagnostic / optional voicing feature; see `VAD_RESEARCH.md`
/// (§5) for the proposed gate. Saturates to ±40 dB so a perfectly periodic or
/// perfectly aperiodic frame doesn't return infinities.
pub fn harmonics_to_noise_db(pcm_bytes: &[u8]) -> f64 {
    let r = frame_periodicity(pcm_bytes).clamp(0.0, 1.0);
    // Guard the endpoints so log10 stays finite.
    if r <= 1e-4 {
        return -40.0;
    }
    if r >= 1.0 - 1e-4 {
        return 40.0;
    }
    (10.0 * (r / (1.0 - r)).log10()).clamp(-40.0, 40.0)
}

/// Spectral flatness measure (Wiener entropy) of a 16-bit LE PCM frame,
/// returned in `0.0..=1.0`.
///
/// Defined as the ratio of the geometric mean to the arithmetic mean of the
/// power spectrum: `exp(mean(ln S)) / mean(S)`. A flat (white-noise-like)
/// spectrum → near 1.0; a tonal/voiced spectrum with energy concentrated in a
/// few harmonics → near 0.0.
///
/// This is the discriminator rVAD (Tan et al. 2019) and Moattar & Homayounpour
/// use to separate speech from noise: it complements time-domain periodicity by
/// catching steady *tonal* noise (a motor whine) that autocorrelation alone
/// would mistake for voiced speech — such a tone is periodic but spectrally
/// *peaky in the wrong band*, and broadband noise is flat. Computed with a
/// dependency-free radix-2 DFT over the frame (zero-padded to the next power of
/// two), so no FFT crate is required.
pub fn spectral_flatness(pcm_bytes: &[u8]) -> f64 {
    let samples: Vec<f64> = pcm_bytes
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f64)
        .collect();
    let n = samples.len();
    if n < 8 {
        return 0.0;
    }

    // Next power of two ≥ n for the radix-2 DFT.
    let mut fft_len = 1usize;
    while fft_len < n {
        fft_len <<= 1;
    }

    // Hann window reduces spectral leakage so the flatness estimate is stable.
    let mut re = vec![0.0f64; fft_len];
    let mut im = vec![0.0f64; fft_len];
    for (i, &s) in samples.iter().enumerate() {
        let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / (n.max(2) - 1) as f64).cos();
        re[i] = s * w;
    }

    dft_in_place(&mut re, &mut im);

    // Power spectrum over the positive-frequency bins (skip DC). Add a tiny
    // floor so ln() of a zero bin doesn't blow up.
    let half = fft_len / 2;
    let mut sum_log = 0.0f64;
    let mut sum_lin = 0.0f64;
    let mut count = 0usize;
    for k in 1..half {
        let power = re[k] * re[k] + im[k] * im[k] + 1e-9;
        sum_log += power.ln();
        sum_lin += power;
        count += 1;
    }
    if count == 0 || sum_lin <= 0.0 {
        return 0.0;
    }
    let geo = (sum_log / count as f64).exp();
    let arith = sum_lin / count as f64;
    (geo / arith).clamp(0.0, 1.0)
}

/// In-place iterative radix-2 Cooley–Tukey FFT. `re`/`im` must be the same
/// power-of-two length. No external dependency; used only for the per-frame
/// spectral-flatness estimate (frame ≤ 512 samples, so this is cheap).
fn dft_in_place(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    debug_assert!(n.is_power_of_two());
    // Bit-reversal permutation.
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    // Danielson–Lanczos butterflies.
    let mut len = 2usize;
    while len <= n {
        let ang = -std::f64::consts::TAU / len as f64;
        let (wlen_cos, wlen_sin) = (ang.cos(), ang.sin());
        let mut i = 0;
        while i < n {
            let (mut wr, mut wi) = (1.0f64, 0.0f64);
            for k in 0..len / 2 {
                let u_re = re[i + k];
                let u_im = im[i + k];
                let v_re = re[i + k + len / 2] * wr - im[i + k + len / 2] * wi;
                let v_im = re[i + k + len / 2] * wi + im[i + k + len / 2] * wr;
                re[i + k] = u_re + v_re;
                im[i + k] = u_im + v_im;
                re[i + k + len / 2] = u_re - v_re;
                im[i + k + len / 2] = u_im - v_im;
                let next_wr = wr * wlen_cos - wi * wlen_sin;
                wi = wr * wlen_sin + wi * wlen_cos;
                wr = next_wr;
            }
            i += len;
        }
        len <<= 1;
    }
}

/// Root-mean-square energy of a 16-bit LE PCM byte buffer.
/// Returns a value in [0, 32767].
pub fn rms_energy(pcm_bytes: &[u8]) -> u32 {
    if pcm_bytes.len() < 2 {
        return 0;
    }
    let sum_sq: u64 = pcm_bytes
        .chunks_exact(2)
        .map(|b| {
            let s = i16::from_le_bytes([b[0], b[1]]) as i64;
            (s * s) as u64
        })
        .sum();
    let mean_sq = sum_sq / (pcm_bytes.len() as u64 / 2);
    (mean_sq as f64).sqrt() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn silent_frame(samples: usize) -> Vec<u8> {
        vec![0u8; samples * 2]
    }

    /// Square wave at `amplitude` — loud but maximally periodic, used for the
    /// fixed-threshold (gate-off) tests.
    fn loud_frame(samples: usize, amplitude: i16) -> Vec<u8> {
        (0..samples)
            .flat_map(|i| {
                let v: i16 = if i % 2 == 0 { amplitude } else { -amplitude };
                v.to_le_bytes()
            })
            .collect()
    }

    /// A pitched, modulated, voiced-like frame at the given lag (pitch period)
    /// and amplitude — high periodicity, used for adaptive-mode speech tests.
    fn voiced_frame(samples: usize, lag: usize, amplitude: i16) -> Vec<u8> {
        (0..samples)
            .flat_map(|i| {
                // Triangle-ish periodic wave at `lag` samples/period.
                let phase = (i % lag) as f64 / lag as f64;
                let v = (amplitude as f64) * (2.0 * phase - 1.0);
                (v as i16).to_le_bytes()
            })
            .collect()
    }

    /// Loud but *aperiodic* broadband noise — a faithful model of a fan / AC /
    /// hiss / chair-scrape. High energy, no pitch peak. Uses a high-quality
    /// PRNG (SplitMix64) so successive samples are uncorrelated (real white
    /// noise scores ~0.10 periodicity; a weak PRNG can leave spurious structure).
    fn noise_frame(samples: usize, amplitude: i16, seed: u64) -> Vec<u8> {
        let mut state = seed.wrapping_add(0x9E3779B97F4A7C15);
        let mut next = || {
            // SplitMix64
            state = state.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        };
        (0..samples)
            .flat_map(|_| {
                let span = 2 * amplitude as i64 + 1;
                let r = (next() % span as u64) as i64 - amplitude as i64;
                (r as i16).to_le_bytes()
            })
            .collect()
    }

    /// Push `frames` copies and report whether SpeechStart ever fired.
    fn ran_speech_start(vad: &mut Vad, frame: &[u8], frames: usize) -> bool {
        (0..frames).any(|_| vad.push(frame).contains(&VadEvent::SpeechStart))
    }

    /// Seed modulation history with quiet frames, then push a continuous run of
    /// loud frames; report whether SpeechStart fired. Models real speech onset.
    fn ran_seeded_speech_start(
        vad: &mut Vad,
        loud: &[u8],
        quiet: &[u8],
        loud_frames: usize,
    ) -> bool {
        for _ in 0..6 {
            vad.push(quiet);
        }
        ran_speech_start(vad, loud, loud_frames)
    }

    /// Build a VAD with a fixed threshold so loudness-only tests are
    /// deterministic and independent of the adaptive gates.
    fn fixed_vad(threshold: u32) -> Vad {
        let mut v = Vad::new();
        v.fixed_threshold = Some(threshold);
        v.min_modulation = 0.0;
        v.min_periodicity = 0.0;
        v.silence_frames = 5;
        // Keep the min-utterance guard small for the short loudness tests so it
        // doesn't mask the behavior they assert (they push ~5 loud frames).
        v.min_speech_frames = 1;
        v
    }

    #[test]
    fn rms_silent_is_zero_normal() {
        assert_eq!(rms_energy(&silent_frame(320)), 0);
    }

    #[test]
    fn rms_loud_above_threshold_normal() {
        assert!(rms_energy(&loud_frame(320, 5000)) > 300);
    }

    #[test]
    fn zcr_silent_is_zero_normal() {
        assert_eq!(zero_crossing_rate(&silent_frame(320)), 0.0);
    }

    #[test]
    fn zcr_alternating_is_high_normal() {
        assert!(zero_crossing_rate(&loud_frame(320, 5000)) > 0.9);
    }

    #[test]
    fn periodicity_high_for_pitched_signal_normal() {
        // A clean periodic wave at a pitch lag should score high.
        let p = frame_periodicity(&voiced_frame(320, 80, 6000));
        assert!(p > 0.5, "pitched signal should be periodic, got {p}");
    }

    #[test]
    fn periodicity_low_for_broadband_noise_robust() {
        // Aperiodic noise (a fan) should score low — this is what lets the VAD
        // distinguish it from a voice at the same loudness.
        let p = frame_periodicity(&noise_frame(320, 6000, 7));
        assert!(p < 0.5, "broadband noise should be aperiodic, got {p}");
    }

    #[test]
    fn vad_detects_speech_start_normal() {
        let mut vad = fixed_vad(300);
        let loud = loud_frame(320, 5000);
        let events: Vec<VadEvent> = (0..5).flat_map(|_| vad.push(&loud)).collect();
        assert!(events.contains(&VadEvent::SpeechStart));
    }

    #[test]
    fn vad_detects_speech_end_after_silence_normal() {
        let mut vad = fixed_vad(300);
        let loud = loud_frame(320, 5000);
        for _ in 0..5 {
            vad.push(&loud);
        }
        let silent = silent_frame(320);
        let events: Vec<VadEvent> = (0..10).flat_map(|_| vad.push(&silent)).collect();
        assert!(events.contains(&VadEvent::SpeechEnd));
    }

    #[test]
    fn vad_no_speech_end_without_start_robust() {
        let mut vad = fixed_vad(300);
        let silent = silent_frame(320);
        let events: Vec<VadEvent> = (0..100).flat_map(|_| vad.push(&silent)).collect();
        assert!(!events.contains(&VadEvent::SpeechEnd));
    }

    #[test]
    fn hysteresis_keeps_speech_through_quiet_dip_normal() {
        let mut vad = fixed_vad(300); // onset 300, offset 180
        let loud = loud_frame(320, 5000);
        for _ in 0..5 {
            vad.push(&loud);
        }
        assert!(vad.is_speaking());
        // RMS ~200: below onset(300) but above offset(180) → stays in speech.
        let dip = loud_frame(320, 200);
        let events: Vec<VadEvent> = (0..3).flat_map(|_| vad.push(&dip)).collect();
        assert!(!events.contains(&VadEvent::SpeechEnd));
        assert!(vad.is_speaking());
    }

    #[test]
    fn adaptive_floor_rises_with_background_noise_normal() {
        let mut vad = Vad::new();
        vad.fixed_threshold = None;
        let noise = loud_frame(320, 300);
        for _ in 0..50 {
            vad.push(&noise);
        }
        assert!(
            vad.noise_floor() > 150,
            "floor should rise with sustained noise"
        );
    }

    #[test]
    fn adaptive_rejects_loud_fan_noise_normal() {
        // THE fan fix: loud but aperiodic broadband noise must NOT start speech
        // in adaptive mode, even though it clears any energy threshold.
        let mut vad = Vad::new();
        vad.fixed_threshold = None;
        // Vary the seed each frame so the noise is genuinely non-repeating.
        let started = (0..80).any(|i| {
            let fan = noise_frame(320, 5000, i as u64);
            vad.push(&fan).contains(&VadEvent::SpeechStart)
        });
        assert!(
            !started,
            "loud steady fan noise must not be treated as speech"
        );
    }

    #[test]
    fn adaptive_accepts_modulated_voiced_speech_normal() {
        // Pulsing, pitched frames (loud voiced / quiet gaps) carry both
        // periodicity and syllabic modulation → detected as speech.
        let mut vad = Vad::new();
        vad.fixed_threshold = None;
        let loud = voiced_frame(320, 80, 7000);
        let quiet = silent_frame(320);
        assert!(
            ran_seeded_speech_start(&mut vad, &loud, &quiet, 10),
            "modulated voiced speech should be detected"
        );
    }

    #[test]
    fn adaptive_rejects_single_frame_clunks_normal() {
        // A single loud aperiodic frame (a clunk) followed by quiet, repeated,
        // must never satisfy the 3-frame onset debounce → no speech start.
        let mut vad = Vad::new();
        vad.fixed_threshold = None;
        let quiet = silent_frame(320);
        let frames: Vec<Vec<u8>> = (0..40)
            .flat_map(|i| [noise_frame(320, 8000, (i * 31 + 5) as u64), quiet.clone()])
            .collect();
        let started = frames
            .iter()
            .any(|f| vad.push(f).contains(&VadEvent::SpeechStart));
        assert!(!started, "single-frame clunks must not start speech");
    }

    #[test]
    fn voiced_speech_beats_fan_noise_floor_normal() {
        // After a fan raises the noise floor, genuine voiced speech (periodic,
        // louder, modulated) should still trigger SpeechStart.
        let mut vad = Vad::new();
        vad.fixed_threshold = None;
        for i in 0..50 {
            vad.push(&noise_frame(320, 3000, i as u64)); // learn the fan floor
        }
        let loud = voiced_frame(320, 80, 9000);
        let quiet = silent_frame(320);
        assert!(
            ran_seeded_speech_start(&mut vad, &loud, &quiet, 10),
            "voiced speech should rise above a learned fan floor"
        );
    }

    #[test]
    fn fan_noise_does_not_trigger_speech_normal() {
        // Steady broadband noise (a fan): loud but aperiodic. Must not start
        // speech in adaptive mode, no matter how long it runs.
        let mut vad = Vad::new();
        vad.fixed_threshold = None;
        let started = (0..100).any(|i| ran_speech_start(&mut vad, &noise_frame(320, 5000, i), 1));
        assert!(!started, "steady fan noise must not be detected as speech");
    }

    /// Feed an alternating burst/quiet "chair scrape" pattern and report whether
    /// any frame started speech. Flat (no nested branching) for clarity.
    fn ran_chair_scrape_pattern(vad: &mut Vad) -> bool {
        let quiet = silent_frame(320);
        let frames: Vec<Vec<u8>> = (0..30)
            .flat_map(|burst| {
                let bursts = (0..4).map(move |f| noise_frame(320, 7000, (burst * 10 + f) as u64));
                let quiets = (0..3).map(|_| quiet.clone());
                bursts.chain(quiets)
            })
            .collect();
        frames
            .iter()
            .any(|f| vad.push(f).contains(&VadEvent::SpeechStart))
    }

    #[test]
    fn chair_movement_transient_does_not_trigger_speech_normal() {
        // Moving around in a chair / wheelchair: bursts of loud aperiodic noise
        // separated by quiet. None is pitched, so the periodicity gate (plus
        // the onset debounce) must reject it all.
        let mut vad = Vad::new();
        vad.fixed_threshold = None;
        assert!(
            !ran_chair_scrape_pattern(&mut vad),
            "chair/wheelchair movement (aperiodic transients) must not start speech"
        );
    }

    #[test]
    fn voiced_speech_still_detected_amid_noise_floor_normal() {
        // After a fan has raised the noise floor, genuine pitched speech
        // (periodic, louder) still triggers.
        let mut vad = Vad::new();
        vad.fixed_threshold = None;
        for i in 0..50 {
            vad.push(&noise_frame(320, 3000, i)); // learn the fan floor
        }
        let started = ran_speech_start(&mut vad, &voiced_frame(320, 80, 9000), 8);
        assert!(started, "real voiced speech must beat the noise floor");
    }

    #[test]
    fn spectral_flatness_low_for_tone_normal() {
        // A pure pitched tone has energy in a few bins → low flatness.
        let tone = voiced_frame(320, 80, 8000);
        let flatness = spectral_flatness(&tone);
        assert!(
            flatness < 0.5,
            "tonal signal should have low flatness, got {flatness}"
        );
    }

    #[test]
    fn spectral_flatness_high_for_noise_robust() {
        // Broadband white noise spreads energy across all bins → high flatness.
        let noise = noise_frame(320, 8000, 3);
        let flatness = spectral_flatness(&noise);
        assert!(
            flatness > 0.3,
            "broadband noise should have higher flatness than a tone, got {flatness}"
        );
    }

    #[test]
    fn spectral_flatness_separates_tone_from_noise_normal() {
        // The discriminator must rank noise strictly flatter than a tone.
        let tone = spectral_flatness(&voiced_frame(320, 80, 8000));
        let noise = spectral_flatness(&noise_frame(320, 8000, 9));
        assert!(
            noise > tone,
            "noise ({noise}) should be flatter than tone ({tone})"
        );
    }

    #[test]
    fn flatness_gate_rejects_tonal_noise_when_enabled_normal() {
        // With the opt-in flatness gate, broadband noise that somehow clears
        // periodicity is still rejected for being too flat.
        let mut vad = Vad::new();
        vad.fixed_threshold = None;
        vad.min_periodicity = 0.0; // isolate the flatness gate
        vad.max_flatness = 0.4;
        let started = (0..40).any(|i| ran_speech_start(&mut vad, &noise_frame(320, 6000, i), 1));
        assert!(
            !started,
            "flat broadband noise must be rejected by the flatness gate"
        );
    }

    #[test]
    fn level_meter_in_range_normal() {
        let mut vad = fixed_vad(300);
        vad.push(&loud_frame(320, 5000));
        let lvl = vad.level();
        assert!((0.0..=1.0).contains(&lvl));
        assert!(lvl > 0.0);
    }

    #[test]
    fn level_meter_silent_is_low_robust() {
        let mut vad = fixed_vad(300);
        vad.push(&silent_frame(320));
        assert!(vad.level() < 0.2);
    }

    /// REGRESSION (premature endpoint): the adaptive noise floor must NOT rise
    /// during an active utterance. Bug symptom: while you talk, the between-word
    /// pauses ratcheted the floor up, the offset threshold rose with it, and a
    /// normal pause then read as silence → SpeechEnd fired mid-sentence, the
    /// fragment transcribed, and continued speech started a new utterance.
    #[test]
    fn noise_floor_frozen_during_speech_robust() {
        let mut vad = Vad::new();
        vad.fixed_threshold = None;
        // Calibrate the idle floor on quiet room tone.
        let quiet = loud_frame(320, 200);
        for _ in 0..20 {
            vad.push(&quiet);
        }
        // Enter speech with loud voiced frames.
        let loud = voiced_frame(320, 80, 8000);
        for _ in 0..6 {
            vad.push(&loud);
        }
        assert!(vad.is_speaking(), "should be mid-utterance");
        let floor_at_speech_start = vad.noise_floor();
        // A natural mid-sentence pause: several non-voiced frames (but below the
        // silence hangover, so the utterance is still active).
        let pause = loud_frame(320, 250);
        for _ in 0..3 {
            vad.push(&pause);
        }
        assert!(
            vad.is_speaking(),
            "pause shorter than hangover must not end speech"
        );
        assert_eq!(
            vad.noise_floor(),
            floor_at_speech_start,
            "noise floor must be frozen while in_speech (premature-endpoint bug)"
        );
    }

    /// REGRESSION: a long utterance with periodic short pauses must stay ONE
    /// segment — no SpeechEnd until a real silence ≥ the hangover. Directly
    /// models the reported "gets cut off then catches up" behavior.
    #[test]
    fn long_utterance_with_pauses_stays_one_segment_robust() {
        let mut vad = Vad::new();
        vad.fixed_threshold = None;
        let loud = voiced_frame(320, 80, 9000);
        let short_pause = loud_frame(320, 250); // below hangover length
        // Warm up the floor, then start speaking.
        for _ in 0..10 {
            vad.push(&loud_frame(320, 200));
        }
        let mut ends = 0;
        for _ in 0..8 {
            // ~10 voiced frames, then a 2-frame pause (well under the 1s/50-frame
            // hangover) — a natural speaking cadence.
            for _ in 0..10 {
                ends += vad
                    .push(&loud)
                    .iter()
                    .filter(|e| **e == VadEvent::SpeechEnd)
                    .count();
            }
            for _ in 0..2 {
                ends += vad
                    .push(&short_pause)
                    .iter()
                    .filter(|e| **e == VadEvent::SpeechEnd)
                    .count();
            }
        }
        assert_eq!(
            ends, 0,
            "short within-utterance pauses must not fire SpeechEnd"
        );
    }

    /// A genuine end-of-utterance silence (≥ hangover) still fires exactly one
    /// SpeechEnd after the pause-tolerant change — we didn't break endpointing.
    #[test]
    fn real_silence_still_ends_utterance_normal() {
        let mut vad = Vad::new();
        vad.fixed_threshold = None;
        let loud = voiced_frame(320, 80, 9000);
        for _ in 0..10 {
            vad.push(&loud_frame(320, 200));
        }
        for _ in 0..10 {
            vad.push(&loud);
        }
        assert!(vad.is_speaking());
        // A full second of real silence (> 50-frame hangover).
        let silent = silent_frame(320);
        let ends: usize = (0..60)
            .flat_map(|_| vad.push(&silent))
            .filter(|e| *e == VadEvent::SpeechEnd)
            .count();
        assert_eq!(
            ends, 1,
            "a real end-of-utterance silence must fire one SpeechEnd"
        );
        assert!(!vad.is_speaking());
    }

    /// REGRESSION (anti-truncation): a single stray quiet frame immediately
    /// after onset must NOT fire SpeechEnd before the minimum utterance length,
    /// so the very start of a sentence can't be truncated.
    #[test]
    fn min_utterance_guard_blocks_immediate_end_robust() {
        let mut vad = fixed_vad(300);
        vad.min_speech_frames = 10; // ~200ms guard
        vad.silence_frames = 2; // make end easy to trigger if unguarded
        let loud = loud_frame(320, 5000);
        // Just enough loud frames to start speech (3-frame onset).
        for _ in 0..3 {
            vad.push(&loud);
        }
        assert!(vad.is_speaking());
        // Immediate silence — without the guard this would end after 2 frames,
        // truncating a 3-frame utterance.
        let silent = silent_frame(320);
        let ends: usize = (0..5)
            .flat_map(|_| vad.push(&silent))
            .filter(|e| *e == VadEvent::SpeechEnd)
            .count();
        assert_eq!(
            ends, 0,
            "min-utterance guard must block truncating SpeechEnd"
        );
    }

    #[test]
    fn force_end_while_speaking_normal() {
        let mut vad = fixed_vad(300);
        for _ in 0..5 {
            vad.push(&loud_frame(320, 5000));
        }
        assert!(vad.is_speaking());
        assert!(vad.force_end());
        assert!(!vad.is_speaking());
    }

    #[test]
    fn force_end_while_silent_returns_false_robust() {
        let mut vad = fixed_vad(300);
        assert!(!vad.force_end());
    }

    // ── Intensity-invariant voicing (HNR) ──────────────────────────────────

    /// HNR must be HIGH for a pitched/voiced frame and LOW for white noise —
    /// this is the "real voice vs. white noise" discrimination, independent of
    /// loudness.
    #[test]
    fn hnr_separates_voice_from_white_noise_normal() {
        let voice = voiced_frame(320, 100, 6000); // pitched (160 Hz)
        let noise = noise_frame(320, 6000, 42); // broadband, same amplitude
        let hnr_voice = harmonics_to_noise_db(&voice);
        let hnr_noise = harmonics_to_noise_db(&noise);
        assert!(
            hnr_voice > hnr_noise + 6.0,
            "voiced HNR ({hnr_voice:.1} dB) must clearly exceed noise HNR ({hnr_noise:.1} dB)"
        );
    }

    /// REGRESSION (the user's "my voice changes tone / volume" concern): HNR is
    /// derived from the *normalized* autocorrelation, so it must be ~invariant
    /// to amplitude. The same voiced shape at a quiet and a loud level must
    /// yield nearly identical HNR — i.e. detection shouldn't depend on how loud
    /// you happen to be.
    #[test]
    fn hnr_is_amplitude_invariant_robust() {
        let quiet = voiced_frame(320, 100, 800); // soft voice
        let loud = voiced_frame(320, 100, 16000); // loud voice, same pitch
        let hnr_quiet = harmonics_to_noise_db(&quiet);
        let hnr_loud = harmonics_to_noise_db(&loud);
        assert!(
            (hnr_quiet - hnr_loud).abs() < 1.0,
            "HNR must be amplitude-invariant: quiet={hnr_quiet:.2} dB vs loud={hnr_loud:.2} dB"
        );
        // And both must read as clearly voiced (well above 0 dB).
        assert!(hnr_quiet > 3.0, "soft voice should still read as voiced ({hnr_quiet:.1} dB)");
    }

    /// Periodicity (the gate the VAD actually uses) is likewise amplitude
    /// invariant — proves a quiet voice isn't penalized by the speech-vs-noise
    /// gate, only by the energy floor (which is itself adaptive).
    #[test]
    fn periodicity_is_amplitude_invariant_robust() {
        let quiet = frame_periodicity(&voiced_frame(320, 100, 800));
        let loud = frame_periodicity(&voiced_frame(320, 100, 16000));
        assert!(
            (quiet - loud).abs() < 0.05,
            "normalized periodicity must be amplitude-invariant: {quiet:.3} vs {loud:.3}"
        );
    }

    // ── Energy VAD long-idle floor recovery (no drift) ─────────────────────

    /// REGRESSION (long-idle, energy engine): unlike the neural model, the
    /// energy VAD's adaptive noise floor is a symmetric idle-only EMA, so after
    /// a loud noisy idle stretch it must DECAY back down when the room goes
    /// quiet again — it must not ratchet up and then miss a normal-volume voice.
    #[test]
    fn noise_floor_recovers_after_noisy_idle_robust() {
        let mut vad = Vad::new(); // adaptive mode
        let quiet = silent_frame(320);
        // Settle on a quiet room.
        for _ in 0..30 {
            vad.push(&quiet);
        }
        let floor_quiet = vad.noise_floor();
        // A loud, aperiodic idle stretch (e.g. AC kicks on) — must not be
        // mistaken for speech (it's gated out) but it raises the floor.
        for i in 0..120 {
            vad.push(&noise_frame(320, 4000, i as u64));
        }
        let floor_noisy = vad.noise_floor();
        assert!(
            floor_noisy > floor_quiet,
            "floor should rise during noisy idle ({floor_quiet} -> {floor_noisy})"
        );
        // Room goes quiet again — the floor MUST recover (decay back down).
        for _ in 0..300 {
            vad.push(&quiet);
        }
        let floor_recovered = vad.noise_floor();
        assert!(
            floor_recovered < floor_noisy / 2,
            "floor must decay back down when quiet returns ({floor_noisy} -> {floor_recovered}), \
             otherwise a ratcheted floor would miss a normal-volume voice"
        );
    }
}
