use crate::{KnowledgeStore, Result};

use super::rows::{LearningEventRow, ToolRunLedgerRow, learning_event_from};

impl KnowledgeStore {
    pub async fn record_tool_run(&self, row: &ToolRunLedgerRow) -> Result<()> {
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
        sqlx::query(
            "INSERT INTO learning_events \
             (id, source_session_id, source_turn_id, source_tool_run_id, candidate_rule, status, \
              verifier_evidence, recurrence_count, created_at_ms, updated_at_ms) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) \
             ON CONFLICT(id) DO UPDATE SET \
                status=excluded.status, verifier_evidence=excluded.verifier_evidence, \
                recurrence_count=excluded.recurrence_count, updated_at_ms=excluded.updated_at_ms"
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
        let limit = limit as i64;
        let mut out = Vec::new();
        if let Some(status) = status {
            let rows = sqlx::query(
                "SELECT id, source_session_id, source_turn_id, source_tool_run_id, candidate_rule, \
                        status, verifier_evidence, recurrence_count, created_at_ms, updated_at_ms \
                 FROM learning_events WHERE status = ?1 ORDER BY updated_at_ms DESC LIMIT ?2"
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
                 FROM learning_events ORDER BY updated_at_ms DESC LIMIT ?1"
            )
                .bind(limit)
                .fetch_all(&self.pool)
                .await?;
            for row in rows {
                out.push(learning_event_from(&row)?);
            }
        }
        Ok(out)
    }
}

pub(crate) async fn delete_session_scoped_rows(tx: &mut sqlx::sqlite::SqliteConnection, session_id: &str) -> Result<usize> {
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
    Ok(context + agent + tools + learning)
}
