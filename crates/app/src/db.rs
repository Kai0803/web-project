//! 資料庫連線與 migration。

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
