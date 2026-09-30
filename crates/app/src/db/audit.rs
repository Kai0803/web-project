//! 變更稽核。
//!
//! 稽核寫入必須與被稽核的變更落在**同一交易**內，否則會出現
//! 有紀錄但無變更，或有變更但無紀錄。這也是 `db` 函式收 executor
//! 而非 pool 的其中一個理由。

use acms_domain::UserId;
use sqlx::PgExecutor;

/// `changes` 只記錄有變動欄位的前後值，不存整體快照：
///
/// ```json
/// { "status": {"from": "pending", "to": "approved"} }
/// ```
///
/// 存整體快照會讓單列 jsonb 達數百 KB 並觸發 TOAST 外存，
/// 備份體積會被稽核表主導，而稽核軌跡查詢也會因此變慢。
pub async fn record(
    ex: impl PgExecutor<'_>,
    actor: Option<UserId>,
    entity: &str,
    entity_id: i64,
    action: &str,
    changes: Option<serde_json::Value>,
) -> sqlx::Result<()> {
    sqlx::query!(
        r#"
        INSERT INTO audit_logs (actor_id, entity, entity_id, action, changes)
        VALUES ($1, $2, $3, $4, $5)
        "#,
        actor.map(|u| u.0),
        entity,
        entity_id,
        action,
        changes,
    )
    .execute(ex)
    .await
    .map(|_| ())
}

/// 某筆資料的變更歷程，時間新到舊。
/// 索引 `(entity, entity_id, created_at DESC)` 直接供應此排序。
pub async fn history(
    ex: impl PgExecutor<'_>,
    entity: &str,
    entity_id: i64,
    limit: i64,
) -> sqlx::Result<Vec<AuditEntry>> {
    sqlx::query!(
        r#"
        SELECT id, actor_id, action, changes, created_at
        FROM audit_logs
        WHERE entity = $1 AND entity_id = $2
        ORDER BY created_at DESC, id DESC
        LIMIT $3
        "#,
        entity,
        entity_id,
        limit,
    )
    .fetch_all(ex)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(|r| AuditEntry {
                id: r.id,
                actor_id: r.actor_id.map(UserId),
                action: r.action,
                changes: r.changes,
                created_at: r.created_at,
            })
            .collect()
    })
}

#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub id: i64,
    pub actor_id: Option<UserId>,
    pub action: String,
    pub changes: Option<serde_json::Value>,
    pub created_at: time::OffsetDateTime,
}
