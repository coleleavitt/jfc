use anyhow::{Context, Result};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::config::VoiceConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackCommand {
    program: String,
    args: Vec<String>,
}

impl PlaybackCommand {
    pub fn new(
        program: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
        }
    }
}

pub struct PcmPlayback {
    program: String,
    child: tokio::process::Child,
    stdin: Option<tokio::process::ChildStdin>,
}

impl PcmPlayback {
    pub fn start(cfg: &VoiceConfig) -> Result<Self> {
        let command = detect_playback_command(
            cfg.tts_playback_command.as_deref(),
            cfg.selected_speaker_device_id.as_deref(),
        )
            .context("no PCM playback command found (install ffmpeg/mpv/pulseaudio-utils/alsa-utils or configure voice.tts_playback_command)")?;
        Self::start_command(command)
    }

    pub fn start_command(command: PlaybackCommand) -> Result<Self> {
        let mut child = Command::new(&command.program)
            .args(&command.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .with_context(|| format!("failed to spawn {}", command.program))?;
        let stdin = child
            .stdin
            .take()
            .context("playback command stdin unavailable")?;
        Ok(Self {
            program: command.program,
            child,
            stdin: Some(stdin),
        })
    }

    pub async fn write_audio(&mut self, pcm: &[u8]) -> Result<()> {
        let stdin = self
            .stdin
            .as_mut()
            .context("playback command stdin closed")?;
        stdin.write_all(pcm).await?;
        Ok(())
    }

    /// Kill the playback process immediately (barge-in). Drops any buffered
    /// audio still queued in the player rather than letting it drain.
    pub fn kill(&mut self) {
        drop(self.stdin.take());
        let _ = self.child.start_kill();
    }

    pub async fn finish(mut self) -> Result<()> {
        drop(self.stdin.take());
        let status = self
            .child
            .wait()
            .await
            .with_context(|| format!("failed to wait for {}", self.program))?;
        if !status.success() {
            return Err(anyhow::anyhow!(
                "playback command {} exited with {status}",
                self.program
            ));
        }
        Ok(())
    }
}

/// The ffplay command for raw 16 kHz mono s16le PCM on stdin.
///
/// Uses `-ch_layout mono`, NOT `-ac 1`: FFmpeg 5.1+ ffplay rejects `-ac`
/// ("Option not found" on 8.x), which makes ffplay exit immediately → broken
/// pipe → silent no-audio. This was the read-aloud "no sound" bug.
fn ffplay_pcm_command() -> PlaybackCommand {
    PlaybackCommand::new(
        "ffplay",
        [
            "-nodisp", "-autoexit", "-loglevel", "quiet", "-f", "s16le", "-ar", "16000",
            "-ch_layout", "mono", "-i", "pipe:0",
        ],
    )
}

pub fn detect_playback_command(
    override_cmd: Option<&str>,
    speaker_device_id: Option<&str>,
) -> Option<PlaybackCommand> {
    if let Some(cmd) = override_cmd.map(str::trim).filter(|cmd| !cmd.is_empty()) {
        return Some(PlaybackCommand::new("sh", ["-lc", cmd]));
    }
    if let Some(device_id) = speaker_device_id
        .map(str::trim)
        .filter(|device_id| !device_id.is_empty())
    {
        if crate::platform::which("paplay") {
            return Some(PlaybackCommand::new(
                "paplay",
                [
                    "--raw".to_owned(),
                    "--rate=16000".to_owned(),
                    "--channels=1".to_owned(),
                    "--format=s16le".to_owned(),
                    format!("--device={device_id}"),
                ],
            ));
        }
        if crate::platform::which("aplay") {
            return Some(PlaybackCommand::new(
                "aplay",
                [
                    "-q".to_owned(),
                    "-D".to_owned(),
                    device_id.to_owned(),
                    "-f".to_owned(),
                    "S16_LE".to_owned(),
                    "-r".to_owned(),
                    "16000".to_owned(),
                    "-c".to_owned(),
                    "1".to_owned(),
                ],
            ));
        }
    }
    if crate::platform::which("ffplay") {
        return Some(ffplay_pcm_command());
    }
    if crate::platform::which("mpv") {
        return Some(PlaybackCommand::new(
            "mpv",
            [
                "--no-terminal",
                "--really-quiet",
                "--demuxer=rawaudio",
                "--demuxer-rawaudio-format=s16le",
                "--demuxer-rawaudio-rate=16000",
                "--demuxer-rawaudio-channels=1",
                "-",
            ],
        ));
    }
    if crate::platform::which("paplay") {
        return Some(PlaybackCommand::new(
            "paplay",
            ["--raw", "--rate=16000", "--channels=1", "--format=s16le"],
        ));
    }
    if crate::platform::which("aplay") {
        return Some(PlaybackCommand::new(
            "aplay",
            ["-q", "-f", "S16_LE", "-r", "16000", "-c", "1"],
        ));
    }
    None
}

pub async fn speak_anthropic_tts(
    cfg: &VoiceConfig,
    token: &str,
    user_agent: &str,
    text: &str,
) -> Result<crate::tts::TtsStats> {
    let mut player = PcmPlayback::start(cfg)?;
    let stdin = player
        .stdin
        .as_mut()
        .context("playback command stdin closed")?;
    let stats = crate::tts::synthesize_to_writer(cfg, token, user_agent, text, stdin).await;
    player.finish().await?;
    stats
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_playback_command_uses_shell_normal() {
        let command = detect_playback_command(Some("aplay -q -f S16_LE"), Some("ignored")).unwrap();
        assert_eq!(command.program, "sh");
        assert_eq!(command.args, vec!["-lc", "aplay -q -f S16_LE"]);
    }

    #[test]
    fn ffplay_command_avoids_ac_flag_regression() {
        // Regression: `-ac 1` makes FFmpeg 5.1+ ffplay exit ("Option not found")
        // → broken pipe → no audio. Must use `-ch_layout mono` instead.
        let cmd = ffplay_pcm_command();
        assert_eq!(cmd.program, "ffplay");
        assert!(
            !cmd.args.iter().any(|a| a == "-ac"),
            "ffplay must not use -ac: {:?}",
            cmd.args
        );
        let joined = cmd.args.join(" ");
        assert!(joined.contains("-ch_layout mono"), "args: {joined}");
        assert!(joined.contains("-f s16le"), "args: {joined}");
        assert!(joined.contains("-ar 16000"), "args: {joined}");
    }
}
