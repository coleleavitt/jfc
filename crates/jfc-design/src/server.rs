//! Dependency-light localhost preview server.
//!
//! Serves a project directory's files over HTTP so HTML designs render with their
//! relative asset paths intact (the way the design preview pane needs). Hand-rolled
//! on `std::net` — one thread per connection, no async runtime — so it adds no heavy
//! dependencies and is safe to run alongside the TUI.
//!
//! This is the static + read-only-JSON slice of the design server. The interactive
//! browser-host endpoints (eval-js, screenshot, gen-pptx, SSE project events) are the
//! server phase — see `docs/design-parity-roadmap.md`.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};

use crate::{capabilities, mime};

/// Start the preview server for `dir` on `addr` (e.g. `127.0.0.1:4321`). Blocks.
pub fn serve(dir: impl AsRef<Path>, addr: &str) -> std::io::Result<()> {
    let root = dir
        .as_ref()
        .canonicalize()
        .unwrap_or_else(|_| dir.as_ref().to_path_buf());
    let listener = TcpListener::bind(addr)?;
    let local = listener.local_addr()?;
    tracing::info!(target: "jfc::design", %local, root = %root.display(), "preview server listening");
    println!(
        "jfc-design preview: serving {} at http://{local}",
        root.display()
    );
    println!("  open http://{local}/<your-file>.html   (Ctrl-C to stop)");

    serve_listener(root, listener)
}

/// Handle requests on a detached background thread and return the bound address.
pub fn spawn(dir: impl AsRef<Path>, addr: &str) -> std::io::Result<PreviewServer> {
    let root = dir
        .as_ref()
        .canonicalize()
        .unwrap_or_else(|_| dir.as_ref().to_path_buf());
    let listener = TcpListener::bind(addr)?;
    let local_addr = listener.local_addr()?;
    let thread_root = root.clone();
    std::thread::spawn(move || {
        if let Err(e) = serve_listener(thread_root, listener) {
            tracing::warn!(target: "jfc::design", error = %e, "preview server stopped");
        }
    });
    Ok(PreviewServer { root, local_addr })
}

/// A detached preview server returned by [`spawn`].
#[derive(Debug, Clone)]
pub struct PreviewServer {
    pub root: PathBuf,
    pub local_addr: SocketAddr,
}

fn serve_listener(root: PathBuf, listener: TcpListener) -> std::io::Result<()> {
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let root = root.clone();
                std::thread::spawn(move || {
                    if let Err(e) = handle(s, &root) {
                        tracing::debug!(target: "jfc::design", error = %e, "connection error");
                    }
                });
            }
            Err(e) => tracing::debug!(target: "jfc::design", error = %e, "accept error"),
        }
    }
    Ok(())
}

fn handle(mut stream: TcpStream, root: &Path) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    // Drain headers (we don't need them for static GETs).
    let mut header = String::new();
    loop {
        header.clear();
        if reader.read_line(&mut header)? == 0 || header == "\r\n" || header == "\n" {
            break;
        }
    }

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("/");
    if method != "GET" && method != "HEAD" {
        return respond(
            &mut stream,
            405,
            "text/plain",
            b"method not allowed",
            method == "HEAD",
        );
    }

    let path_only = target.split('?').next().unwrap_or(target);
    let head = method == "HEAD";

    match path_only {
        "/__jfc/health" => return respond(&mut stream, 200, "text/plain", b"ok", head),
        "/__jfc/capabilities" => {
            let body = serde_json::to_vec_pretty(&capabilities::matrix())
                .unwrap_or_else(|_| b"[]".to_vec());
            return respond(&mut stream, 200, "application/json", &body, head);
        }
        "/__jfc/files" => {
            let files = list_files(root);
            let body = serde_json::to_vec_pretty(&files).unwrap_or_else(|_| b"[]".to_vec());
            return respond(&mut stream, 200, "application/json", &body, head);
        }
        _ => {}
    }

    match resolve(root, path_only) {
        Some(file) => {
            let bytes = std::fs::read(&file)?;
            let ct = mime::guess(&file);
            respond(&mut stream, 200, &ct, &bytes, head)
        }
        None => respond(&mut stream, 404, "text/plain", b"not found", head),
    }
}

/// Resolve a request path to an on-disk file inside `root` (no traversal escape).
fn resolve(root: &Path, req: &str) -> Option<PathBuf> {
    let decoded = percent_decode(req);
    let mut p = root.to_path_buf();
    for comp in Path::new(&decoded).components() {
        match comp {
            Component::Normal(c) => p.push(c),
            Component::CurDir | Component::RootDir => {}
            Component::ParentDir | Component::Prefix(_) => return None,
        }
    }
    if p.is_dir() {
        p.push("index.html");
    }
    // Final containment check.
    let canon = p.canonicalize().ok()?;
    let root_canon = root.canonicalize().ok()?;
    if !canon.starts_with(&root_canon) {
        return None;
    }
    canon.is_file().then_some(canon)
}

fn list_files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file()
            && let Ok(rel) = entry.path().strip_prefix(root)
        {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    out.sort();
    out
}

fn respond(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    head_only: bool,
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "OK",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    if !head_only {
        stream.write_all(body)?;
    }
    stream.flush()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_blocks_traversal_normal() {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("jfc_srv_{n}"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("index.html"), b"<h1>hi</h1>").unwrap();

        assert!(resolve(&root, "/").is_some());
        assert!(resolve(&root, "/index.html").is_some());
        assert!(resolve(&root, "/../../etc/passwd").is_none());
        assert!(resolve(&root, "/nope.html").is_none());
        std::fs::remove_dir_all(&root).ok();
    }
}
