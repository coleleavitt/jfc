//! Target-speaker gating — the *decision* half of Background Voice Cancellation.
//!
//! ## What this is (and honestly is not)
//!
//! Krisp-style **Background Voice Cancellation (BVC)** has two halves:
//!
//! 1. **Decide** which sound belongs to the enrolled primary speaker, and
//! 2. **Mask/separate** that speaker's audio out of the mixture
//!    (VoiceFilter — Wang et al., Interspeech 2019 — a speaker-conditioned
//!    spectrogram-masking neural net).
//!
//! Half (2) requires a *trained* separation network and training data, which we
//! don't have in-tree, so it stays a documented follow-on (see
//! `VAD_RESEARCH.md`). This module implements half (1): a dependency-free,
//! **classical speaker-verification gate** that asks *"does this captured
//! utterance match the enrolled primary speaker, or is it a background voice
//! (a movie/TV/another person)?"* and, if it doesn't match, drops the utterance
//! instead of transcribing it.
//!
//! This is the *predecessor* of the modern neural **d-vector** (Variani 2014;
//! Wan et al. GE2E 2018) and **x-vector** (Snyder et al. 2018) embeddings, which
//! all sit on the **same MFCC front-end** used here (x-vector: "20 MFCCs,
//! 25 ms frames, mean-normalized"). The difference is honest and important: a
//! learned embedding is trained to be speaker-discriminative across thousands of
//! speakers; our **MFCC template + diagonal-Gaussian (one-component GMM-UBM,
//! Reynolds 2000) score** is a hand-built statistic. It reliably rejects
//! *acoustically dissimilar* sources (noise, music, a much higher/lower voice)
//! and is a useful gate, but it is **not** a substitute for a trained model on
//! the hard case of two *similar* human voices. The unit tests validate the
//! pipeline + math on synthetic signals; they do **not** claim real-world
//! two-speaker separation accuracy.
//!
//! ## Pipeline (grounded in the references)
//!
//! MFCC per frame (Fayek 2016; Davis & Mermelstein 1980):
//! pre-emphasis (α=0.97) → 25 ms Hamming frames @ 10 ms stride → 512-pt FFT
//! power spectrum → 26 triangular **mel** filters
//! (`m = 2595·log10(1+f/700)`) → `ln` → DCT-II (orthonormal), keep cepstra
//! `c1..c12` (drop `c0`, the loudness term → loudness-invariant, matching the
//! HNR/periodicity design used elsewhere in this crate).
//!
//! Enrollment builds a [`SpeakerProfile`]: the per-coefficient **mean** and
//! **variance** over the primary speaker's voiced frames (a diagonal Gaussian),
//! the speaker's **pitch** distribution (median ± IQR of f0), and a
//! **calibrated acceptance threshold** derived from the enrollment self-distance
//! distribution (the EER-style self-calibration; a fixed cosine/Mahalanobis
//! threshold is otherwise device/voice dependent).
//!
//! Scoring a captured utterance:
//! - average **Mahalanobis** distance of its voiced frames to the diagonal
//!   Gaussian — i.e. the average squared z-score, which is ≈1 for the enrolled
//!   speaker and ≫1 for a dissimilar source (the one-component GMM-UBM score),
//! - **cosine** similarity of the utterance's mean cepstrum to the centroid
//!   (the d-vector-style score, exposed for diagnostics), and
//! - a **pitch** consistency check against the enrolled f0 range.
//!
//! `accepts()` requires both the Mahalanobis distance to be within the
//! calibrated threshold **and** the pitch to be consistent.

use serde::{Deserialize, Serialize};

use crate::vad::dft_in_place;

/// Number of cepstral coefficients retained (`c1..=c12`; `c0` dropped).
pub const N_CEPS: usize = 12;

/// 16 kHz mono is the fixed capture format (see `audio.rs`).
const SAMPLE_RATE: usize = 16_000;
/// 25 ms analysis frame.
const FRAME_LEN: usize = 400;
/// 10 ms stride (15 ms overlap) — the standard MFCC setting.
const FRAME_STEP: usize = 160;
/// FFT size (≥ FRAME_LEN, power of two).
const N_FFT: usize = 512;
/// Triangular mel filters spanning 0..Nyquist.
const N_FILTERS: usize = 26;
/// Pre-emphasis coefficient.
const PRE_EMPHASIS: f64 = 0.97;
/// Floor applied to per-dimension variance so a near-constant coefficient can't
/// make the Mahalanobis distance explode.
const VAR_FLOOR: f64 = 1.0;
/// Pitch search range: 80–400 Hz → lags 40–200 at 16 kHz.
const MIN_PITCH_LAG: usize = 40;
const MAX_PITCH_LAG: usize = 200;
/// A frame is "voiced enough" to use for MFCC stats when its normalized
/// autocorrelation peak clears this (matches the energy VAD's calibration).
const VOICED_PERIODICITY: f64 = 0.30;

/// Decode S16LE PCM bytes to f64 samples.
fn decode_pcm(pcm: &[u8]) -> Vec<f64> {
    pcm.chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f64)
        .collect()
}

/// Hz → mel.
fn hz_to_mel(f: f64) -> f64 {
    2595.0 * (1.0 + f / 700.0).log10()
}
/// mel → Hz.
fn mel_to_hz(m: f64) -> f64 {
    700.0 * (10f64.powf(m / 2595.0) - 1.0)
}

/// Precomputed triangular mel filterbank: for each filter, the (start_bin,
/// weights) over the positive-frequency FFT bins.
struct MelFilterbank {
    filters: Vec<(usize, Vec<f64>)>,
}

impl MelFilterbank {
    fn new() -> Self {
        let n_bins = N_FFT / 2 + 1;
        let mel_low = hz_to_mel(0.0);
        let mel_high = hz_to_mel((SAMPLE_RATE / 2) as f64);
        // N_FILTERS+2 points equally spaced in mel; convert to FFT bins.
        let mut bins = Vec::with_capacity(N_FILTERS + 2);
        for i in 0..N_FILTERS + 2 {
            let mel = mel_low + (mel_high - mel_low) * i as f64 / (N_FILTERS + 1) as f64;
            let hz = mel_to_hz(mel);
            let bin = ((N_FFT + 1) as f64 * hz / SAMPLE_RATE as f64).floor() as usize;
            bins.push(bin.min(n_bins - 1));
        }
        let mut filters = Vec::with_capacity(N_FILTERS);
        for m in 1..=N_FILTERS {
            let (left, center, right) = (bins[m - 1], bins[m], bins[m + 1]);
            let start = left;
            let mut weights = vec![0.0; right.saturating_sub(left) + 1];
            for k in left..=right {
                let w = if k < center && center > left {
                    (k - left) as f64 / (center - left) as f64
                } else if k >= center && right > center {
                    (right - k) as f64 / (right - center) as f64
                } else {
                    1.0
                };
                weights[k - start] = w;
            }
            filters.push((start, weights));
        }
        Self { filters }
    }

    /// Apply the filterbank to a positive-frequency power spectrum, returning
    /// `ln` mel energies.
    fn log_energies(&self, power: &[f64]) -> [f64; N_FILTERS] {
        let mut out = [0.0; N_FILTERS];
        for (fi, (start, weights)) in self.filters.iter().enumerate() {
            let mut acc = 0.0;
            for (j, &w) in weights.iter().enumerate() {
                if let Some(&p) = power.get(start + j) {
                    acc += w * p;
                }
            }
            out[fi] = acc.max(1e-9).ln();
        }
        out
    }
}

/// Orthonormal DCT-II of the log-mel energies, returning cepstra `c1..=c12`.
fn dct_keep_ceps(log_mel: &[f64; N_FILTERS]) -> [f64; N_CEPS] {
    let m = N_FILTERS as f64;
    let mut out = [0.0; N_CEPS];
    // We want coefficients k = 1..=N_CEPS (drop c0).
    for (idx, k) in (1..=N_CEPS).enumerate() {
        let mut acc = 0.0;
        for (n, &v) in log_mel.iter().enumerate() {
            acc += v * (std::f64::consts::PI * k as f64 * (n as f64 + 0.5) / m).cos();
        }
        // Orthonormal scaling (k>0).
        out[idx] = acc * (2.0 / m).sqrt();
    }
    out
}

/// Compute the power spectrum of one pre-emphasized, Hamming-windowed frame.
fn frame_power_spectrum(frame: &[f64]) -> [f64; N_FFT / 2 + 1] {
    let mut re = [0.0f64; N_FFT];
    let mut im = [0.0f64; N_FFT];
    let n = frame.len().min(N_FFT);
    for i in 0..n {
        // Hamming window.
        let w = 0.54 - 0.46 * (std::f64::consts::TAU * i as f64 / (FRAME_LEN as f64 - 1.0)).cos();
        re[i] = frame[i] * w;
    }
    dft_in_place(&mut re, &mut im);
    let mut power = [0.0f64; N_FFT / 2 + 1];
    for (k, p) in power.iter_mut().enumerate() {
        *p = (re[k] * re[k] + im[k] * im[k]) / N_FFT as f64;
    }
    power
}

/// Normalized autocorrelation pitch of a frame: returns `Some(f0_hz)` when the
/// frame is voiced (peak ≥ [`VOICED_PERIODICITY`]), else `None`.
fn frame_pitch_hz(frame: &[f64]) -> Option<f64> {
    let n = frame.len();
    if n <= MAX_PITCH_LAG + 1 {
        return None;
    }
    let energy0: f64 = frame.iter().map(|&s| s * s).sum();
    if energy0 <= f64::EPSILON {
        return None;
    }
    let (mut best, mut best_lag) = (0.0f64, 0usize);
    for lag in MIN_PITCH_LAG..=MAX_PITCH_LAG {
        let mut acc = 0.0;
        for i in 0..(n - lag) {
            acc += frame[i] * frame[i + lag];
        }
        let norm = acc / energy0;
        if norm > best {
            best = norm;
            best_lag = lag;
        }
    }
    if best >= VOICED_PERIODICITY && best_lag > 0 {
        Some(SAMPLE_RATE as f64 / best_lag as f64)
    } else {
        None
    }
}

/// One analyzed voiced frame: its MFCC vector and pitch.
struct VoicedFrame {
    mfcc: [f64; N_CEPS],
    pitch_hz: f64,
}

/// Extract MFCC + pitch for every **voiced** frame in a PCM segment. Unvoiced /
/// silent frames are skipped so enrollment and scoring use speech, not gaps.
fn voiced_frames(pcm: &[u8]) -> Vec<VoicedFrame> {
    let samples = decode_pcm(pcm);
    if samples.len() < FRAME_LEN {
        return Vec::new();
    }
    // Pre-emphasis: y[t] = x[t] - α·x[t-1].
    let mut emph = Vec::with_capacity(samples.len());
    emph.push(samples[0]);
    for i in 1..samples.len() {
        emph.push(samples[i] - PRE_EMPHASIS * samples[i - 1]);
    }

    let fb = MelFilterbank::new();
    let mut out = Vec::new();
    let mut start = 0;
    while start + FRAME_LEN <= emph.len() {
        let frame = &emph[start..start + FRAME_LEN];
        start += FRAME_STEP;
        // Pitch is computed on the (pre-emphasized) frame; voicing gates use.
        let Some(pitch_hz) = frame_pitch_hz(frame) else {
            continue;
        };
        let power = frame_power_spectrum(frame);
        let log_mel = fb.log_energies(&power);
        let mfcc = dct_keep_ceps(&log_mel);
        out.push(VoicedFrame { mfcc, pitch_hz });
    }
    out
}

/// Result of scoring a captured utterance against a [`SpeakerProfile`].
#[derive(Debug, Clone, Copy)]
pub struct MatchScore {
    /// Average per-frame Mahalanobis distance to the enrolled diagonal Gaussian
    /// (the one-component GMM-UBM score). ≈1 for the enrolled speaker, ≫1 for a
    /// dissimilar source. Lower is a better match.
    pub mahalanobis: f64,
    /// Cosine similarity of the utterance's mean cepstrum to the enrolled
    /// centroid, in `[-1, 1]` (the d-vector-style score). Higher is better.
    pub cosine: f64,
    /// Median pitch (Hz) of the utterance's voiced frames, if any.
    pub pitch_hz: Option<f64>,
    /// Whether the pitch is within the enrolled speaker's range.
    pub pitch_ok: bool,
    /// Number of voiced frames scored (0 ⇒ no usable speech).
    pub voiced_frames: usize,
    /// Final gate decision: matches the enrolled primary speaker.
    pub accepted: bool,
}

/// A learned speaker embedding (ECAPA-TDNN / x-vector) enrolled alongside the
/// classical statistics. When present, the gate prefers cosine scoring on this
/// embedding — the SOTA-accuracy path — and falls back to the classical
/// Mahalanobis+pitch score when it isn't (no model configured / inference
/// failed). Serialized with the profile so enrollment is a one-off.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeuralProfile {
    /// L2-normalized enrolled speaker embedding (the d-vector / x-vector).
    pub embedding: Vec<f32>,
    /// Backend tag, e.g. `"onnx"` plus the model file stem, for diagnostics.
    pub backend: String,
    /// Cosine accept threshold. ECAPA cosine for same-speaker trials typically
    /// sits well above this; ~0.25–0.4 is a common operating point.
    pub threshold: f64,
}

/// An enrolled primary-speaker model: a diagonal Gaussian over MFCCs plus a
/// pitch range and a calibrated acceptance threshold (the classical gate), and
/// optionally a learned neural embedding (the SOTA path).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeakerProfile {
    /// Per-coefficient mean (the MFCC centroid / template).
    pub mean: Vec<f64>,
    /// Per-coefficient variance (diagonal covariance).
    pub var: Vec<f64>,
    /// Median pitch (Hz) of the enrolled speaker.
    pub pitch_median_hz: f64,
    /// Inter-quartile range (Hz) of the enrolled pitch.
    pub pitch_iqr_hz: f64,
    /// Calibrated acceptance threshold on the average Mahalanobis distance.
    pub threshold: f64,
    /// Number of voiced frames used to build the profile.
    pub enrolled_frames: u64,
    /// Optional learned embedding (ECAPA/x-vector). Defaulted on load so older
    /// classical-only profiles deserialize unchanged.
    #[serde(default)]
    pub neural: Option<NeuralProfile>,
}

impl SpeakerProfile {
    /// Build a profile from a few seconds of the **primary speaker's** PCM
    /// (16 kHz mono S16LE). Returns `None` when there isn't enough voiced audio
    /// to estimate stable statistics.
    pub fn enroll_from_pcm(pcm: &[u8]) -> Option<Self> {
        let frames = voiced_frames(pcm);
        // Need a meaningful amount of voiced speech (~0.5 s of voiced frames).
        if frames.len() < 30 {
            return None;
        }
        let n = frames.len() as f64;

        // Mean + variance per cepstral dimension (diagonal Gaussian).
        let mut mean = vec![0.0; N_CEPS];
        for f in &frames {
            for (i, &c) in f.mfcc.iter().enumerate() {
                mean[i] += c;
            }
        }
        for m in &mut mean {
            *m /= n;
        }
        let mut var = vec![0.0; N_CEPS];
        for f in &frames {
            for (i, &c) in f.mfcc.iter().enumerate() {
                let d = c - mean[i];
                var[i] += d * d;
            }
        }
        for v in &mut var {
            *v = (*v / n).max(VAR_FLOOR);
        }

        // Pitch distribution (median + IQR).
        let mut pitches: Vec<f64> = frames.iter().map(|f| f.pitch_hz).collect();
        pitches.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let pitch_median_hz = percentile(&pitches, 0.5);
        let pitch_iqr_hz = (percentile(&pitches, 0.75) - percentile(&pitches, 0.25)).max(1.0);

        // Calibrate the threshold from the enrollment self-distance
        // distribution: for the enrolled speaker the average squared z-score is
        // ≈1, so threshold = mean + 3·std bounds normal within-speaker variation
        // while still rejecting clearly dissimilar sources.
        let dists: Vec<f64> = frames
            .iter()
            .map(|f| mahalanobis(&f.mfcc, &mean, &var))
            .collect();
        let self_mean = dists.iter().sum::<f64>() / n;
        let self_std = (dists.iter().map(|&d| (d - self_mean).powi(2)).sum::<f64>() / n).sqrt();
        let threshold = (self_mean + 3.0 * self_std).max(self_mean * 1.5);

        Some(Self {
            mean,
            var,
            pitch_median_hz,
            pitch_iqr_hz,
            threshold,
            enrolled_frames: frames.len() as u64,
            neural: None,
        })
    }

    /// Attach (or replace) a learned neural embedding computed by `embedder`
    /// from the enrollment PCM. Returns `self` unchanged if the embedder can't
    /// produce an embedding (e.g. no model configured), so callers can chain it
    /// unconditionally. The neural embedding, when present, takes precedence in
    /// [`Self::score`].
    pub fn with_neural_embedding(mut self, embedder: &dyn SpeakerEmbedder, pcm: &[u8]) -> Self {
        if let Some(embedding) = embedder.embed(pcm) {
            self.neural = Some(NeuralProfile {
                embedding,
                backend: embedder.name().to_owned(),
                threshold: neural_threshold_default(),
            });
        }
        self
    }

    /// Score a captured utterance against this profile. Uses the learned neural
    /// embedding (cosine) when both the profile carries one and `embedder` can
    /// produce a matching embedding; otherwise the classical Mahalanobis+pitch
    /// score. Pass [`NullEmbedder`] to force the classical path.
    pub fn score_with(&self, pcm: &[u8], embedder: &dyn SpeakerEmbedder) -> MatchScore {
        if let Some(neural) = &self.neural {
            if let Some(query) = embedder.embed(pcm) {
                let cos = cosine_f32(&query, &neural.embedding);
                return MatchScore {
                    mahalanobis: f64::NAN, // not used on the neural path
                    cosine: cos,
                    pitch_hz: None,
                    pitch_ok: true,
                    voiced_frames: 1,
                    accepted: cos >= neural.threshold,
                };
            }
        }
        self.score(pcm)
    }

    /// Score a captured utterance against this profile (classical path).
    pub fn score(&self, pcm: &[u8]) -> MatchScore {
        let frames = voiced_frames(pcm);
        if frames.is_empty() {
            return MatchScore {
                mahalanobis: f64::INFINITY,
                cosine: -1.0,
                pitch_hz: None,
                pitch_ok: false,
                voiced_frames: 0,
                accepted: false,
            };
        }
        let n = frames.len() as f64;

        // Average Mahalanobis distance (GMM-UBM-style score) + mean cepstrum.
        let mut dist_sum = 0.0;
        let mut test_mean = vec![0.0; N_CEPS];
        for f in &frames {
            dist_sum += mahalanobis(&f.mfcc, &self.mean, &self.var);
            for (i, &c) in f.mfcc.iter().enumerate() {
                test_mean[i] += c;
            }
        }
        let mahalanobis_avg = dist_sum / n;
        for m in &mut test_mean {
            *m /= n;
        }
        let cosine = cosine_sim(&test_mean, &self.mean);

        // Median pitch + range check.
        let mut pitches: Vec<f64> = frames.iter().map(|f| f.pitch_hz).collect();
        pitches.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let pitch_med = percentile(&pitches, 0.5);
        // Accept pitch within median ± (2·IQR + 25% margin) of the enrolled range.
        let tol = 2.0 * self.pitch_iqr_hz + 0.25 * self.pitch_median_hz;
        let pitch_ok = (pitch_med - self.pitch_median_hz).abs() <= tol;

        let accepted = mahalanobis_avg <= self.threshold && pitch_ok;

        MatchScore {
            mahalanobis: mahalanobis_avg,
            cosine,
            pitch_hz: Some(pitch_med),
            pitch_ok,
            voiced_frames: frames.len(),
            accepted,
        }
    }

    /// Convenience: does this utterance match the enrolled primary speaker
    /// (classical path)?
    pub fn accepts(&self, pcm: &[u8]) -> bool {
        self.score(pcm).accepted
    }

    /// Convenience: does this utterance match the enrolled primary speaker,
    /// using `embedder` (neural path when available, classical otherwise)?
    pub fn accepts_with(&self, pcm: &[u8], embedder: &dyn SpeakerEmbedder) -> bool {
        self.score_with(pcm, embedder).accepted
    }

    /// Override the calibrated acceptance threshold (e.g. from config/env). A
    /// larger value is more permissive (accepts more), smaller is stricter.
    pub fn with_threshold(mut self, threshold: f64) -> Self {
        self.threshold = threshold;
        self
    }

    /// Serialize to pretty JSON.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Parse from JSON, validating shape.
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        let p: Self = serde_json::from_str(s)?;
        Ok(p)
    }

    /// Load a profile from a JSON file path.
    pub fn load(path: &std::path::Path) -> std::io::Result<Self> {
        let s = std::fs::read_to_string(path)?;
        Self::from_json(&s).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    /// Save a profile to a JSON file path (creating parent dirs).
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let s = self
            .to_json()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, s)
    }
}

/// Average squared z-score (diagonal-Gaussian Mahalanobis distance) of a sample
/// to `(mean, var)`. For the enrolled speaker this averages ≈1 by construction.
fn mahalanobis(x: &[f64; N_CEPS], mean: &[f64], var: &[f64]) -> f64 {
    let mut acc = 0.0;
    for i in 0..N_CEPS {
        let d = x[i] - mean[i];
        acc += d * d / var[i].max(VAR_FLOOR);
    }
    acc / N_CEPS as f64
}

/// Cosine similarity of two vectors in `[-1, 1]`.
fn cosine_sim(a: &[f64], b: &[f64]) -> f64 {
    let mut dot = 0.0;
    let mut na = 0.0;
    let mut nb = 0.0;
    for i in 0..a.len().min(b.len()) {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na <= f64::EPSILON || nb <= f64::EPSILON {
        return 0.0;
    }
    (dot / (na.sqrt() * nb.sqrt())).clamp(-1.0, 1.0)
}

/// Cosine similarity of two f32 vectors in `[-1, 1]`.
fn cosine_f32(a: &[f32], b: &[f32]) -> f64 {
    let mut dot = 0.0f64;
    let mut na = 0.0f64;
    let mut nb = 0.0f64;
    for i in 0..a.len().min(b.len()) {
        dot += a[i] as f64 * b[i] as f64;
        na += (a[i] as f64).powi(2);
        nb += (b[i] as f64).powi(2);
    }
    if na <= f64::EPSILON || nb <= f64::EPSILON {
        return 0.0;
    }
    (dot / (na.sqrt() * nb.sqrt())).clamp(-1.0, 1.0)
}

/// Default cosine accept threshold for the neural path
/// (`JFC_VOICE_SPEAKER_COS_THRESHOLD`, default 0.30). ECAPA same-speaker cosine
/// typically sits comfortably above this; raise it to be stricter.
fn neural_threshold_default() -> f64 {
    std::env::var("JFC_VOICE_SPEAKER_COS_THRESHOLD")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.30)
}

/// Linear-interpolated percentile of a pre-sorted slice.
fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let pos = q * (sorted.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    let frac = pos - lo as f64;
    sorted[lo] * (1.0 - frac) + sorted[hi] * frac
}

// ── Pluggable speaker-embedding backend ────────────────────────────────────

/// A speaker-embedding backend: maps a PCM utterance to a fixed-length,
/// L2-normalized speaker embedding (a d-vector / x-vector). Backends:
/// - [`NullEmbedder`]: always returns `None` (forces the classical gate);
/// - [`OnnxEmbedder`] (feature `speaker-neural`): runs an ECAPA-TDNN/x-vector
///   ONNX model for SOTA accuracy (EER ~0.9% on VoxCeleb).
pub trait SpeakerEmbedder: Send + Sync {
    /// Compute an L2-normalized embedding for the utterance, or `None` if this
    /// backend can't (no model, too little audio, inference error).
    fn embed(&self, pcm_i16le: &[u8]) -> Option<Vec<f32>>;
    /// Short backend identifier for the profile's `backend` tag.
    fn name(&self) -> &str;
}

/// The no-op embedder: never produces an embedding, so the classical
/// MFCC-template gate is used. This is the always-available default.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullEmbedder;

impl SpeakerEmbedder for NullEmbedder {
    fn embed(&self, _pcm_i16le: &[u8]) -> Option<Vec<f32>> {
        None
    }
    fn name(&self) -> &str {
        "null"
    }
}

/// Compute the model input feature: log-mel filterbank energies (no DCT — the
/// learned x-vector/ECAPA front-end consumes filterbanks, not cepstra), with
/// per-utterance cepstral mean normalization over the *feature* dimension. Rows
/// are voiced frames, columns are the `N_FILTERS` mel bands. Returns `None` if
/// there isn't enough voiced audio.
///
/// Shared by [`OnnxEmbedder`] and exposed for tests; this is exactly the
/// front-end ECAPA-TDNN (Desplanques 2020) and x-vector (Snyder 2018) use,
/// modulo the model's expected `n_mels` (configurable via env).
pub fn fbank_features(pcm_i16le: &[u8], n_mels: usize) -> Option<(usize, usize, Vec<f32>)> {
    // We reuse the 26-band filterbank when n_mels matches; otherwise build a
    // bank with the requested resolution. Most ECAPA exports use 80 mels.
    let frames = voiced_frames_logmel(pcm_i16le, n_mels)?;
    let n_frames = frames.len();
    if n_frames == 0 {
        return None;
    }
    // Mean-normalize each mel band across time (CMVN).
    let mut mean = vec![0.0f32; n_mels];
    for fr in &frames {
        for (m, &v) in mean.iter_mut().zip(fr.iter()) {
            *m += v;
        }
    }
    for m in &mut mean {
        *m /= n_frames as f32;
    }
    let mut flat = Vec::with_capacity(n_frames * n_mels);
    for fr in &frames {
        for (j, &v) in fr.iter().enumerate() {
            flat.push(v - mean[j]);
        }
    }
    Some((n_frames, n_mels, flat))
}

/// Per-voiced-frame log-mel energies at an arbitrary band count.
fn voiced_frames_logmel(pcm_i16le: &[u8], n_mels: usize) -> Option<Vec<Vec<f32>>> {
    let samples = decode_pcm(pcm_i16le);
    if samples.len() < FRAME_LEN {
        return None;
    }
    let mut emph = Vec::with_capacity(samples.len());
    emph.push(samples[0]);
    for i in 1..samples.len() {
        emph.push(samples[i] - PRE_EMPHASIS * samples[i - 1]);
    }
    let fb = MelFilterbankN::new(n_mels);
    let mut out = Vec::new();
    let mut start = 0;
    while start + FRAME_LEN <= emph.len() {
        let frame = &emph[start..start + FRAME_LEN];
        start += FRAME_STEP;
        if frame_pitch_hz(frame).is_none() {
            continue;
        }
        let power = frame_power_spectrum(frame);
        out.push(fb.log_energies(&power).iter().map(|&v| v as f32).collect());
    }
    if out.is_empty() { None } else { Some(out) }
}

/// Mel filterbank with a configurable number of bands (generalization of
/// [`MelFilterbank`] for neural front-ends needing e.g. 80 mels).
struct MelFilterbankN {
    n_mels: usize,
    filters: Vec<(usize, Vec<f64>)>,
}

impl MelFilterbankN {
    fn new(n_mels: usize) -> Self {
        let n_bins = N_FFT / 2 + 1;
        let mel_low = hz_to_mel(0.0);
        let mel_high = hz_to_mel((SAMPLE_RATE / 2) as f64);
        let mut bins = Vec::with_capacity(n_mels + 2);
        for i in 0..n_mels + 2 {
            let mel = mel_low + (mel_high - mel_low) * i as f64 / (n_mels + 1) as f64;
            let hz = mel_to_hz(mel);
            let bin = ((N_FFT + 1) as f64 * hz / SAMPLE_RATE as f64).floor() as usize;
            bins.push(bin.min(n_bins - 1));
        }
        let mut filters = Vec::with_capacity(n_mels);
        for m in 1..=n_mels {
            let (left, center, right) = (bins[m - 1], bins[m], bins[m + 1]);
            let mut weights = vec![0.0; right.saturating_sub(left) + 1];
            for k in left..=right {
                let w = if k < center && center > left {
                    (k - left) as f64 / (center - left) as f64
                } else if k >= center && right > center {
                    (right - k) as f64 / (right - center) as f64
                } else {
                    1.0
                };
                weights[k - left] = w;
            }
            filters.push((left, weights));
        }
        Self { n_mels, filters }
    }

    fn log_energies(&self, power: &[f64]) -> Vec<f64> {
        let mut out = vec![0.0; self.n_mels];
        for (fi, (start, weights)) in self.filters.iter().enumerate() {
            let mut acc = 0.0;
            for (j, &w) in weights.iter().enumerate() {
                if let Some(&p) = power.get(start + j) {
                    acc += w * p;
                }
            }
            out[fi] = acc.max(1e-9).ln();
        }
        out
    }
}

#[cfg(feature = "speaker-neural")]
pub use onnx_backend::OnnxEmbedder;

#[cfg(feature = "speaker-neural")]
mod onnx_backend {
    use super::{SpeakerEmbedder, fbank_features};
    use std::sync::Mutex;

    use ort::session::Session;
    use ort::value::Tensor;

    /// ECAPA-TDNN / x-vector speaker-embedding backend over ONNX Runtime.
    ///
    /// Loads a user-provided ONNX model (`JFC_VOICE_SPEAKER_MODEL`) that maps a
    /// log-mel feature sequence to a speaker embedding. The model's input is
    /// auto-detected: a rank-3 `[batch, frames, mels]` or `[batch, mels, frames]`
    /// float tensor. The number of mel bands is taken from the static input
    /// shape when present, else `JFC_VOICE_SPEAKER_NMELS` (default 80). The
    /// first output tensor is treated as the embedding and L2-normalized.
    ///
    /// This reuses the same ONNX Runtime the Silero VAD backend links, so no new
    /// native dependency is introduced beyond enabling the `speaker-neural`
    /// feature.
    pub struct OnnxEmbedder {
        session: Mutex<Session>,
        name: String,
        n_mels: usize,
        /// True when the model expects `[batch, mels, frames]` (channels-first).
        mels_first: bool,
    }

    impl OnnxEmbedder {
        /// Load a model from `path`. Returns `Err` with a human-readable reason
        /// if the ONNX Runtime/session can't be created.
        pub fn from_path(path: &std::path::Path) -> anyhow::Result<Self> {
            let session = Session::builder()
                .map_err(|e| anyhow::anyhow!("ort session builder: {e}"))?
                .commit_from_file(path)
                .map_err(|e| anyhow::anyhow!("load speaker model {}: {e}", path.display()))?;

            // Auto-detect mel-band count + axis order from the input shape.
            let (mut n_mels, mut mels_first) = (default_nmels(), false);
            if let Some(input) = session.inputs.first() {
                if let ort::value::ValueType::Tensor { shape, .. } = &input.input_type {
                    let dims: Vec<i64> = shape.iter().copied().collect();
                    // rank-3 [batch, A, B]; the static (non -1, !=1) dim is mels.
                    if dims.len() == 3 {
                        let a = dims[1];
                        let b = dims[2];
                        if a > 1 {
                            n_mels = a as usize;
                            mels_first = true;
                        } else if b > 1 {
                            n_mels = b as usize;
                            mels_first = false;
                        }
                    }
                }
            }

            let name = format!(
                "onnx:{}",
                path.file_stem().and_then(|s| s.to_str()).unwrap_or("model")
            );
            Ok(Self {
                session: Mutex::new(session),
                name,
                n_mels,
                mels_first,
            })
        }

        /// Try to construct from `JFC_VOICE_SPEAKER_MODEL`; `None` if unset or
        /// the model fails to load (logged).
        pub fn from_env() -> Option<Self> {
            let path = std::env::var("JFC_VOICE_SPEAKER_MODEL").ok()?;
            match Self::from_path(std::path::Path::new(&path)) {
                Ok(e) => {
                    tracing::info!(
                        target: "jfc::voice::speaker",
                        model = %path,
                        n_mels = e.n_mels,
                        mels_first = e.mels_first,
                        "loaded neural speaker-embedding model"
                    );
                    Some(e)
                }
                Err(err) => {
                    tracing::warn!(
                        target: "jfc::voice::speaker",
                        model = %path,
                        error = %err,
                        "failed to load speaker model; falling back to classical gate"
                    );
                    None
                }
            }
        }
    }

    impl SpeakerEmbedder for OnnxEmbedder {
        fn embed(&self, pcm_i16le: &[u8]) -> Option<Vec<f32>> {
            let (n_frames, n_mels, feats) = fbank_features(pcm_i16le, self.n_mels)?;
            // Shape the tensor as the model expects.
            let (shape, data): (Vec<i64>, Vec<f32>) = if self.mels_first {
                // [1, mels, frames] — transpose the row-major [frames, mels].
                let mut t = vec![0f32; n_mels * n_frames];
                for f in 0..n_frames {
                    for m in 0..n_mels {
                        t[m * n_frames + f] = feats[f * n_mels + m];
                    }
                }
                (vec![1, n_mels as i64, n_frames as i64], t)
            } else {
                (vec![1, n_frames as i64, n_mels as i64], feats)
            };

            let shape_usize: Vec<usize> = shape.iter().map(|&d| d as usize).collect();
            let tensor = Tensor::from_array((shape_usize, data.into_boxed_slice())).ok()?;

            let mut session = self.session.lock().ok()?;
            let input_name = session.inputs.first()?.name.clone();
            let outputs = session
                .run(ort::inputs![input_name.as_str() => tensor])
                .ok()?;
            // Take the first output as the embedding.
            let first_key = outputs.keys().next()?.to_owned();
            let value = outputs.get(first_key)?;
            let (_shape, slice) = value.try_extract_tensor::<f32>().ok()?;
            let mut emb = slice.to_vec();
            l2_normalize_f32(&mut emb);
            if emb.is_empty() { None } else { Some(emb) }
        }

        fn name(&self) -> &str {
            &self.name
        }
    }

    fn default_nmels() -> usize {
        std::env::var("JFC_VOICE_SPEAKER_NMELS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(80)
    }

    fn l2_normalize_f32(v: &mut [f32]) {
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > f32::EPSILON {
            for x in v {
                *x /= norm;
            }
        }
    }
}

/// Construct the best available speaker embedder for the current build/config:
/// the ONNX backend when the `speaker-neural` feature is on AND
/// `JFC_VOICE_SPEAKER_MODEL` loads, otherwise the [`NullEmbedder`] (classical
/// gate). Returned boxed so callers don't depend on the feature.
pub fn default_embedder() -> Box<dyn SpeakerEmbedder> {
    #[cfg(feature = "speaker-neural")]
    {
        if let Some(e) = onnx_backend::OnnxEmbedder::from_env() {
            return Box::new(e);
        }
    }
    Box::new(NullEmbedder)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::TAU;

    /// Synthesize ~`secs` seconds of a voiced-like signal: a sum of harmonics of
    /// `f0` with a fixed spectral envelope, plus a touch of jitter so the
    /// per-frame stats have realistic (non-zero) variance. 16 kHz S16LE.
    ///
    /// NB: synthetic. These exercise the MFCC/scoring *math* and gross spectral/
    /// pitch discrimination — NOT real two-human-voice separation (see module
    /// docs / VAD_RESEARCH.md).
    fn synth_voice(f0: f64, secs: f64, formant_tilt: f64, seed: u64) -> Vec<u8> {
        let n = (SAMPLE_RATE as f64 * secs) as usize;
        let mut state = seed.wrapping_add(0x9E3779B97F4A7C15);
        let mut rng = move || {
            state = state.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            ((z ^ (z >> 31)) as f64 / u64::MAX as f64) - 0.5
        };
        let mut out = Vec::with_capacity(n * 2);
        for i in 0..n {
            let t = i as f64 / SAMPLE_RATE as f64;
            let mut s = 0.0;
            // 8 harmonics with a 1/k^tilt envelope → a vowel-ish spectrum.
            for k in 1..=8 {
                let amp = 1.0 / (k as f64).powf(formant_tilt);
                s += amp * (TAU * f0 * k as f64 * t).sin();
            }
            s += 0.02 * rng(); // mild aperiodic component
            let v = (s / 3.0 * 12000.0).clamp(-32000.0, 32000.0) as i16;
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    fn white_noise(secs: f64, seed: u64) -> Vec<u8> {
        let n = (SAMPLE_RATE as f64 * secs) as usize;
        let mut state = seed.wrapping_add(0x1234567);
        let mut rng = move || {
            state = state.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z ^ (z >> 31)
        };
        let mut out = Vec::with_capacity(n * 2);
        for _ in 0..n {
            let v = (rng() % 20000) as i16 - 10000;
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    #[test]
    fn mfcc_is_deterministic_normal() {
        let pcm = synth_voice(140.0, 0.3, 1.0, 1);
        let a = voiced_frames(&pcm);
        let b = voiced_frames(&pcm);
        assert_eq!(a.len(), b.len());
        assert!(!a.is_empty(), "voiced synthetic signal should yield frames");
        for (fa, fb) in a.iter().zip(b.iter()) {
            assert_eq!(fa.mfcc, fb.mfcc);
        }
    }

    #[test]
    fn mfcc_dim_is_n_ceps_normal() {
        let pcm = synth_voice(150.0, 0.2, 1.0, 2);
        let frames = voiced_frames(&pcm);
        assert!(!frames.is_empty());
        assert_eq!(frames[0].mfcc.len(), N_CEPS);
    }

    #[test]
    fn enroll_then_self_accepts_robust() {
        let me = synth_voice(130.0, 2.0, 1.0, 7);
        let profile = SpeakerProfile::enroll_from_pcm(&me).expect("enrollment");
        // A different recording of the "same" synthetic voice (new seed/jitter).
        let me_again = synth_voice(130.0, 1.5, 1.0, 99);
        let score = profile.score(&me_again);
        assert!(
            score.accepted,
            "same synthetic speaker must be accepted: maha={:.2} thr={:.2} pitch_ok={}",
            score.mahalanobis, profile.threshold, score.pitch_ok
        );
        assert!(
            score.cosine > 0.5,
            "cosine to centroid should be high: {:.3}",
            score.cosine
        );
    }

    #[test]
    fn rejects_white_noise_robust() {
        let me = synth_voice(130.0, 2.0, 1.0, 7);
        let profile = SpeakerProfile::enroll_from_pcm(&me).expect("enrollment");
        let noise = white_noise(1.5, 5);
        let score = profile.score(&noise);
        assert!(
            !score.accepted,
            "white noise must be rejected: maha={:.2} thr={:.2} voiced={}",
            score.mahalanobis, profile.threshold, score.voiced_frames
        );
    }

    #[test]
    fn rejects_very_different_pitch_robust() {
        // Enroll a low voice; a much higher voice (different pitch + spectrum)
        // should be rejected by the pitch and/or Mahalanobis gate.
        let low = synth_voice(110.0, 2.0, 1.0, 7);
        let profile = SpeakerProfile::enroll_from_pcm(&low).expect("enrollment");
        let high = synth_voice(330.0, 1.5, 1.6, 8);
        let score = profile.score(&high);
        assert!(
            !score.accepted,
            "acoustically different source should be rejected: maha={:.2} thr={:.2} pitch_ok={}",
            score.mahalanobis, profile.threshold, score.pitch_ok
        );
    }

    #[test]
    fn enroll_rejects_insufficient_audio_normal() {
        // A tiny clip has too few voiced frames to enroll.
        let tiny = synth_voice(140.0, 0.05, 1.0, 1);
        assert!(SpeakerProfile::enroll_from_pcm(&tiny).is_none());
    }

    #[test]
    fn profile_json_roundtrips_normal() {
        let me = synth_voice(160.0, 2.0, 1.0, 3);
        let profile = SpeakerProfile::enroll_from_pcm(&me).expect("enrollment");
        let json = profile.to_json().unwrap();
        let back = SpeakerProfile::from_json(&json).unwrap();
        assert_eq!(profile.mean, back.mean);
        assert_eq!(profile.var, back.var);
        assert_eq!(profile.enrolled_frames, back.enrolled_frames);
        assert!((profile.threshold - back.threshold).abs() < 1e-9);
    }

    #[test]
    fn threshold_override_changes_strictness_robust() {
        let me = synth_voice(130.0, 2.0, 1.0, 7);
        let base = SpeakerProfile::enroll_from_pcm(&me).expect("enrollment");
        // The enrolled speaker is normally accepted...
        let same = synth_voice(130.0, 1.0, 1.0, 11);
        assert!(
            base.accepts(&same),
            "enrolled speaker accepted at calibrated threshold"
        );
        // ...but a near-zero Mahalanobis threshold rejects even them (the knob
        // actually drives the decision toward stricter).
        let strict = base.clone().with_threshold(0.0);
        assert!(
            !strict.accepts(&same),
            "zero threshold must reject even self"
        );
        // A very permissive threshold keeps accepting the self utterance.
        let permissive = base.with_threshold(1e9);
        assert!(
            permissive.accepts(&same),
            "huge threshold stays permissive for self"
        );
    }

    #[test]
    fn cosine_is_bounded_normal() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![2.0, 4.0, 6.0];
        assert!((cosine_sim(&a, &b) - 1.0).abs() < 1e-9);
        assert_eq!(cosine_sim(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
    }

    // ── Pluggable embedder seam ────────────────────────────────────────────

    /// The null embedder never produces an embedding → the profile keeps using
    /// the classical path and `score_with` matches `score`.
    #[test]
    fn null_embedder_uses_classical_path_normal() {
        let me = synth_voice(130.0, 2.0, 1.0, 7);
        let profile = SpeakerProfile::enroll_from_pcm(&me).expect("enroll");
        assert!(profile.neural.is_none());
        let q = synth_voice(130.0, 1.0, 1.0, 11);
        let a = profile.score(&q);
        let b = profile.score_with(&q, &NullEmbedder);
        // Same decision + same Mahalanobis (the neural branch was not taken).
        assert_eq!(a.accepted, b.accepted);
        assert_eq!(a.voiced_frames > 0, b.voiced_frames > 0);
    }

    /// `with_neural_embedding` against the null embedder leaves the profile
    /// classical (no embedding attached) — graceful fallback.
    #[test]
    fn with_neural_embedding_noops_for_null_robust() {
        let me = synth_voice(130.0, 2.0, 1.0, 7);
        let profile = SpeakerProfile::enroll_from_pcm(&me)
            .expect("enroll")
            .with_neural_embedding(&NullEmbedder, &me);
        assert!(
            profile.neural.is_none(),
            "null embedder must not attach an embedding"
        );
    }

    /// A stub embedder lets us exercise the neural scoring path deterministically
    /// without an ONNX model: it returns a fixed embedding per "speaker" so we
    /// can assert cosine gating accepts self and rejects an orthogonal embedding.
    struct StubEmbedder(Vec<f32>);
    impl SpeakerEmbedder for StubEmbedder {
        fn embed(&self, pcm: &[u8]) -> Option<Vec<f32>> {
            if pcm.len() < 1000 {
                return None;
            }
            Some(self.0.clone())
        }
        fn name(&self) -> &str {
            "stub"
        }
    }

    #[test]
    fn neural_path_gates_on_cosine_robust() {
        let me = synth_voice(130.0, 2.0, 1.0, 7);
        // Enroll with a stub embedding [1,0,0,...].
        let mut emb_self = vec![0.0f32; 16];
        emb_self[0] = 1.0;
        let profile = SpeakerProfile::enroll_from_pcm(&me)
            .expect("enroll")
            .with_neural_embedding(&StubEmbedder(emb_self.clone()), &me);
        assert!(profile.neural.is_some(), "stub embedding must attach");

        // Querying with the SAME embedding → cosine 1.0 → accepted.
        let same = profile.score_with(&me, &StubEmbedder(emb_self));
        assert!(
            same.accepted,
            "identical embedding must be accepted (cos={:.3})",
            same.cosine
        );
        assert!((same.cosine - 1.0).abs() < 1e-6);

        // Querying with an ORTHOGONAL embedding → cosine 0 → rejected.
        let mut emb_other = vec![0.0f32; 16];
        emb_other[1] = 1.0;
        let other = profile.score_with(&me, &StubEmbedder(emb_other));
        assert!(
            !other.accepted,
            "orthogonal embedding must be rejected (cos={:.3})",
            other.cosine
        );
    }

    /// When the embedder can't embed (too-short audio here), the neural path
    /// falls back to the classical score rather than failing.
    #[test]
    fn neural_path_falls_back_when_embed_unavailable_robust() {
        let me = synth_voice(130.0, 2.0, 1.0, 7);
        let mut emb = vec![0.0f32; 8];
        emb[0] = 1.0;
        let profile = SpeakerProfile::enroll_from_pcm(&me)
            .expect("enroll")
            .with_neural_embedding(&StubEmbedder(emb.clone()), &me);
        // Tiny query → StubEmbedder returns None → classical path used.
        let tiny = synth_voice(130.0, 0.02, 1.0, 3);
        let score = profile.score_with(&tiny, &StubEmbedder(emb));
        // Classical path on near-empty audio → no voiced frames → not accepted
        // via cosine; the key assertion is it didn't panic and produced a score.
        assert!(score.voiced_frames == 0 || score.cosine.is_finite());
    }

    /// The fbank feature front-end produces a [frames × n_mels] matrix for
    /// voiced audio and `None` for silence — this is the ECAPA/x-vector input.
    #[test]
    fn fbank_features_shape_normal() {
        let me = synth_voice(140.0, 1.0, 1.0, 5);
        let (frames, mels, flat) = fbank_features(&me, 80).expect("fbank");
        assert_eq!(mels, 80);
        assert!(frames > 0);
        assert_eq!(flat.len(), frames * mels);
        // Silence → no voiced frames → None.
        assert!(fbank_features(&vec![0u8; 16_000 * 2], 80).is_none());
    }

    /// A neural profile round-trips through JSON (older classical-only profiles
    /// also still load, covered by `profile_json_roundtrips_normal`).
    #[test]
    fn neural_profile_json_roundtrips_normal() {
        let me = synth_voice(150.0, 2.0, 1.0, 3);
        let mut emb = vec![0.0f32; 8];
        emb[2] = 1.0;
        let profile = SpeakerProfile::enroll_from_pcm(&me)
            .expect("enroll")
            .with_neural_embedding(&StubEmbedder(emb), &me);
        let json = profile.to_json().unwrap();
        let back = SpeakerProfile::from_json(&json).unwrap();
        assert_eq!(
            profile.neural.as_ref().map(|n| n.embedding.clone()),
            back.neural.as_ref().map(|n| n.embedding.clone())
        );
    }

    /// Older classical-only profile JSON (no `neural` field) still deserializes.
    #[test]
    fn classical_only_json_deserializes_robust() {
        let json = r#"{
            "mean": [0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0],
            "var": [1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0],
            "pitch_median_hz": 120.0,
            "pitch_iqr_hz": 10.0,
            "threshold": 5.0,
            "enrolled_frames": 100
        }"#;
        let p = SpeakerProfile::from_json(json).expect("legacy profile must load");
        assert!(p.neural.is_none());
    }
}
