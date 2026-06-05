// Expected nodes: 2 Functions (ping, pong)
// Expected edges: ping->pong (Calls), pong->ping (Calls) — CYCLE
// Used to test cycle detection in traversal

pub fn ping(n: u32) -> u32 {
    if n == 0 { 0 } else { pong(n - 1) }
}

pub fn pong(n: u32) -> u32 {
    if n == 0 { 1 } else { ping(n - 1) }
}
