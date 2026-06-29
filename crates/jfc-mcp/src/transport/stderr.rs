use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, BufReader};

use super::StderrRing;

const DEFAULT_STDERR_RING_CAPACITY: usize = 200;

pub(super) fn new_ring() -> StderrRing {
    Arc::new(tokio::sync::Mutex::new(
        std::collections::VecDeque::with_capacity(DEFAULT_STDERR_RING_CAPACITY),
    ))
}

pub(super) fn empty_ring() -> StderrRing {
    Arc::new(tokio::sync::Mutex::new(std::collections::VecDeque::new()))
}

pub(super) fn spawn_stderr_drain(
    server_name: String,
    stderr: tokio::process::ChildStderr,
    ring: StderrRing,
) {
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            if line.trim().is_empty() {
                continue;
            }
            linkscope::record_items("mcp.stderr.line", 1);
            linkscope::record_bytes("mcp.stderr.line", len_to_u64(line.len()));
            tracing::debug!(
                target: "jfc::mcp",
                server = %server_name,
                stderr = %line,
                "mcp stderr"
            );
            let mut guard = ring.lock().await;
            if guard.len() == DEFAULT_STDERR_RING_CAPACITY {
                guard.pop_front();
                linkscope::record_items("mcp.stderr.evicted", 1);
            }
            guard.push_back(line);
        }
    });
}

fn len_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
