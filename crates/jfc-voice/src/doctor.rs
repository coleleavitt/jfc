//! Voice self-diagnostic — probes the mic and measures captured signal level.
//!
//! Answers the question "is my microphone actually picking up my voice?"
//! by recording a short sample and computing the same RMS energy the VAD
//! uses. Surfaces the three failure modes that block voice mode:
//!
//! 1. **No backend** — no arecord/sox/ffmpeg on PATH.
//! 2. **Silence** — mic captured ~0 amplitude (muted, wrong device, no perms).
//! 3. **Too quiet** — speech registers but below the VAD threshold; suggests
//!    a `JFC_VAD_THRESHOLD` value that would work.

use std::time::Duration;

use crate::audio::{AudioCapture, CaptureBackend};
use crate::vad::rms_energy;

/// How long to record for a diagnostic sample.
const SAMPLE_DURATION: Duration = Duration::from_secs(3);
/// 20ms frame at 16kHz 16-bit mono = 320 samples = 640 bytes.
const FRAME_BYTES: usize = 640;
/// Default VAD energy threshold (matches `vad::Vad::new`).
const DEFAULT_THRESHOLD: u32 = 300;
/// Below this peak amplitude we treat capture as effective silence.
const SILENCE_PEAK: i16 = 50;

/// Outcome of a voice diagnostic run.
#[derive(Debug, Clone)]
pub struct VoiceDiagnostic {
    /// Which capture backend was used.
    pub backend: Option<CaptureBackend>,
    /// Total bytes captured.
    pub bytes_captured: usize,
    /// Peak absolute amplitude (0..32767).
    pub peak_amplitude: i16,
    /// Maximum 20ms-frame RMS energy.
    pub max_frame_rms: u32,
    /// How many frames exceeded the default VAD threshold.
    pub voiced_frames: usize,
    /// Total frames analyzed.
    pub total_frames: usize,
    /// Human-readable verdict.
    pub verdict: Verdict,
}

/// The diagnostic verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// No audio recording backend found.
    NoBackend,
    /// Mic captured silence (muted / wrong device / no permission).
    Silence,
    /// Speech detected but below threshold; carries a suggested threshold.
    TooQuiet { suggested_threshold: u32 },
    /// Mic works and VAD would trigger at the default threshold.
    Working,
}

impl Verdict {
    /// Short one-line summary for display.
    pub fn summary(&self) -> String {
        match self {
            Self::NoBackend => {
                "No audio backend found. Install arecord (ALSA), sox, or ffmpeg.".to_owned()
            }
            Self::Silence => "Mic captured silence. Check it isn't muted, the right device is \
                 selected, and the app has microphone permission."
                .to_owned(),
            Self::TooQuiet {
                suggested_threshold,
            } => format!(
                "Mic works but your voice is quiet. Set JFC_VAD_THRESHOLD={suggested_threshold} \
                 (or raise your system mic volume)."
            ),
            Self::Working => {
                "Mic works — voice activity detection will trigger at the default threshold."
                    .to_owned()
            }
        }
    }

    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Working | Self::TooQuiet { .. })
    }
}

/// Run a full voice diagnostic: record a short sample and analyze it.
///
/// Records [`SAMPLE_DURATION`] of audio from the default mic in the exact
/// format the STT pipeline uses (16kHz 16-bit mono), then reports peak
/// amplitude, frame RMS, and whether the VAD would trigger.
pub async fn run_diagnostic() -> VoiceDiagnostic {
    let Some(backend) = AudioCapture::detect_backend().await else {
        return VoiceDiagnostic {
            backend: None,
            bytes_captured: 0,
            peak_amplitude: 0,
            max_frame_rms: 0,
            voiced_frames: 0,
            total_frames: 0,
            verdict: Verdict::NoBackend,
        };
    };

    let pcm = match record_sample(backend).await {
        Ok(pcm) => pcm,
        Err(_) => {
            return VoiceDiagnostic {
                backend: Some(backend),
                bytes_captured: 0,
                peak_amplitude: 0,
                max_frame_rms: 0,
                voiced_frames: 0,
                total_frames: 0,
                verdict: Verdict::Silence,
            };
        }
    };

    analyze(backend, &pcm)
}

/// Record a fixed-duration sample by reading frames until the deadline.
async fn record_sample(backend: CaptureBackend) -> anyhow::Result<Vec<u8>> {
    let mut capture = AudioCapture::start(backend).await?;
    let mut pcm = Vec::with_capacity(96_000);
    let mut chunk = vec![0u8; FRAME_BYTES];
    let deadline = tokio::time::Instant::now() + SAMPLE_DURATION;

    loop {
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        let read =
            tokio::time::timeout(Duration::from_millis(500), capture.read_chunk(&mut chunk)).await;
        match read {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => pcm.extend_from_slice(&chunk[..n]),
            Ok(Err(_)) => break,
            Err(_) => break, // read timeout — treat as silence/stall
        }
    }
    let tail = capture.stop().await;
    pcm.extend_from_slice(&tail);
    Ok(pcm)
}

/// Analyze captured PCM and produce a verdict. Pure function — unit-testable.
pub fn analyze(backend: CaptureBackend, pcm: &[u8]) -> VoiceDiagnostic {
    let peak = peak_amplitude(pcm);
    let (max_frame_rms, voiced_frames, total_frames) = frame_stats(pcm);

    let verdict = if peak < SILENCE_PEAK {
        Verdict::Silence
    } else if voiced_frames >= 3 {
        Verdict::Working
    } else {
        // Suggest a threshold at ~50% of the loudest frame, floored at 80.
        let suggested = (max_frame_rms / 2).max(80);
        Verdict::TooQuiet {
            suggested_threshold: suggested,
        }
    };

    VoiceDiagnostic {
        backend: Some(backend),
        bytes_captured: pcm.len(),
        peak_amplitude: peak,
        max_frame_rms,
        voiced_frames,
        total_frames,
        verdict,
    }
}

/// Peak absolute amplitude across all 16-bit LE samples.
fn peak_amplitude(pcm: &[u8]) -> i16 {
    pcm.chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]).saturating_abs())
        .max()
        .unwrap_or(0)
}

/// `(max_frame_rms, voiced_frame_count, total_frames)` over 20ms frames.
fn frame_stats(pcm: &[u8]) -> (u32, usize, usize) {
    let mut max_rms = 0u32;
    let mut voiced = 0usize;
    let mut total = 0usize;
    let mut pos = 0;
    while pos + FRAME_BYTES <= pcm.len() {
        let rms = rms_energy(&pcm[pos..pos + FRAME_BYTES]);
        max_rms = max_rms.max(rms);
        if rms > DEFAULT_THRESHOLD {
            voiced += 1;
        }
        total += 1;
        pos += FRAME_BYTES;
    }
    (max_rms, voiced, total)
}

/// Render a full diagnostic report as human-readable text.
pub fn format_report(diag: &VoiceDiagnostic) -> String {
    let backend = diag.backend.map(|b| b.label()).unwrap_or("none");
    let pct = if diag.peak_amplitude > 0 {
        100.0 * diag.peak_amplitude as f64 / 32767.0
    } else {
        0.0
    };
    format!(
        "Voice diagnostic\n\
         ─────────────────────────────────────\n\
         backend:        {backend}\n\
         captured:       {} bytes\n\
         peak amplitude: {} / 32767 ({pct:.1}% of full scale)\n\
         max frame RMS:  {}\n\
         voiced frames:  {} / {} (threshold {DEFAULT_THRESHOLD})\n\
         ─────────────────────────────────────\n\
         {} {}",
        diag.bytes_captured,
        diag.peak_amplitude,
        diag.max_frame_rms,
        diag.voiced_frames,
        diag.total_frames,
        if diag.verdict.is_ok() { "✓" } else { "✗" },
        diag.verdict.summary(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn silent(samples: usize) -> Vec<u8> {
        vec![0u8; samples * 2]
    }

    fn loud(samples: usize, amplitude: i16) -> Vec<u8> {
        (0..samples)
            .flat_map(|i| {
                let v = if i % 2 == 0 { amplitude } else { -amplitude };
                v.to_le_bytes()
            })
            .collect()
    }

    #[test]
    fn analyze_silence_normal() {
        let pcm = silent(16_000); // 1s of silence
        let diag = analyze(CaptureBackend::Arecord, &pcm);
        assert_eq!(diag.verdict, Verdict::Silence);
        assert_eq!(diag.peak_amplitude, 0);
        assert!(!diag.verdict.is_ok());
    }

    #[test]
    fn analyze_loud_speech_working_normal() {
        let pcm = loud(16_000, 5000); // loud across many frames
        let diag = analyze(CaptureBackend::Arecord, &pcm);
        assert_eq!(diag.verdict, Verdict::Working);
        assert!(diag.voiced_frames >= 3);
        assert!(diag.verdict.is_ok());
    }

    #[test]
    fn analyze_quiet_speech_suggests_threshold_robust() {
        // Amplitude ~200 → RMS ~200, above SILENCE_PEAK(50) but below threshold(300)
        let pcm = loud(16_000, 200);
        let diag = analyze(CaptureBackend::Arecord, &pcm);
        match diag.verdict {
            Verdict::TooQuiet {
                suggested_threshold,
            } => {
                assert!(suggested_threshold >= 80);
                assert!(suggested_threshold < DEFAULT_THRESHOLD);
            }
            other => panic!("expected TooQuiet, got {other:?}"),
        }
    }

    #[test]
    fn peak_amplitude_normal() {
        let pcm = loud(100, 1234);
        assert_eq!(peak_amplitude(&pcm), 1234);
    }

    #[test]
    fn frame_stats_counts_voiced_normal() {
        let pcm = loud(3200, 5000); // 10 frames, all loud
        let (max_rms, voiced, total) = frame_stats(&pcm);
        assert!(max_rms > 300);
        assert_eq!(voiced, total);
        assert_eq!(total, 10);
    }

    #[test]
    fn format_report_contains_verdict_normal() {
        let diag = analyze(CaptureBackend::Arecord, &loud(16_000, 5000));
        let report = format_report(&diag);
        assert!(report.contains("Voice diagnostic"));
        assert!(report.contains("peak amplitude"));
        assert!(report.contains("voiced frames"));
    }
}
