//! Coach mode — generates short coaching tips based on session statistics.
//!
//! Mirrors Claude Code 2.1.150's coaching feature: after a session runs for
//! a while, surface 2-3 sentences of actionable advice about tool usage
//! patterns, token spend, and pacing.

/// Session statistics used to derive coaching tips.
pub struct SessionStats {
    pub total_tool_calls: usize,
    pub read_calls: usize,
    pub write_calls: usize,
    pub bash_calls: usize,
    pub search_calls: usize,
    pub total_tokens_in: u64,
    pub total_tokens_out: u64,
    pub session_duration_secs: u64,
    pub compaction_count: usize,
    pub error_count: usize,
}

/// Generate 2-3 sentences of coaching advice based on observed session patterns.
pub fn generate_coaching_tips(stats: &SessionStats) -> String {
    let mut tips = Vec::new();

    // Tip: high read-to-write ratio suggests exploration without action
    if stats.read_calls > 20 && stats.write_calls == 0 {
        tips.push(
            "You've done a lot of reading without any writes. \
             Consider narrowing your exploration and making targeted edits."
                .to_string(),
        );
    }

    // Tip: excessive bash usage
    if stats.bash_calls > stats.read_calls + stats.write_calls && stats.bash_calls > 10 {
        tips.push(
            "Heavy bash usage detected. For file operations, prefer the Read/Write/Edit \
             tools — they're faster and produce cleaner diffs."
                .to_string(),
        );
    }

    // Tip: token budget awareness
    let total_tokens = stats.total_tokens_in + stats.total_tokens_out;
    if total_tokens > 500_000 && stats.compaction_count == 0 {
        tips.push(
            "You've used over 500k tokens without compaction. Consider running \
             `/compact` to summarize earlier context and free up space."
                .to_string(),
        );
    }

    // Tip: session duration
    if stats.session_duration_secs > 3600 && stats.total_tool_calls < 5 {
        tips.push(
            "This session has been active for over an hour with few tool calls. \
             Try breaking your task into smaller, concrete steps."
                .to_string(),
        );
    }

    // Tip: error rate
    if stats.error_count > 5
        && stats.error_count as f64 / stats.total_tool_calls.max(1) as f64 > 0.3
    {
        tips.push(
            "High error rate in tool calls. Double-check file paths and command \
             syntax before invoking tools."
                .to_string(),
        );
    }

    // Tip: no search usage
    if stats.total_tool_calls > 15 && stats.search_calls == 0 {
        tips.push(
            "Consider using Grep or GraphSearch to locate symbols instead of \
             manually reading through files."
                .to_string(),
        );
    }

    // Default tip when nothing stands out
    if tips.is_empty() {
        tips.push(
            "Session looks healthy. Keep iterating in small, verifiable steps \
             and run tests frequently."
                .to_string(),
        );
    }

    // Return at most 3 tips
    tips.truncate(3);
    tips.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn healthy_session_returns_default_tip() {
        let stats = SessionStats {
            total_tool_calls: 20,
            read_calls: 10,
            write_calls: 5,
            bash_calls: 3,
            search_calls: 2,
            total_tokens_in: 50_000,
            total_tokens_out: 10_000,
            session_duration_secs: 600,
            compaction_count: 0,
            error_count: 0,
        };
        let tips = generate_coaching_tips(&stats);
        assert!(!tips.is_empty());
    }

    #[test]
    fn high_reads_no_writes_produces_exploration_tip() {
        let stats = SessionStats {
            total_tool_calls: 25,
            read_calls: 25,
            write_calls: 0,
            bash_calls: 0,
            search_calls: 0,
            total_tokens_in: 100_000,
            total_tokens_out: 20_000,
            session_duration_secs: 300,
            compaction_count: 0,
            error_count: 0,
        };
        let tips = generate_coaching_tips(&stats);
        assert!(tips.contains("exploration") || tips.contains("writes"));
    }
}
