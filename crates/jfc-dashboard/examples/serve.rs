//! Long-running dashboard server with rich mock data — for visual design /
//! Playwright screenshots. `cargo run -p jfc-dashboard --example serve`, then
//! open http://127.0.0.1:4330.

use std::thread;
use std::time::Duration;

use jfc_context::{ContextAccount, ContextContributor, ContributorId};
use jfc_dashboard::{
    CompartmentSummary, DashboardSnapshot, ModelUsageRow, ProfilePhase, TimelineSample,
};

fn contributor(id: &str, label: &str, tokens: u64) -> ContextContributor {
    ContextContributor::new(ContributorId::new(id).expect("valid id"), label).with_tokens(tokens)
}

#[allow(clippy::too_many_arguments)]
fn sample(
    ts: u64,
    prompt: &str,
    input: u64,
    output: u64,
    cache: u64,
    cost: f64,
    used: u64,
    rsi_sections: u64,
    rsi_rules: u64,
    flags: &[&str],
) -> TimelineSample {
    TimelineSample {
        ts_unix: ts,
        model: "claude-opus-4-8".into(),
        prompt: Some(prompt.into()),
        input_delta: input,
        output_delta: output,
        cache_read_delta: cache,
        cache_hit_pct: if input > 0 {
            (cache as f64 / input as f64 * 100.0).min(100.0)
        } else {
            0.0
        },
        cost_delta_usd: cost,
        context_used_tokens: used,
        context_window_tokens: 1_000_000,
        rsi_prompt_sections: rsi_sections,
        rsi_tool_visibility_rules: rsi_rules,
        flags: flags.iter().map(|f| (*f).to_owned()).collect(),
        ..Default::default()
    }
}

fn main() {
    let account = ContextAccount::new(vec![
        contributor("builtin.system", "System", 9_726),
        contributor("builtin.docs", "Docs", 3_560),
        contributor("builtin.compartments", "Compartments", 12_000),
        contributor("builtin.memories", "Memories", 1_940),
        contributor("builtin.conversation", "Conversation", 48_300),
        contributor("builtin.tool-calls", "Tool Calls", 16_036),
        contributor("builtin.tool-defs", "Tool Defs", 11_525),
    ]);

    let snapshot = DashboardSnapshot {
        generated_at_unix: 1_782_700_000,
        session_id: Some("ses_20260628_preview".into()),
        model: Some("claude-opus-4-8".into()),
        context_window_tokens: 1_000_000,
        context_used_tokens: 103_087,
        account,
        compartments: CompartmentSummary {
            count: 3,
            recent: 1,
            warm: 1,
            cold: 1,
            archived: 0,
            total_tokens: 12_000,
        },
        usage_by_model: vec![ModelUsageRow {
            model: "claude-opus-4-8".into(),
            input_tokens: 232,
            output_tokens: 59_881,
            cache_read_tokens: 14_600_000,
            cache_write_tokens: 2_400_000,
            thinking_tokens: 18_400,
            cache_hit_pct: 100.0,
            cost_usd: 71.21,
        }],
        total_cost_usd: 71.21,
        rsi_prompt_sections: 4,
        rsi_tool_visibility_rules: 2,
        // RSI grows 1 → 4 sections across the session — the "improving" story.
        timeline: vec![
            sample(1_782_699_100, "wire the compartments in", 2_100, 1_800, 0, 0.30, 18_400, 1, 0, &[]),
            sample(1_782_699_200, "wire the compartments in", 28_400, 2_300, 24_000, 0.55, 47_657, 1, 1, &["input_spike"]),
            sample(1_782_699_350, "add the token timeline", 31_200, 3_900, 152_551, 0.42, 61_300, 2, 1, &[]),
            sample(1_782_699_500, "add the token timeline", 9_800, 1_200, 9_800, 0.18, 64_100, 2, 2, &["cache_hit_drop"]),
            sample(1_782_699_700, "extract rsi-rs", 42_700, 4_100, 12_000, 1.10, 92_000, 3, 2, &["input_spike", "cost_spike", "context_near_window"]),
            sample(1_782_699_900, "extract rsi-rs", 15_300, 2_900, 14_900, 0.34, 103_087, 4, 2, &[]),
        ],
        profile: vec![
            ProfilePhase { name: "turn.submit".into(), ms: 48_201.4, spans: 6, ..Default::default() },
            ProfilePhase { name: "turn.compact".into(), ms: 3_127.9, spans: 1, ..Default::default() },
            ProfilePhase { name: "stream_context_budget".into(), ms: 12.6, spans: 6, ..Default::default() },
        ],
        ..Default::default()
    };

    let handle = jfc_dashboard::new_handle();
    jfc_dashboard::publish(&handle, snapshot);
    let server = jfc_dashboard::spawn(handle, "127.0.0.1:4330").expect("spawn dashboard");
    println!("PREVIEW http://{}", server.local_addr);
    loop {
        thread::sleep(Duration::from_secs(3600));
    }
}
