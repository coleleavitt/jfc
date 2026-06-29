use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::process::Command;

use crate::runtime::ExecutionResult;

const HCOM_BIN_ENV: &str = "JFC_HCOM_BIN";
const HCOM_OUTPUT_LIMIT: usize = 200_000;

const HCOM_TOOL_NAMES: &[&str] = &[
    "HcomStatus",
    "HcomList",
    "HcomSend",
    "HcomEvents",
    "HcomListen",
    "HcomTranscript",
    "HcomBundle",
    "HcomTerm",
    "HcomLaunch",
    "HcomResume",
    "HcomFork",
    "HcomKill",
    "HcomRelay",
    "HcomRun",
];

pub fn is_hcom_tool_name(name: &str) -> bool {
    HCOM_TOOL_NAMES
        .iter()
        .any(|tool| tool.eq_ignore_ascii_case(name))
}

pub fn hcom_available() -> bool {
    hcom_binary().is_some()
}

pub fn system_prompt_section() -> Option<&'static str> {
    if hcom_available() {
        Some(
            "## hcom Agent Bus\n\n\
             hcom is available as an optional cross-agent coordination bus. \
             Use the Hcom* tools when you need to message, observe, subscribe \
             to, launch, resume, fork, kill, or inspect external coding-agent \
             sessions outside this JFC process. HcomSend sends addressed \
             messages to hcom agents; HcomList/HcomStatus/HcomEvents observe \
             the bus; HcomTranscript/HcomBundle pull handoff context; \
             HcomTerm controls PTY-backed agents; HcomLaunch/HcomResume/\
             HcomFork/HcomKill manage external agent lifecycle; HcomRelay \
             manages cross-device relay state. Prefer JFC's SendMessage for \
             in-process JFC teammates and HcomSend for external hcom agents.",
        )
    } else {
        None
    }
}

fn hcom_binary() -> Option<PathBuf> {
    if let Ok(path) = std::env::var(HCOM_BIN_ENV) {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            let candidate = PathBuf::from(trimmed);
            if candidate.exists() {
                return Some(candidate);
            }
            return None;
        }
    }
    which("hcom")
}

fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(bin))
        .find(|candidate| candidate.exists())
}

fn push_flag(args: &mut Vec<String>, flag: &str, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        args.push(flag.to_owned());
        args.push(value.to_owned());
    }
}

fn push_bool(args: &mut Vec<String>, flag: &str, value: Option<bool>) {
    if value == Some(true) {
        args.push(flag.to_owned());
    }
}

fn push_number(args: &mut Vec<String>, flag: &str, value: Option<u64>) {
    if let Some(value) = value {
        args.push(flag.to_owned());
        args.push(value.to_string());
    }
}

fn normalize_target(target: &str) -> Option<String> {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        None
    } else if trimmed.starts_with('@') {
        Some(trimmed.to_owned())
    } else {
        Some(format!("@{trimmed}"))
    }
}

fn append_extra(args: &mut Vec<String>, extra_args: Vec<String>) {
    args.extend(
        extra_args
            .into_iter()
            .map(|arg| arg.trim().to_owned())
            .filter(|arg| !arg.is_empty()),
    );
}

fn format_output(status: std::process::ExitStatus, stdout: &[u8], stderr: &[u8]) -> String {
    let mut output = String::new();
    let out = String::from_utf8_lossy(stdout);
    let err = String::from_utf8_lossy(stderr);

    if !out.trim().is_empty() {
        output.push_str(out.trim_end());
    }
    if !err.trim().is_empty() {
        if !output.is_empty() {
            output.push_str("\n\nstderr:\n");
        }
        output.push_str(err.trim_end());
    }
    if output.is_empty() {
        output.push_str(&format!("hcom exited with status {status} and no output"));
    }
    if output.len() > HCOM_OUTPUT_LIMIT {
        let end = output.floor_char_boundary(HCOM_OUTPUT_LIMIT);
        output.truncate(end);
        output.push_str("\n\n[truncated hcom output]");
    }
    output
}

async fn run_hcom(args: Vec<String>, cwd: &Path) -> ExecutionResult {
    let Some(bin) = hcom_binary() else {
        return ExecutionResult::failure(
            "hcom is not available on PATH. Install hcom or set JFC_HCOM_BIN to the hcom binary.",
        );
    };

    let display = format!("{} {}", bin.display(), args.join(" "));
    let output = Command::new(&bin)
        .args(&args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await;

    match output {
        Ok(output) if output.status.success() => {
            ExecutionResult::success(format_output(output.status, &output.stdout, &output.stderr))
        }
        Ok(output) => ExecutionResult::failure(format!(
            "hcom command failed: {display}\n\n{}",
            format_output(output.status, &output.stdout, &output.stderr)
        )),
        Err(error) => {
            ExecutionResult::failure(format!("failed to run hcom command `{display}`: {error}"))
        }
    }
}

pub async fn execute_hcom_status(
    json: Option<bool>,
    logs: Option<bool>,
    cwd: &Path,
) -> ExecutionResult {
    let mut args = vec!["status".to_owned()];
    push_bool(&mut args, "--json", json);
    push_bool(&mut args, "--logs", logs);
    run_hcom(args, cwd).await
}

#[allow(clippy::too_many_arguments)]
pub async fn execute_hcom_list(
    name: Option<String>,
    field: Option<String>,
    stopped: Option<bool>,
    json: Option<bool>,
    names: Option<bool>,
    verbose: Option<bool>,
    all: Option<bool>,
    last: Option<u64>,
    format: Option<String>,
    cwd: &Path,
) -> ExecutionResult {
    let mut args = vec!["list".to_owned()];
    push_bool(&mut args, "--stopped", stopped);
    push_bool(&mut args, "--json", json);
    push_bool(&mut args, "--names", names);
    push_bool(&mut args, "--verbose", verbose);
    push_bool(&mut args, "--all", all);
    push_number(&mut args, "--last", last);
    push_flag(&mut args, "--format", format.as_deref());
    if let Some(name) = name.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()) {
        args.push(name);
        if let Some(field) = field.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()) {
            args.push(field);
        }
    }
    run_hcom(args, cwd).await
}

#[allow(clippy::too_many_arguments)]
pub async fn execute_hcom_send(
    targets: Vec<String>,
    message: String,
    intent: Option<String>,
    reply_to: Option<String>,
    thread: Option<String>,
    from: Option<String>,
    title: Option<String>,
    description: Option<String>,
    events: Option<String>,
    files: Vec<String>,
    transcript: Option<String>,
    extends: Option<String>,
    cwd: &Path,
) -> ExecutionResult {
    let mut args = vec!["send".to_owned()];
    args.extend(targets.iter().filter_map(|target| normalize_target(target)));
    push_flag(&mut args, "--intent", intent.as_deref());
    push_flag(&mut args, "--reply-to", reply_to.as_deref());
    push_flag(&mut args, "--thread", thread.as_deref());
    push_flag(&mut args, "--from", from.as_deref());
    push_flag(&mut args, "--title", title.as_deref());
    push_flag(&mut args, "--description", description.as_deref());
    push_flag(&mut args, "--events", events.as_deref());
    if !files.is_empty() {
        args.push("--files".to_owned());
        args.push(files.join(","));
    }
    push_flag(&mut args, "--transcript", transcript.as_deref());
    push_flag(&mut args, "--extends", extends.as_deref());
    args.push("--".to_owned());
    args.push(message);
    run_hcom(args, cwd).await
}

pub async fn execute_hcom_events(args: Vec<String>, cwd: &Path) -> ExecutionResult {
    let mut command = vec!["events".to_owned()];
    append_extra(&mut command, args);
    run_hcom(command, cwd).await
}

pub async fn execute_hcom_listen(
    timeout: Option<u64>,
    json: Option<bool>,
    sql: Option<String>,
    args: Vec<String>,
    cwd: &Path,
) -> ExecutionResult {
    let mut command = vec!["listen".to_owned()];
    push_number(&mut command, "--timeout", timeout);
    push_bool(&mut command, "--json", json);
    push_flag(&mut command, "--sql", sql.as_deref());
    append_extra(&mut command, args);
    run_hcom(command, cwd).await
}

pub async fn execute_hcom_transcript(args: Vec<String>, cwd: &Path) -> ExecutionResult {
    let mut command = vec!["transcript".to_owned()];
    append_extra(&mut command, args);
    run_hcom(command, cwd).await
}

pub async fn execute_hcom_bundle(args: Vec<String>, cwd: &Path) -> ExecutionResult {
    let mut command = vec!["bundle".to_owned()];
    append_extra(&mut command, args);
    run_hcom(command, cwd).await
}

pub async fn execute_hcom_term(args: Vec<String>, cwd: &Path) -> ExecutionResult {
    let mut command = vec!["term".to_owned()];
    append_extra(&mut command, args);
    run_hcom(command, cwd).await
}

#[allow(clippy::too_many_arguments)]
pub async fn execute_hcom_launch(
    tool: String,
    count: Option<u64>,
    tag: Option<String>,
    terminal: Option<String>,
    headless: Option<bool>,
    device: Option<String>,
    dir: Option<String>,
    prompt: Option<String>,
    system_prompt: Option<String>,
    batch_id: Option<String>,
    run_here: Option<bool>,
    args: Vec<String>,
    cwd: &Path,
) -> ExecutionResult {
    let mut command = Vec::new();
    if let Some(count) = count.filter(|count| *count > 1) {
        command.push(count.to_string());
    }
    command.push(tool);
    push_flag(&mut command, "--tag", tag.as_deref());
    push_flag(&mut command, "--terminal", terminal.as_deref());
    push_bool(&mut command, "--headless", headless);
    push_flag(&mut command, "--device", device.as_deref());
    push_flag(&mut command, "--dir", dir.as_deref());
    push_flag(&mut command, "--hcom-prompt", prompt.as_deref());
    push_flag(
        &mut command,
        "--hcom-system-prompt",
        system_prompt.as_deref(),
    );
    push_flag(&mut command, "--batch-id", batch_id.as_deref());
    match run_here {
        Some(true) => command.push("--run-here".to_owned()),
        Some(false) => command.push("--no-run-here".to_owned()),
        None => {}
    }
    append_extra(&mut command, args);
    run_hcom(command, cwd).await
}

pub async fn execute_hcom_resume(target: String, args: Vec<String>, cwd: &Path) -> ExecutionResult {
    let mut command = vec!["resume".to_owned(), target];
    append_extra(&mut command, args);
    run_hcom(command, cwd).await
}

pub async fn execute_hcom_fork(target: String, args: Vec<String>, cwd: &Path) -> ExecutionResult {
    let mut command = vec!["fork".to_owned(), target];
    append_extra(&mut command, args);
    run_hcom(command, cwd).await
}

pub async fn execute_hcom_kill(targets: Vec<String>, cwd: &Path) -> ExecutionResult {
    let mut command = vec!["kill".to_owned()];
    append_extra(&mut command, targets);
    run_hcom(command, cwd).await
}

pub async fn execute_hcom_relay(args: Vec<String>, cwd: &Path) -> ExecutionResult {
    let mut command = vec!["relay".to_owned()];
    append_extra(&mut command, args);
    run_hcom(command, cwd).await
}

pub async fn execute_hcom_run(
    script: Option<String>,
    args: Vec<String>,
    cwd: &Path,
) -> ExecutionResult {
    let mut command = vec!["run".to_owned()];
    if let Some(script) = script
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
    {
        command.push(script);
    }
    append_extra(&mut command, args);
    run_hcom(command, cwd).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_target_prefixes_missing_at_normal() {
        assert_eq!(normalize_target("luna").as_deref(), Some("@luna"));
        assert_eq!(normalize_target("@luna").as_deref(), Some("@luna"));
        assert_eq!(normalize_target("  ").as_deref(), None);
    }

    #[test]
    fn hcom_tool_name_match_is_case_insensitive_normal() {
        assert!(is_hcom_tool_name("HcomSend"));
        assert!(is_hcom_tool_name("hcomsend"));
        assert!(!is_hcom_tool_name("SendMessage"));
    }
}
