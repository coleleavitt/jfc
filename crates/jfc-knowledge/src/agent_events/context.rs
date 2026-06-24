use crate::{KnowledgeStore, Result};

use super::rows::{ContextEventRow, context_event_from};

impl KnowledgeStore {
    pub async fn record_context_event(&self, row: &ContextEventRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO context_events \
             (id, session_id, turn_id, agent_id, subagent_id, model, input_tokens, output_tokens, \
              thinking_tokens, cache_read_tokens, cache_write_tokens, context_limit, bust_cause, \
              drop_cause, payload, created_at_ms) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16) \
             ON CONFLICT(id) DO UPDATE SET \
                input_tokens=excluded.input_tokens, output_tokens=excluded.output_tokens, \
                thinking_tokens=excluded.thinking_tokens, cache_read_tokens=excluded.cache_read_tokens, \
                cache_write_tokens=excluded.cache_write_tokens, context_limit=excluded.context_limit, \
                bust_cause=excluded.bust_cause, drop_cause=excluded.drop_cause, \
                payload=excluded.payload, created_at_ms=excluded.created_at_ms"
        )
            .bind(&row.id)
            .bind(&row.session_id)
            .bind(&row.turn_id)
            .bind(&row.agent_id)
            .bind(&row.subagent_id)
            .bind(&row.model)
            .bind(row.input_tokens)
            .bind(row.output_tokens)
            .bind(row.thinking_tokens)
            .bind(row.cache_read_tokens)
            .bind(row.cache_write_tokens)
            .bind(row.context_limit)
            .bind(&row.bust_cause)
            .bind(&row.drop_cause)
            .bind(&row.payload)
            .bind(row.created_at_ms)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_context_events(
        &self,
        session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ContextEventRow>> {
        let limit = limit as i64;
        let mut out = Vec::new();
        if let Some(session_id) = session_id {
            let rows = sqlx::query(
                "SELECT id, session_id, turn_id, agent_id, subagent_id, model, input_tokens, \
                        output_tokens, thinking_tokens, cache_read_tokens, cache_write_tokens, \
                        context_limit, bust_cause, drop_cause, payload, created_at_ms \
                 FROM context_events WHERE session_id = ?1 ORDER BY created_at_ms ASC LIMIT ?2"
            )
                .bind(session_id)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?;
            for row in rows {
                out.push(context_event_from(&row)?);
            }
        } else {
            let rows = sqlx::query(
                "SELECT id, session_id, turn_id, agent_id, subagent_id, model, input_tokens, \
                        output_tokens, thinking_tokens, cache_read_tokens, cache_write_tokens, \
                        context_limit, bust_cause, drop_cause, payload, created_at_ms \
                 FROM context_events ORDER BY created_at_ms DESC LIMIT ?1"
            )
                .bind(limit)
                .fetch_all(&self.pool)
                .await?;
            for row in rows {
                out.push(context_event_from(&row)?);
            }
            out.reverse();
        }
        Ok(out)
    }
}

pub(crate) async fn clear_derived_context_events(
    tx: &mut sqlx::sqlite::SqliteConnection,
    session_id: &str,
) -> Result<usize> {
    let result = sqlx::query(
        "DELETE FROM context_events WHERE session_id = ?1 AND agent_id IS NULL AND subagent_id IS NULL"
    )
        .bind(session_id)
        .execute(&mut *tx)
        .await?;
    Ok(result.rows_affected() as usize)
}

pub(crate) async fn insert_context_events_from_messages(
    tx: &mut sqlx::sqlite::SqliteConnection,
    row: &crate::SessionRow,
    messages: &[crate::SessionMessage],
    created_at_ms: i64,
) -> Result<()> {
    for message in messages {
        if message.role != "assistant" {
            continue;
        }
        let Some(meta) = message.meta.as_deref().and_then(parse_json) else {
            continue;
        };
        let Some(usage) = usage_from_meta(&meta) else {
            continue;
        };
        if usage.total_tokens() == 0 {
            continue;
        }
        let model = meta
            .get("model_name")
            .and_then(serde_json::Value::as_str)
            .or(row.model.as_deref())
            .unwrap_or("unknown")
            .to_owned();
        let turn_id = format!("{}:{}", row.id, message.seq);
        let payload = context_payload(message.seq, &meta);
        sqlx::query(
            "INSERT INTO context_events \
             (id, session_id, turn_id, agent_id, subagent_id, model, input_tokens, output_tokens, \
              thinking_tokens, cache_read_tokens, cache_write_tokens, context_limit, bust_cause, \
              drop_cause, payload, created_at_ms) \
             VALUES (?1,?2,?3,NULL,NULL,?4,?5,?6,?7,?8,?9,NULL,?10,?11,?12,?13)"
        )
            .bind(deterministic_context_id(&row.id, message.seq))
            .bind(&row.id)
            .bind(&turn_id)
            .bind(&model)
            .bind(usage.input_tokens)
            .bind(usage.output_tokens)
            .bind(usage.thinking_tokens)
            .bind(usage.cache_read_tokens)
            .bind(usage.cache_write_tokens)
            .bind(cache_bust_cause(&usage))
            .bind(Option::<String>::None)
            .bind(&payload)
            .bind(created_at_ms)
            .execute(&mut *tx)
            .await?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Default)]
struct UsageSnapshot {
    input_tokens: i64,
    output_tokens: i64,
    thinking_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
}

impl UsageSnapshot {
    fn total_tokens(self) -> i64 {
        self.input_tokens
            + self.output_tokens
            + self.thinking_tokens
            + self.cache_read_tokens
            + self.cache_write_tokens
    }
}

fn usage_from_meta(meta: &serde_json::Value) -> Option<UsageSnapshot> {
    let usage = meta.get("usage")?;
    Some(UsageSnapshot {
        input_tokens: usage_i64(usage, "input_tokens"),
        output_tokens: usage_i64(usage, "output_tokens"),
        thinking_tokens: usage_i64(usage, "thinking_tokens"),
        cache_read_tokens: usage_i64(usage, "cache_read_tokens"),
        cache_write_tokens: usage_i64(usage, "cache_write_tokens"),
    })
}

fn usage_i64(usage: &serde_json::Value, key: &str) -> i64 {
    usage
        .get(key)
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0)
}

fn cache_bust_cause(usage: &UsageSnapshot) -> Option<String> {
    (usage.input_tokens > 10_000 && usage.cache_read_tokens == 0 && usage.cache_write_tokens == 0)
        .then(|| "cache_miss".to_owned())
}

fn context_payload(seq: i64, meta: &serde_json::Value) -> String {
    serde_json::json!({
        "source": "session_transcript",
        "message_seq": seq,
        "message_created_at": meta.get("created_at").cloned(),
    })
    .to_string()
}

fn parse_json(raw: &str) -> Option<serde_json::Value> {
    serde_json::from_str(raw).ok()
}

fn deterministic_context_id(session_id: &str, seq: i64) -> String {
    uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_OID,
        format!("context:{session_id}:{seq}").as_bytes(),
    )
    .simple()
    .to_string()
}
