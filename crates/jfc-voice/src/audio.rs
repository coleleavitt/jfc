//! Audio capture — platform-adaptive raw PCM recording.
//!
//! Captures 16-bit signed PCM at 16 kHz mono (the format the STT backends
//! all expect). Recording backends are tried in order:
//!
//! 1. `arecord` (ALSA — standard on Linux)
//! 2. `rec` (SoX — fallback, works on Linux/macOS/WSL)
//! 3. `ffmpeg` (broadest platform support)
//!
//! Audio is streamed from the subprocess stdout. The caller accumulates
//! chunks via [`AudioCapture::read_chunk`] until [`AudioCapture::stop`] is
//! called.

use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

/// Supported recording backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureBackend {
    Arecord,
    Sox,
    Ffmpeg,
}

impl CaptureBackend {
    pub fn label(self) -> &'static str {
        match self {
            Self::Arecord => "arecord",
            Self::Sox => "rec (sox)",
            Self::Ffmpeg => "ffmpeg",
        }
    }
}

/// Live audio capture session. Drop or call [`stop`] to end recording.
pub struct AudioCapture {
    child: Child,
    pub backend: CaptureBackend,
}

impl AudioCapture {
    /// Detect which recording backend is available.
    pub async fn detect_backend() -> Option<CaptureBackend> {
        if which("arecord") {
            return Some(CaptureBackend::Arecord);
        }
        if which("rec") {
            return Some(CaptureBackend::Sox);
        }
        if which("ffmpeg") {
            return Some(CaptureBackend::Ffmpeg);
        }
        None
    }

    /// Probe whether audio recording is available and return a human-readable
    /// description of the first working backend.
    pub async fn check_availability() -> Result<CaptureBackend, String> {
        Self::detect_backend().await.ok_or_else(|| {
            "No audio recording tool found. Install arecord (ALSA), sox, or ffmpeg.".to_owned()
        })
    }

    /// Start recording raw PCM (16-bit signed LE, 16 kHz, mono).
    /// Returns a handle for reading audio chunks and stopping.
    pub async fn start(backend: CaptureBackend) -> anyhow::Result<Self> {
        let child = match backend {
            CaptureBackend::Arecord => {
                // Output raw PCM to stdout; stderr stays for diagnostics
                Command::new("arecord")
                    .args([
                        "-f", "S16_LE", // 16-bit signed little-endian
                        "-r", "16000", // 16 kHz
                        "-c", "1", // mono
                        "-t", "raw", // raw PCM (no WAV header)
                        "-",   // stdout
                    ])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()?
            }
            CaptureBackend::Sox => Command::new("rec")
                .args([
                    "-r", "16000", "-c", "1", "-e", "signed", "-b", "16", "-t", "raw", "-",
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()?,
            CaptureBackend::Ffmpeg => {
                Command::new("ffmpeg")
                    .args([
                        "-f",
                        "alsa",
                        "-i",
                        "default",
                        "-ar",
                        "16000",
                        "-ac",
                        "1",
                        "-f",
                        "s16le", // raw PCM, no container
                        "-",     // stdout
                        "-loglevel",
                        "quiet",
                    ])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()?
            }
        };
        Ok(Self { child, backend })
    }

    /// Read up to `buf.len()` bytes of audio from the recorder.
    /// Returns the number of bytes read (0 = EOF).
    pub async fn read_chunk(&mut self, buf: &mut [u8]) -> anyhow::Result<usize> {
        let stdout = self
            .child
            .stdout
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("no stdout on recorder process"))?;
        Ok(stdout.read(buf).await?)
    }

    /// Read all audio until the process ends or we stop it.
    pub async fn read_all(&mut self) -> anyhow::Result<Vec<u8>> {
        let stdout = self
            .child
            .stdout
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("no stdout on recorder process"))?;
        let mut buf = Vec::new();
        stdout.read_to_end(&mut buf).await?;
        Ok(buf)
    }

    /// Stop recording and collect the remaining audio.
    pub async fn stop(mut self) -> Vec<u8> {
        // Kill the subprocess so we get EOF on stdout.
        // Ignore the kill error — process may have already exited.
        if let Err(err) = self.child.kill().await {
            tracing::debug!(
                target: "jfc::voice::audio",
                error = %err,
                "recorder process already exited before stop()"
            );
        }
        // Drain whatever is already buffered.
        if let Some(stdout) = self.child.stdout.take() {
            let mut buf = Vec::new();
            let mut reader = tokio::io::BufReader::new(stdout);
            if let Err(err) = reader.read_to_end(&mut buf).await {
                tracing::debug!(
                    target: "jfc::voice::audio",
                    error = %err,
                    "error draining audio stdout after stop"
                );
            }
            return buf;
        }
        Vec::new()
    }
}

/// Wrap raw PCM in a minimal WAV header so STT APIs that require WAV work.
///
/// Format: PCM signed 16-bit LE, 16 kHz, mono.
pub fn wrap_wav(pcm: &[u8]) -> Vec<u8> {
    let data_len = pcm.len() as u32;
    let channels: u16 = 1;
    let sample_rate: u32 = 16_000;
    let bits_per_sample: u16 = 16;
    let block_align: u16 = channels * bits_per_sample / 8;
    let byte_rate: u32 = sample_rate * block_align as u32;
    let riff_size: u32 = 36 + data_len;

    let mut wav = Vec::with_capacity(44 + pcm.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&riff_size.to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // chunk size
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM format
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&bits_per_sample.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(pcm);
    wav
}

use crate::platform::which;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_wav_header_normal() {
        let pcm = vec![0u8; 320]; // 10ms at 16kHz 16-bit mono
        let wav = wrap_wav(&pcm);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(wav.len(), 44 + 320);
    }

    #[test]
    fn wrap_wav_sample_rate_normal() {
        let wav = wrap_wav(&[]);
        // sample rate at offset 24: 16000 LE = [0x80, 0x3e, 0x00, 0x00]
        let sr = u32::from_le_bytes(wav[24..28].try_into().unwrap());
        assert_eq!(sr, 16_000);
    }
}
