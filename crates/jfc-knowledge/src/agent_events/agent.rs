use crate::{KnowledgeStore, Result, record};

use super::rows::{
    AgentEventRow, AgentMailboxRow, AgentSessionRow, agent_event_from, agent_mailbox_from,
    agent_session_from,
};

impl KnowledgeStore {
    pub async fn upsert_agent_session(&self, row: &AgentSessionRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO agent_sessions \
             (id, parent_session_id, role, model, status, budget_tokens, task_id, team_id, \
              created_at_ms, updated_at_ms) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) \
             ON CONFLICT(id) DO UPDATE SET \
                parent_session_id=excluded.parent_session_id, role=excluded.role, \
                model=excluded.model, status=excluded.status, budget_tokens=excluded.budget_tokens, \
                task_id=excluded.task_id, team_id=excluded.team_id, updated_at_ms=excluded.updated_at_ms"
        )
            .bind(&row.id)
            .bind(&row.parent_session_id)
            .bind(&row.role)
            .bind(&row.model)
            .bind(&row.status)
            .bind(row.budget_tokens)
            .bind(&row.task_id)
            .bind(&row.team_id)
            .bind(row.created_at_ms)
            .bind(row.updated_at_ms)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_agent_session(&self, id: &str) -> Result<Option<AgentSessionRow>> {
        let row = sqlx::query(
            "SELECT id, parent_session_id, role, model, status, budget_tokens, task_id, \
                    team_id, created_at_ms, updated_at_ms \
             FROM agent_sessions WHERE id = ?1"
        )
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| agent_session_from(&r)).transpose()
    }

    pub async fn record_agent_event(&self, row: &AgentEventRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO agent_events \
             (id, session_id, from_agent, to_agent, kind, content, turn_id, causal_parent_id, \
              created_at_ms) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)"
        )
            .bind(&row.id)
            .bind(&row.session_id)
            .bind(&row.from_agent)
            .bind(&row.to_agent)
            .bind(&row.kind)
            .bind(&row.content)
            .bind(&row.turn_id)
            .bind(&row.causal_parent_id)
            .bind(row.created_at_ms)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_agent_events(&self, session_id: &str, limit: usize) -> Result<Vec<AgentEventRow>> {
        let rows = sqlx::query(
            "SELECT id, session_id, from_agent, to_agent, kind, content, turn_id, \
                    causal_parent_id, created_at_ms \
             FROM agent_events WHERE session_id = ?1 ORDER BY created_at_ms ASC LIMIT ?2"
        )
            .bind(session_id)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?;
        let mut out = Vec::new();
        for row in rows {
            out.push(agent_event_from(&row)?);
        }
        Ok(out)
    }

    pub async fn enqueue_agent_mailbox(&self, row: &AgentMailboxRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO agent_mailbox \
             (id, to_agent, from_agent, thread_id, task_id, priority, content, read_at_ms, \
              summarized_at_ms, created_at_ms) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)"
        )
            .bind(&row.id)
            .bind(&row.to_agent)
            .bind(&row.from_agent)
            .bind(&row.thread_id)
            .bind(&row.task_id)
            .bind(row.priority)
            .bind(&row.content)
            .bind(row.read_at_ms)
            .bind(row.summarized_at_ms)
            .bind(row.created_at_ms)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_agent_mailbox(
        &self,
        to_agent: &str,
        unread_only: bool,
    ) -> Result<Vec<AgentMailboxRow>> {
        let sql = if unread_only {
            "SELECT id, to_agent, from_agent, thread_id, task_id, priority, content, \
                    read_at_ms, summarized_at_ms, created_at_ms \
             FROM agent_mailbox WHERE to_agent = ?1 AND read_at_ms IS NULL \
             ORDER BY priority DESC, created_at_ms ASC"
        } else {
            "SELECT id, to_agent, from_agent, thread_id, task_id, priority, content, \
                    read_at_ms, summarized_at_ms, created_at_ms \
             FROM agent_mailbox WHERE to_agent = ?1 \
             ORDER BY priority DESC, created_at_ms ASC"
        };
        let rows = sqlx::query(sql)
            .bind(to_agent)
            .fetch_all(&self.pool)
            .await?;
        let mut out = Vec::new();
        for row in rows {
            out.push(agent_mailbox_from(&row)?);
        }
        Ok(out)
    }

    pub async fn mark_agent_mailbox_read(&self, id: &str) -> Result<usize> {
        let result = sqlx::query(
            "UPDATE agent_mailbox SET read_at_ms = ?2 WHERE id = ?1 AND read_at_ms IS NULL"
        )
            .bind(id)
            .bind(record::now_ms())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() as usize)
    }

    pub async fn mark_all_agent_mailbox_read(&self, to_agent: &str) -> Result<usize> {
        let result = sqlx::query(
            "UPDATE agent_mailbox SET read_at_ms = ?2 WHERE to_agent = ?1 AND read_at_ms IS NULL"
        )
            .bind(to_agent)
            .bind(record::now_ms())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() as usize)
    }

    pub async fn clear_agent_mailbox(&self, to_agent: &str) -> Result<usize> {
        let result = sqlx::query("DELETE FROM agent_mailbox WHERE to_agent = ?1")
            .bind(to_agent)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() as usize)
    }
}
