//! Local media generation helpers for design artifacts.
//!
//! These are deterministic local fallbacks. They give the design workspace real
//! generated assets even when no external image/audio provider keys are present.

use std::f32::consts::PI;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{DesignError, Result, io_err};

#[derive(Debug, Clone, Deserialize)]
pub struct GenerateImageRequest {
    pub prompt: String,
    pub output: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub style: Option<String>,
    pub provider: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GenerateSoundRequest {
    pub prompt: String,
    pub output: Option<String>,
    pub duration_ms: Option<u32>,
    pub sample_rate: Option<u32>,
    pub provider: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GeneratedMedia {
    pub output: String,
    pub bytes: usize,
    pub provider: String,
    pub mime: String,
    pub prompt: String,
}

pub fn generate_image(root: &Path, req: GenerateImageRequest) -> Result<GeneratedMedia> {
    let prompt = clean_prompt(&req.prompt);
    if wants_external(req.provider.as_deref())
        && let Some(generated) = external_media(
            root,
            "image",
            "JFC_DESIGN_IMAGE_PROVIDER_CMD",
            json!({
                "prompt": prompt,
                "output": req.output,
                "width": req.width,
                "height": req.height,
                "style": req.style,
                "provider": req.provider,
            }),
        )?
    {
        return Ok(generated);
    }
    let width = req.width.unwrap_or(1280).clamp(256, 4096);
    let height = req.height.unwrap_or(720).clamp(256, 4096);
    let output = req
        .output
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| format!("generated/{}.svg", slug(&prompt, "image")));
    let style = req.style.unwrap_or_else(|| "editorial".to_owned());
    let svg = generated_svg(&prompt, &style, width, height);
    let path = root.join(&output);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
    }
    std::fs::write(&path, svg.as_bytes()).map_err(|e| io_err(&path, e))?;
    Ok(GeneratedMedia {
        output,
        bytes: svg.len(),
        provider: "local-svg".to_owned(),
        mime: "image/svg+xml".to_owned(),
        prompt,
    })
}

pub fn generate_sound(root: &Path, req: GenerateSoundRequest) -> Result<GeneratedMedia> {
    let prompt = clean_prompt(&req.prompt);
    if wants_external(req.provider.as_deref())
        && let Some(generated) = external_media(
            root,
            "sound",
            "JFC_DESIGN_SOUND_PROVIDER_CMD",
            json!({
                "prompt": prompt,
                "output": req.output,
                "duration_ms": req.duration_ms,
                "sample_rate": req.sample_rate,
                "provider": req.provider,
            }),
        )?
    {
        return Ok(generated);
    }
    let output = req
        .output
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| format!("generated/{}.wav", slug(&prompt, "sound")));
    let sample_rate = req.sample_rate.unwrap_or(44_100).clamp(8_000, 96_000);
    let duration_ms = req.duration_ms.unwrap_or(1800).clamp(200, 30_000);
    let wav = generated_wav(&prompt, duration_ms, sample_rate);
    let path = root.join(&output);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
    }
    std::fs::write(&path, &wav).map_err(|e| io_err(&path, e))?;
    Ok(GeneratedMedia {
        output,
        bytes: wav.len(),
        provider: "local-wav-synth".to_owned(),
        mime: "audio/wav".to_owned(),
        prompt,
    })
}

fn wants_external(provider: Option<&str>) -> bool {
    !matches!(provider, None | Some("") | Some("auto") | Some("local"))
        || std::env::var("JFC_DESIGN_IMAGE_PROVIDER_CMD").is_ok()
        || std::env::var("JFC_DESIGN_SOUND_PROVIDER_CMD").is_ok()
}

fn external_media(
    root: &Path,
    kind: &str,
    env_name: &str,
    payload: Value,
) -> Result<Option<GeneratedMedia>> {
    let Ok(command) = std::env::var(env_name) else {
        return Ok(None);
    };
    let command = command.trim();
    if command.is_empty() {
        return Ok(None);
    }
    let mut parts = command.split_whitespace();
    let Some(program) = parts.next() else {
        return Ok(None);
    };
    let mut child = Command::new(program)
        .args(parts)
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| DesignError::Bundle(format!("{kind} provider failed to start: {e}")))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(serde_json::to_string(&payload)?.as_bytes())
            .map_err(|e| DesignError::Bundle(format!("{kind} provider stdin failed: {e}")))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|e| DesignError::Bundle(format!("{kind} provider wait failed: {e}")))?;
    if !output.status.success() {
        return Err(DesignError::Bundle(format!(
            "{kind} provider failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let value: Value = serde_json::from_slice(&output.stdout)?;
    let media_output = value
        .get("output")
        .and_then(Value::as_str)
        .filter(|path| !path.trim().is_empty())
        .ok_or_else(|| DesignError::Bundle(format!("{kind} provider response missing output")))?;
    let path = root.join(media_output);
    let bytes = value
        .get("bytes")
        .and_then(Value::as_u64)
        .map(|bytes| bytes as usize)
        .or_else(|| {
            std::fs::metadata(&path)
                .ok()
                .map(|meta| meta.len() as usize)
        })
        .unwrap_or(0);
    Ok(Some(GeneratedMedia {
        output: media_output.to_owned(),
        bytes,
        provider: value
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or(env_name)
            .to_owned(),
        mime: value
            .get("mime")
            .and_then(Value::as_str)
            .unwrap_or(if kind == "image" {
                "image/png"
            } else {
                "audio/mpeg"
            })
            .to_owned(),
        prompt: value
            .get("prompt")
            .and_then(Value::as_str)
            .or_else(|| payload.get("prompt").and_then(Value::as_str))
            .unwrap_or("Generated media")
            .to_owned(),
    }))
}

fn generated_svg(prompt: &str, style: &str, width: u32, height: u32) -> String {
    let seed = hash(prompt.as_bytes());
    let c1 = color(seed);
    let c2 = color(seed.rotate_left(11));
    let c3 = color(seed.rotate_left(23));
    let text = xml_escape(prompt);
    let style = xml_escape(style);
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" width="{width}" height="{height}">
  <defs>
    <linearGradient id="g" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#{c1}"/>
      <stop offset="0.55" stop-color="#{c2}"/>
      <stop offset="1" stop-color="#{c3}"/>
    </linearGradient>
    <filter id="soft"><feGaussianBlur stdDeviation="18"/></filter>
  </defs>
  <rect width="{width}" height="{height}" fill="url(#g)"/>
  <g opacity="0.28" filter="url(#soft)">
    <circle cx="{cx1}" cy="{cy1}" r="{r1}" fill="#ffffff"/>
    <circle cx="{cx2}" cy="{cy2}" r="{r2}" fill="#0b1020"/>
  </g>
  <g fill="none" stroke="#ffffff" stroke-opacity="0.38" stroke-width="2">
    <path d="M {p1} {h1} C {p2} {h2}, {p3} {h3}, {p4} {h4}"/>
    <path d="M {p1} {hh1} C {p2} {hh2}, {p3} {hh3}, {p4} {hh4}"/>
  </g>
  <rect x="{pad}" y="{pad}" width="{box_w}" height="{box_h}" rx="8" fill="#ffffff" fill-opacity="0.84"/>
  <text x="{text_x}" y="{text_y}" fill="#14211f" font-family="Inter, ui-sans-serif, system-ui" font-size="{font}" font-weight="700">{text}</text>
  <text x="{text_x}" y="{sub_y}" fill="#3f4d4a" font-family="Inter, ui-sans-serif, system-ui" font-size="{sub_font}">Generated locally by JFC Design · {style}</text>
</svg>
"##,
        cx1 = width / 5,
        cy1 = height / 4,
        r1 = height / 3,
        cx2 = width * 4 / 5,
        cy2 = height * 3 / 4,
        r2 = height / 4,
        p1 = width / 14,
        p2 = width / 3,
        p3 = width * 2 / 3,
        p4 = width * 13 / 14,
        h1 = height / 3,
        h2 = height / 8,
        h3 = height * 7 / 8,
        h4 = height * 2 / 3,
        hh1 = height * 2 / 3,
        hh2 = height * 7 / 8,
        hh3 = height / 8,
        hh4 = height / 3,
        pad = width.min(height) / 16,
        box_w = width * 7 / 10,
        box_h = height / 5,
        text_x = width.min(height) / 16 + 28,
        text_y = width.min(height) / 16 + 62,
        sub_y = width.min(height) / 16 + 108,
        font = (width / 34).clamp(24, 52),
        sub_font = (width / 64).clamp(14, 24),
    )
}

fn generated_wav(prompt: &str, duration_ms: u32, sample_rate: u32) -> Vec<u8> {
    let seed = hash(prompt.as_bytes());
    let frames = (u64::from(duration_ms) * u64::from(sample_rate) / 1000) as usize;
    let base = 180.0 + (seed % 220) as f32;
    let harmonic = base * (1.25 + ((seed >> 8) % 60) as f32 / 100.0);
    let mut pcm = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let t = i as f32 / sample_rate as f32;
        let fade_in = (i as f32 / (sample_rate as f32 * 0.04)).min(1.0);
        let fade_out = ((frames - i) as f32 / (sample_rate as f32 * 0.08)).min(1.0);
        let env = fade_in.min(fade_out) * 0.32;
        let sample = ((2.0 * PI * base * t).sin() * 0.7
            + (2.0 * PI * harmonic * t).sin() * 0.25
            + (2.0 * PI * (base / 2.0) * t).sin() * 0.2)
            * env;
        let v = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        pcm.extend_from_slice(&v.to_le_bytes());
    }
    wav_header(sample_rate, frames as u32, &pcm)
}

fn wav_header(sample_rate: u32, frames: u32, pcm: &[u8]) -> Vec<u8> {
    let data_len = frames * 2;
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

fn clean_prompt(prompt: &str) -> String {
    let clean = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.is_empty() {
        "Untitled generated asset".to_owned()
    } else {
        clean.chars().take(180).collect()
    }
}

fn slug(prompt: &str, fallback: &str) -> String {
    let mut out = String::new();
    for c in prompt.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-');
    if out.is_empty() {
        fallback.to_owned()
    } else {
        out.chars().take(48).collect()
    }
}

fn hash(bytes: &[u8]) -> u64 {
    let mut out = 0xcbf29ce484222325u64;
    for byte in bytes {
        out ^= u64::from(*byte);
        out = out.wrapping_mul(0x100000001b3);
    }
    out
}

fn color(seed: u64) -> String {
    let r = 48 + (seed & 0x7f) as u8;
    let g = 64 + ((seed >> 9) & 0x8f) as u8;
    let b = 72 + ((seed >> 18) & 0x8f) as u8;
    format!("{r:02X}{g:02X}{b:02X}")
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
