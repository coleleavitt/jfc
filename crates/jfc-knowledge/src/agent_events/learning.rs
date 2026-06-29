use crate::{KnowledgeStore, Result};

use super::rows::{LearningEventRow, ToolRunLedgerRow, learning_event_from};

impl KnowledgeStore {
    pub async fn record_tool_run(&self, row: &ToolRunLedgerRow) -> Result<()> {
        let _linkscope_tool = linkscope::phase("knowledge.tool_run.record");
        linkscope::event_fields(
            "knowledge.tool_run.record",
            [
                linkscope::TraceField::text("id", row.id.clone()),
                linkscope::TraceField::text("kind", row.kind.clone()),
                linkscope::TraceField::text("status", row.status.clone()),
                linkscope::TraceField::count("background", u64::from(row.background)),
                linkscope::TraceField::count(
                    "duration_ms",
                    row.duration_ms
                        .and_then(|value| u64::try_from(value).ok())
                        .unwrap_or(0),
                ),
            ],
        );
        sqlx::query(
            "INSERT INTO tool_runs \
             (id, agent_id, session_id, runtime_id, kind, command, input_json, output_ref, status, \
              duration_ms, background, created_at_ms, updated_at_ms) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13) \
             ON CONFLICT(id) DO UPDATE SET \
                output_ref=excluded.output_ref, status=excluded.status, duration_ms=excluded.duration_ms, \
                background=excluded.background, updated_at_ms=excluded.updated_at_ms"
        )
            .bind(&row.id)
            .bind(&row.agent_id)
            .bind(&row.session_id)
            .bind(&row.runtime_id)
            .bind(&row.kind)
            .bind(&row.command)
            .bind(&row.input_json)
            .bind(&row.output_ref)
            .bind(&row.status)
            .bind(row.duration_ms)
            .bind(row.background)
            .bind(row.created_at_ms)
            .bind(row.updated_at_ms)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn record_learning_event(&self, row: &LearningEventRow) -> Result<()> {
        let _linkscope_learning = linkscope::phase("knowledge.learning_event.record");
        linkscope::event_fields(
            "knowledge.learning_event.record",
            [
                linkscope::TraceField::text("id", row.id.clone()),
                linkscope::TraceField::text("status", row.status.clone()),
                linkscope::TraceField::count(
                    "recurrence_count",
                    u64::try_from(row.recurrence_count).unwrap_or(0),
                ),
                linkscope::TraceField::bytes(
                    "candidate_rule_bytes",
                    u64::try_from(row.candidate_rule.len()).unwrap_or(u64::MAX),
                ),
            ],
        );
        sqlx::query(
            "INSERT INTO learning_events \
             (id, source_session_id, source_turn_id, source_tool_run_id, candidate_rule, status, \
              verifier_evidence, recurrence_count, created_at_ms, updated_at_ms) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) \
             ON CONFLICT(id) DO UPDATE SET \
                status=excluded.status, verifier_evidence=excluded.verifier_evidence, \
                recurrence_count=excluded.recurrence_count, updated_at_ms=excluded.updated_at_ms",
        )
        .bind(&row.id)
        .bind(&row.source_session_id)
        .bind(&row.source_turn_id)
        .bind(&row.source_tool_run_id)
        .bind(&row.candidate_rule)
        .bind(&row.status)
        .bind(&row.verifier_evidence)
        .bind(row.recurrence_count)
        .bind(row.created_at_ms)
        .bind(row.updated_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_learning_events(
        &self,
        status: Option<&str>,
        limit: usize,
    ) -> Result<Vec<LearningEventRow>> {
        let _linkscope_list = linkscope::phase("knowledge.learning_event.list");
        let limit = limit as i64;
        let mut out = Vec::new();
        if let Some(status) = status {
            let rows = sqlx::query(
                "SELECT id, source_session_id, source_turn_id, source_tool_run_id, candidate_rule, \
                        status, verifier_evidence, recurrence_count, created_at_ms, updated_at_ms \
                 FROM learning_events WHERE status = ?1 ORDER BY updated_at_ms DESC LIMIT ?2",
            )
            .bind(status)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
            for row in rows {
                out.push(learning_event_from(&row)?);
            }
        } else {
            let rows = sqlx::query(
                "SELECT id, source_session_id, source_turn_id, source_tool_run_id, candidate_rule, \
                        status, verifier_evidence, recurrence_count, created_at_ms, updated_at_ms \
                 FROM learning_events ORDER BY updated_at_ms DESC LIMIT ?1",
            )
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
            for row in rows {
                out.push(learning_event_from(&row)?);
            }
        }
        linkscope::event_fields(
            "knowledge.learning_event.list",
            [
                linkscope::TraceField::text("status", status.unwrap_or("*").to_owned()),
                linkscope::TraceField::count("limit", u64::try_from(limit).unwrap_or(0)),
                linkscope::TraceField::count("rows", u64::try_from(out.len()).unwrap_or(u64::MAX)),
            ],
        );
        Ok(out)
    }
}

pub(crate) async fn delete_session_scoped_rows(
    tx: &mut sqlx::sqlite::SqliteConnection,
    session_id: &str,
) -> Result<usize> {
    let _linkscope_delete = linkscope::phase("knowledge.agent_events.delete_session_scoped");
    let context = sqlx::query("DELETE FROM context_events WHERE session_id = ?1")
        .bind(session_id)
        .execute(&mut *tx)
        .await?
        .rows_affected() as usize;
    let agent = sqlx::query("DELETE FROM agent_events WHERE session_id = ?1")
        .bind(session_id)
        .execute(&mut *tx)
        .await?
        .rows_affected() as usize;
    let tools = sqlx::query("DELETE FROM tool_runs WHERE session_id = ?1")
        .bind(session_id)
        .execute(&mut *tx)
        .await?
        .rows_affected() as usize;
    let learning = sqlx::query("DELETE FROM learning_events WHERE source_session_id = ?1")
        .bind(session_id)
        .execute(&mut *tx)
        .await?
        .rows_affected() as usize;
    let total = context + agent + tools + learning;
    linkscope::event_fields(
        "knowledge.agent_events.delete_session_scoped",
        [
            linkscope::TraceField::text("session_id", session_id.to_owned()),
            linkscope::TraceField::count("context", u64::try_from(context).unwrap_or(u64::MAX)),
            linkscope::TraceField::count("agent", u64::try_from(agent).unwrap_or(u64::MAX)),
            linkscope::TraceField::count("tools", u64::try_from(tools).unwrap_or(u64::MAX)),
            linkscope::TraceField::count("learning", u64::try_from(learning).unwrap_or(u64::MAX)),
            linkscope::TraceField::count("total", u64::try_from(total).unwrap_or(u64::MAX)),
        ],
    );
    Ok(total)
}
