//! 資料庫存取。
//!
//! # 簽章契約
//!
//! 所有查詢函式接受 `impl PgExecutor<'_>`，**絕不接受 `PgPool`**。
//! 收 pool 的話任兩個呼叫永遠無法落在同一個交易裡，而報價改版、核准、
//! 轉合約、明細重算這四個流程全都需要單一交易。交易由 `service` 層開啟。
//!
//! 這裡刻意不抽 repository trait：唯一的替換情境是換資料庫，而 Postgres
//! 已定案。抽 trait 只會多一層轉接而換不到實際彈性。純邏輯都在
//! `acms-domain`，那裡本來就沒有 I/O，測試不需要資料庫。
//!
//! 帶 `sqlx::FromRow` 的 row struct 一律不外流，對外只回傳 `acms_domain` 的型別。

pub mod audit;
pub mod company;
pub mod customer;
pub mod numbering;
pub mod project;
pub mod quotation;
pub mod user;

use std::time::Duration;

use sqlx::postgres::{PgPool, PgPoolOptions};
use tracing::{info, warn};

/// 建立連線池，並在資料庫尚未就緒時以指數退避重試。
///
/// `podman kube play` 不保證容器啟動順序，app 可能早於 postgres 就緒。
/// 這裡以重試處理，而非依賴部署層的啟動順序。
pub async fn connect(
    database_url: &str,
    max_connections: u32,
    timeout: Duration,
) -> anyhow::Result<PgPool> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut backoff = Duration::from_millis(250);

    loop {
        let attempt = PgPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(Duration::from_secs(5))
            .connect(database_url)
            .await;

        match attempt {
            Ok(pool) => {
                info!("資料庫連線已建立");
                return Ok(pool);
            }
            Err(err) => {
                let now = tokio::time::Instant::now();
                if now >= deadline {
                    return Err(anyhow::anyhow!(err).context("等待資料庫就緒逾時"));
                }

                let remaining = deadline - now;
                let wait = backoff.min(remaining);
                warn!(error = %err, ?wait, "資料庫尚未就緒，稍後重試");
                tokio::time::sleep(wait).await;
                backoff = (backoff * 2).min(Duration::from_secs(5));
            }
        }
    }
}

/// 執行 migration。
///
/// 單一實例部署，於啟動時執行即可，無並行競爭問題。
/// 若之後要跑多個 app 實例，須改為獨立的 migration 步驟。
pub async fn migrate(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::migrate!("./migrations").run(pool).await?;
    info!("migration 已完成");
    Ok(())
}

/// 供健康檢查使用：實際對資料庫下查詢，而非僅回報行程存活。
pub async fn ping(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT 1").execute(pool).await.map(|_| ())
}

/// 分頁參數。
///
/// `LIMIT/OFFSET` 在此規模足夠：5 年 7,500 張報價的最深一頁約 5–15 ms，
/// 佔 500 ms 預算的 3%。第三階段工時紀錄成長後再評估 keyset 分頁。
#[derive(Debug, Clone, Copy)]
pub struct Page {
    pub limit: i64,
    pub offset: i64,
}

impl Page {
    pub const DEFAULT_LIMIT: i64 = 50;
    pub const MAX_LIMIT: i64 = 200;

    /// 由使用者輸入建立，限制上限避免一次撈整張表。
    #[must_use]
    pub fn new(limit: Option<i64>, offset: Option<i64>) -> Self {
        Self {
            limit: limit
                .unwrap_or(Self::DEFAULT_LIMIT)
                .clamp(1, Self::MAX_LIMIT),
            offset: offset.unwrap_or(0).max(0),
        }
    }
}

impl Default for Page {
    fn default() -> Self {
        Self::new(None, None)
    }
}

#[cfg(test)]
mod tests {
    use super::Page;

    #[test]
    fn page_clamps_limit_and_rejects_negative_offset() {
        assert_eq!(Page::new(None, None).limit, Page::DEFAULT_LIMIT);
        assert_eq!(Page::new(Some(10_000), None).limit, Page::MAX_LIMIT);
        assert_eq!(Page::new(Some(0), None).limit, 1);
        assert_eq!(Page::new(Some(-5), None).limit, 1);
        assert_eq!(Page::new(None, Some(-1)).offset, 0);
    }
}
