#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSessionRow {
    pub id: String,
    pub parent_session_id: Option<String>,
    pub role: String,
    pub model: Option<String>,
    pub status: String,
    pub budget_tokens: Option<i64>,
    pub task_id: Option<String>,
    pub team_id: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentEventRow {
    pub id: String,
    pub session_id: String,
    pub from_agent: Option<String>,
    pub to_agent: Option<String>,
    pub kind: String,
    pub content: String,
    pub turn_id: Option<String>,
    pub causal_parent_id: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentMailboxRow {
    pub id: String,
    pub to_agent: String,
    pub from_agent: Option<String>,
    pub thread_id: Option<String>,
    pub task_id: Option<String>,
    pub priority: i64,
    pub content: String,
    pub read_at_ms: Option<i64>,
    pub summarized_at_ms: Option<i64>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolRunLedgerRow {
    pub id: String,
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    pub runtime_id: Option<String>,
    pub kind: String,
    pub command: Option<String>,
    pub input_json: Option<String>,
    pub output_ref: Option<String>,
    pub status: String,
    pub duration_ms: Option<i64>,
    pub background: bool,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearningEventRow {
    pub id: String,
    pub source_session_id: Option<String>,
    pub source_turn_id: Option<String>,
    pub source_tool_run_id: Option<String>,
    pub candidate_rule: String,
    pub status: String,
    pub verifier_evidence: String,
    pub recurrence_count: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextEventRow {
    pub id: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub agent_id: Option<String>,
    pub subagent_id: Option<String>,
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub thinking_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub context_limit: Option<i64>,
    pub bust_cause: Option<String>,
    pub drop_cause: Option<String>,
    pub payload: String,
    pub created_at_ms: i64,
}

pub(super) fn agent_session_from(row: &sqlx::sqlite::SqliteRow) -> crate::Result<AgentSessionRow> {
    use sqlx::Row;
    Ok(AgentSessionRow {
        id: row.try_get(0)?,
        parent_session_id: row.try_get(1)?,
        role: row.try_get(2)?,
        model: row.try_get(3)?,
        status: row.try_get(4)?,
        budget_tokens: row.try_get(5)?,
        task_id: row.try_get(6)?,
        team_id: row.try_get(7)?,
        created_at_ms: row.try_get(8)?,
        updated_at_ms: row.try_get(9)?,
    })
}

pub(super) fn agent_event_from(row: &sqlx::sqlite::SqliteRow) -> crate::Result<AgentEventRow> {
    use sqlx::Row;
    Ok(AgentEventRow {
        id: row.try_get(0)?,
        session_id: row.try_get(1)?,
        from_agent: row.try_get(2)?,
        to_agent: row.try_get(3)?,
        kind: row.try_get(4)?,
        content: row.try_get(5)?,
        turn_id: row.try_get(6)?,
        causal_parent_id: row.try_get(7)?,
        created_at_ms: row.try_get(8)?,
    })
}

pub(super) fn agent_mailbox_from(row: &sqlx::sqlite::SqliteRow) -> crate::Result<AgentMailboxRow> {
    use sqlx::Row;
    Ok(AgentMailboxRow {
        id: row.try_get(0)?,
        to_agent: row.try_get(1)?,
        from_agent: row.try_get(2)?,
        thread_id: row.try_get(3)?,
        task_id: row.try_get(4)?,
        priority: row.try_get(5)?,
        content: row.try_get(6)?,
        read_at_ms: row.try_get(7)?,
        summarized_at_ms: row.try_get(8)?,
        created_at_ms: row.try_get(9)?,
    })
}

pub(super) fn learning_event_from(
    row: &sqlx::sqlite::SqliteRow,
) -> crate::Result<LearningEventRow> {
    use sqlx::Row;
    Ok(LearningEventRow {
        id: row.try_get(0)?,
        source_session_id: row.try_get(1)?,
        source_turn_id: row.try_get(2)?,
        source_tool_run_id: row.try_get(3)?,
        candidate_rule: row.try_get(4)?,
        status: row.try_get(5)?,
        verifier_evidence: row.try_get(6)?,
        recurrence_count: row.try_get(7)?,
        created_at_ms: row.try_get(8)?,
        updated_at_ms: row.try_get(9)?,
    })
}

pub(super) fn context_event_from(row: &sqlx::sqlite::SqliteRow) -> crate::Result<ContextEventRow> {
    use sqlx::Row;
    Ok(ContextEventRow {
        id: row.try_get(0)?,
        session_id: row.try_get(1)?,
        turn_id: row.try_get(2)?,
        agent_id: row.try_get(3)?,
        subagent_id: row.try_get(4)?,
        model: row.try_get(5)?,
        input_tokens: row.try_get(6)?,
        output_tokens: row.try_get(7)?,
        thinking_tokens: row.try_get(8)?,
        cache_read_tokens: row.try_get(9)?,
        cache_write_tokens: row.try_get(10)?,
        context_limit: row.try_get(11)?,
        bust_cause: row.try_get(12)?,
        drop_cause: row.try_get(13)?,
        payload: row.try_get(14)?,
        created_at_ms: row.try_get(15)?,
    })
}

// collect_rows is no longer needed with sqlx — callers just use fetch_all/fetch_one
// and map rows directly using the helper functions above
