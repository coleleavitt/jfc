//! Hand-rolled localhost HTTP server for the token-audit dashboard.
//!
//! Mirrors the design preview server: `std::net` only, one detached thread per
//! connection, no async runtime, `Connection: close`. Serves three routes:
//! `GET /` (the embedded single-page UI), `GET /api/snapshot` (JSON), and
//! `GET /health`.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};

use crate::{DashboardHandle, trace};

const INDEX_HTML: &str = include_str!("dashboard.html");

/// A detached dashboard server returned by [`spawn`].
#[derive(Debug, Clone)]
pub struct DashboardServer {
    pub local_addr: SocketAddr,
}

/// Bind `addr` (e.g. `127.0.0.1:0` for an ephemeral port), serve the dashboard
/// from a background thread, and return the bound address. The server reads the
/// shared `handle` on each request — it never blocks the engine.
pub fn spawn(handle: DashboardHandle, addr: &str) -> std::io::Result<DashboardServer> {
    let _linkscope_spawn = linkscope::phase("dashboard.server.spawn");
    trace::record_server_bind("dashboard.server.spawn.start", addr.len());
    let listener = TcpListener::bind(addr)?;
    let local_addr = listener.local_addr()?;
    trace::record_server_bind(
        "dashboard.server.spawn.result",
        local_addr.to_string().len(),
    );
    tracing::info!(target: "jfc::dashboard", %local_addr, "token dashboard listening");
    std::thread::Builder::new()
        .name("jfc-dashboard".into())
        .spawn(move || serve_listener(&listener, &handle))?;
    Ok(DashboardServer { local_addr })
}

fn serve_listener(listener: &TcpListener, handle: &DashboardHandle) {
    let _linkscope_serve = linkscope::phase("dashboard.server.serve_listener");
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                linkscope::record_items("dashboard.server.accept.ok", 1);
                let handle = handle.clone();
                if let Err(error) = std::thread::Builder::new()
                    .name("jfc-dashboard-conn".into())
                    .spawn(move || {
                        if let Err(error) = handle_connection(stream, &handle) {
                            tracing::debug!(target: "jfc::dashboard", error = %error, "connection error");
                        }
                    })
                {
                    linkscope::record_items("dashboard.server.spawn_conn.error", 1);
                    tracing::debug!(target: "jfc::dashboard", error = %error, "spawn error");
                }
            }
            Err(error) => {
                linkscope::record_items("dashboard.server.accept.error", 1);
                tracing::debug!(target: "jfc::dashboard", error = %error, "accept error");
            }
        }
    }
}

fn handle_connection(mut stream: TcpStream, handle: &DashboardHandle) -> std::io::Result<()> {
    let _linkscope_connection = linkscope::phase("dashboard.server.handle_connection");
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        linkscope::event_fields(
            "dashboard.server.request",
            [linkscope::TraceField::text("status", "empty")],
        );
        return Ok(());
    }
    // Drain the rest of the headers; static GETs need nothing from them.
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
    let head = method == "HEAD";
    let route = trace::route_label(target);
    trace::record_request(trace::RequestShape {
        method,
        route,
        target_bytes: target.len(),
        head,
    });
    if method != "GET" && !head {
        return respond(
            &mut stream,
            405,
            "text/plain; charset=utf-8",
            b"method not allowed",
            head,
        );
    }

    match route {
        "index" => respond(
            &mut stream,
            200,
            "text/html; charset=utf-8",
            INDEX_HTML.as_bytes(),
            head,
        ),
        "health" => respond(&mut stream, 200, "text/plain; charset=utf-8", b"ok", head),
        "snapshot" => {
            let body = snapshot_json(handle);
            respond(
                &mut stream,
                200,
                "application/json; charset=utf-8",
                &body,
                head,
            )
        }
        "not_found" => respond(
            &mut stream,
            404,
            "text/plain; charset=utf-8",
            b"not found",
            head,
        ),
        _ => unreachable!("route_label returns a closed set"),
    }
}

fn snapshot_json(handle: &DashboardHandle) -> Vec<u8> {
    let _linkscope_snapshot = linkscope::phase("dashboard.server.snapshot_json");
    let snapshot = match handle.lock() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    };
    trace::record_snapshot("dashboard.server.snapshot.shape", &snapshot);
    let body = serde_json::to_vec(&snapshot).unwrap_or_else(|_| b"{}".to_vec());
    linkscope::record_bytes(
        "dashboard.server.snapshot_json.bytes",
        u64::try_from(body.len()).unwrap_or(u64::MAX),
    );
    body
}

fn respond(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    head_only: bool,
) -> std::io::Result<()> {
    let _linkscope_respond = linkscope::phase("dashboard.server.respond");
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "OK",
    };
    trace::record_response(status, content_type, body.len(), head_only);
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
