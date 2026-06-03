//! Browser-host bridge for design artifacts.
//!
//! The heavy browser work lives in a Playwright Node subprocess so the Rust API
//! stays small and dependency-light. The host script is JSON over stdin/stdout.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{DesignError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserViewport {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EvalJsRequest {
    pub path: Option<String>,
    pub script: String,
    pub wait_ms: Option<u64>,
    pub timeout_ms: Option<u64>,
    pub viewport: Option<BrowserViewport>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScreenshotRequest {
    pub path: Option<String>,
    pub output: Option<String>,
    pub selector: Option<String>,
    pub full_page: Option<bool>,
    pub wait_ms: Option<u64>,
    pub timeout_ms: Option<u64>,
    pub viewport: Option<BrowserViewport>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MultiScreenshotRequest {
    pub path: Option<String>,
    pub output_dir: Option<String>,
    pub selector: Option<String>,
    pub max_items: Option<usize>,
    pub max_slides: Option<usize>,
    pub include_data: Option<bool>,
    pub item_wait_ms: Option<u64>,
    pub wait_ms: Option<u64>,
    pub timeout_ms: Option<u64>,
    pub viewport: Option<BrowserViewport>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GenPptxRequest {
    pub path: Option<String>,
    pub output: Option<String>,
    pub selector: Option<String>,
    pub mode: Option<String>,
    pub fallback: Option<bool>,
    pub max_slides: Option<usize>,
    pub title: Option<String>,
    pub slide_wait_ms: Option<u64>,
    pub wait_ms: Option<u64>,
    pub timeout_ms: Option<u64>,
    pub viewport: Option<BrowserViewport>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DirectEditInspectRequest {
    pub path: Option<String>,
    pub selector: Option<String>,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub wait_ms: Option<u64>,
    pub timeout_ms: Option<u64>,
    pub viewport: Option<BrowserViewport>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VerifyRequest {
    pub path: Option<String>,
    pub output: Option<String>,
    pub selector: Option<String>,
    pub max_screenshots: Option<usize>,
    pub include_data: Option<bool>,
    pub wait_ms: Option<u64>,
    pub timeout_ms: Option<u64>,
    pub font_timeout_ms: Option<u64>,
    pub viewport: Option<BrowserViewport>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PrintPdfRequest {
    pub path: Option<String>,
    pub output: Option<String>,
    pub landscape: Option<bool>,
    pub print_background: Option<bool>,
    pub wait_ms: Option<u64>,
    pub timeout_ms: Option<u64>,
    pub viewport: Option<BrowserViewport>,
}

#[derive(Debug, Serialize)]
struct HostEvalJsRequest {
    url: String,
    script: String,
    wait_ms: Option<u64>,
    timeout_ms: Option<u64>,
    viewport: Option<BrowserViewport>,
}

#[derive(Debug, Serialize)]
struct HostScreenshotRequest {
    url: String,
    output: Option<String>,
    selector: Option<String>,
    full_page: Option<bool>,
    wait_ms: Option<u64>,
    timeout_ms: Option<u64>,
    viewport: Option<BrowserViewport>,
}

#[derive(Debug, Serialize)]
struct HostMultiScreenshotRequest {
    url: String,
    output_dir: Option<String>,
    selector: Option<String>,
    max_items: Option<usize>,
    max_slides: Option<usize>,
    include_data: Option<bool>,
    item_wait_ms: Option<u64>,
    wait_ms: Option<u64>,
    timeout_ms: Option<u64>,
    viewport: Option<BrowserViewport>,
}

#[derive(Debug, Serialize)]
struct HostGenPptxRequest {
    url: String,
    output: String,
    selector: Option<String>,
    mode: Option<String>,
    fallback: Option<bool>,
    max_slides: Option<usize>,
    title: Option<String>,
    slide_wait_ms: Option<u64>,
    wait_ms: Option<u64>,
    timeout_ms: Option<u64>,
    viewport: Option<BrowserViewport>,
}

#[derive(Debug, Serialize)]
struct HostDirectEditInspectRequest {
    url: String,
    selector: Option<String>,
    x: Option<f64>,
    y: Option<f64>,
    wait_ms: Option<u64>,
    timeout_ms: Option<u64>,
    viewport: Option<BrowserViewport>,
}

#[derive(Debug, Serialize)]
struct HostVerifyRequest {
    url: String,
    output: Option<String>,
    selector: Option<String>,
    max_screenshots: Option<usize>,
    include_data: Option<bool>,
    wait_ms: Option<u64>,
    timeout_ms: Option<u64>,
    font_timeout_ms: Option<u64>,
    viewport: Option<BrowserViewport>,
}

#[derive(Debug, Serialize)]
struct HostPrintPdfRequest {
    url: String,
    output: String,
    landscape: Option<bool>,
    print_background: Option<bool>,
    wait_ms: Option<u64>,
    timeout_ms: Option<u64>,
    viewport: Option<BrowserViewport>,
}

pub async fn eval_js(req: EvalJsRequest, file: &Path) -> Result<Value> {
    let host = HostEvalJsRequest {
        url: file_url(file)?,
        script: req.script,
        wait_ms: req.wait_ms,
        timeout_ms: req.timeout_ms,
        viewport: req.viewport,
    };
    run_host("eval-js", host).await
}

pub async fn screenshot(
    req: ScreenshotRequest,
    file: &Path,
    output: Option<&Path>,
) -> Result<Value> {
    let host = HostScreenshotRequest {
        url: file_url(file)?,
        output: output.map(|p| p.display().to_string()),
        selector: req.selector,
        full_page: req.full_page,
        wait_ms: req.wait_ms,
        timeout_ms: req.timeout_ms,
        viewport: req.viewport,
    };
    run_host("screenshot", host).await
}

pub async fn multi_screenshot(
    req: MultiScreenshotRequest,
    file: &Path,
    output_dir: Option<&Path>,
) -> Result<Value> {
    let host = HostMultiScreenshotRequest {
        url: file_url(file)?,
        output_dir: output_dir.map(|p| p.display().to_string()),
        selector: req.selector,
        max_items: req.max_items,
        max_slides: req.max_slides,
        include_data: req.include_data,
        item_wait_ms: req.item_wait_ms,
        wait_ms: req.wait_ms,
        timeout_ms: req.timeout_ms,
        viewport: req.viewport,
    };
    run_host("multi-screenshot", host).await
}

pub async fn gen_pptx(req: GenPptxRequest, file: &Path, output: &Path) -> Result<Value> {
    let host = HostGenPptxRequest {
        url: file_url(file)?,
        output: output.display().to_string(),
        selector: req.selector,
        mode: req.mode,
        fallback: req.fallback,
        max_slides: req.max_slides,
        title: req.title,
        slide_wait_ms: req.slide_wait_ms,
        wait_ms: req.wait_ms,
        timeout_ms: req.timeout_ms,
        viewport: req.viewport,
    };
    run_host("gen-pptx", host).await
}

pub async fn direct_edit_inspect(req: DirectEditInspectRequest, file: &Path) -> Result<Value> {
    let host = HostDirectEditInspectRequest {
        url: file_url(file)?,
        selector: req.selector,
        x: req.x,
        y: req.y,
        wait_ms: req.wait_ms,
        timeout_ms: req.timeout_ms,
        viewport: req.viewport,
    };
    run_host("direct-edit-inspect", host).await
}

pub async fn verify(req: VerifyRequest, file: &Path, output: Option<&Path>) -> Result<Value> {
    let host = HostVerifyRequest {
        url: file_url(file)?,
        output: output.map(|p| p.display().to_string()),
        selector: req.selector,
        max_screenshots: req.max_screenshots,
        include_data: req.include_data,
        wait_ms: req.wait_ms,
        timeout_ms: req.timeout_ms,
        font_timeout_ms: req.font_timeout_ms,
        viewport: req.viewport,
    };
    run_host("verify", host).await
}

pub async fn print_pdf(req: PrintPdfRequest, file: &Path, output: &Path) -> Result<Value> {
    let host = HostPrintPdfRequest {
        url: file_url(file)?,
        output: output.display().to_string(),
        landscape: req.landscape,
        print_background: req.print_background,
        wait_ms: req.wait_ms,
        timeout_ms: req.timeout_ms,
        viewport: req.viewport,
    };
    run_host("print-pdf", host).await
}

async fn run_host<T>(command: &'static str, request: T) -> Result<Value>
where
    T: Serialize + Send + 'static,
{
    let script = browser_host_script()?;
    let payload = serde_json::to_vec(&request)?;
    tokio::task::spawn_blocking(move || run_host_blocking(command, script, payload))
        .await
        .map_err(|e| DesignError::Browser(format!("browser host join failed: {e}")))?
}

fn run_host_blocking(command: &str, script: PathBuf, payload: Vec<u8>) -> Result<Value> {
    let node = std::env::var("JFC_DESIGN_NODE").unwrap_or_else(|_| "node".to_owned());
    let mut child = Command::new(node)
        .arg(&script)
        .arg(command)
        .current_dir(script.parent().unwrap_or_else(|| Path::new(".")))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            DesignError::Browser(format!(
                "failed to start browser host script {}: {e}",
                script.display()
            ))
        })?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(&payload).map_err(|e| {
            DesignError::Browser(format!("failed to write browser host request: {e}"))
        })?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| DesignError::Browser(format!("browser host wait failed: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(DesignError::Browser(format!(
            "browser host {command} failed with status {}: {}{}{}",
            output.status,
            stderr.trim(),
            if stderr.trim().is_empty() || stdout.trim().is_empty() {
                ""
            } else {
                " / stdout: "
            },
            stdout.trim()
        )));
    }
    serde_json::from_slice(&output.stdout).map_err(DesignError::from)
}

fn browser_host_script() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("JFC_DESIGN_BROWSER_HOST") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(DesignError::Browser(format!(
            "JFC_DESIGN_BROWSER_HOST does not point to a file: {}",
            path.display()
        )));
    }

    let cwd = std::env::current_dir()
        .map_err(|e| DesignError::Browser(format!("failed to read current dir: {e}")))?;
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidates = [
        cwd.join("apps/design-web/scripts/browser-host.mjs"),
        crate_dir.join("../../apps/design-web/scripts/browser-host.mjs"),
    ];
    for candidate in candidates {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(DesignError::Browser(
        "browser host script not found; set JFC_DESIGN_BROWSER_HOST or run from the workspace root"
            .to_owned(),
    ))
}

pub fn file_url(path: &Path) -> Result<String> {
    let canonical = path
        .canonicalize()
        .map_err(|e| crate::io_err(path, e))?
        .to_string_lossy()
        .replace('\\', "/");
    Ok(format!("file://{}", percent_encode_path(&canonical)))
}

fn percent_encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' | b':' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_encodes_spaces_normal() {
        assert_eq!(percent_encode_path("/tmp/a b.html"), "/tmp/a%20b.html");
    }
}
