use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};

fn main() {
    let handle = jfc_dashboard::new_handle();
    let snapshot = jfc_dashboard::DashboardSnapshot {
        model: Some("claude-opus-4-8".to_owned()),
        total_cost_usd: 1.23,
        context_window_tokens: 1_000_000,
        context_used_tokens: 226_374,
        usage_by_model: vec![jfc_dashboard::ModelUsageRow {
            model: "claude-opus-4-8".to_owned(),
            input_tokens: 232,
            output_tokens: 59_881,
            cache_read_tokens: 14_600_000,
            cache_hit_pct: 100.0,
            cost_usd: 1.23,
            ..Default::default()
        }],
        timeline: vec![jfc_dashboard::TimelineSample {
            ts_unix: 1_782_600_000,
            model: "claude-opus-4-8".to_owned(),
            prompt: Some("add a timeline to the dashboard".to_owned()),
            input_delta: 12,
            output_delta: 2_293,
            cache_read_delta: 152_551,
            cost_delta_usd: 0.42,
            context_used_tokens: 47_657,
            context_window_tokens: 1_000_000,
            cache_hit_pct: 100.0,
            flags: vec!["input_spike".to_owned()],
            ..Default::default()
        }],
        profile: vec![jfc_dashboard::ProfilePhase {
            name: "turn.submit".to_owned(),
            ms: 1234.5,
            spans: 7,
            ..Default::default()
        }],
        ..Default::default()
    };
    jfc_dashboard::publish(&handle, snapshot);

    let server = jfc_dashboard::spawn(handle, "127.0.0.1:0").expect("spawn server");
    let addr = server.local_addr;

    let json = http_get(addr, "/api/snapshot");
    assert!(
        json.contains("\"total_cost_usd\":1.23"),
        "snapshot json: {json}"
    );
    assert!(json.contains("claude-opus-4-8"), "model in json");
    assert!(json.contains("\"cache_hit_pct\":100"), "usage row in json");
    assert!(json.contains("\"timeline\""), "timeline array in json");
    assert!(
        json.contains("add a timeline to the dashboard"),
        "per-prompt text in json"
    );
    assert!(json.contains("input_spike"), "anomaly flag in json");
    assert!(
        json.contains("turn.submit"),
        "linkscope profile phase in json"
    );

    let html = http_get(addr, "/");
    assert!(html.contains("token audit"), "index html title");
    assert!(html.contains("/api/snapshot"), "index polls snapshot");
    assert!(html.contains("renderTimeline"), "index renders timeline");
    assert!(html.contains("Prompts"), "index has per-prompt panel");

    let health = http_get(addr, "/health");
    assert!(health.contains("ok"), "health");

    let missing = http_get(addr, "/nope");
    assert!(missing.contains("404"), "404 path");

    println!("SMOKE OK · http://{addr} · routes /, /api/snapshot, /health, 404 all good");
}

fn http_get(addr: SocketAddr, path: &str) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect");
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"
    )
    .expect("write");
    let mut buf = String::new();
    stream.read_to_string(&mut buf).expect("read");
    buf
}
