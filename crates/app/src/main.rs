//! 空調配管工程管理系統。
//!
//! 此處僅組裝：讀設定、建連線池、跑 migration、掛路由、啟動。
//! 業務規則在 `acms-domain`，交易編排在 `service`，資料庫存取在 `db`。

mod config;
mod db;

use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use sqlx::PgPool;
use tower_http::trace::TraceLayer;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[derive(Clone)]
struct AppState {
    pool: PgPool,
}

#[tokio::main]
async fn main() {
    // 容器 probe 用。執行映像內既有的 binary，不必為了健康檢查在映像裡裝 curl。
    if std::env::args().nth(1).as_deref() == Some("healthcheck") {
        std::process::exit(i32::from(!healthcheck().await));
    }

    init_tracing();

    if let Err(err) = run().await {
        // 以 {:#} 印出完整的錯誤鏈，否則只會看到最外層那句
        error!("啟動失敗：{err:#}");
        std::process::exit(1);
    }
}

/// 對自己的 /healthz 發一個最小的 HTTP 請求。
///
/// 刻意不引入 HTTP client：這裡只需要判斷狀態列是不是 200。
async fn healthcheck() -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".to_owned());
    let request = format!("GET /healthz HTTP/1.0\r\nHost: localhost:{port}\r\n\r\n");

    let attempt = async {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port.parse().ok()?))
            .await
            .ok()?;
        stream.write_all(request.as_bytes()).await.ok()?;

        let mut response = String::new();
        stream.read_to_string(&mut response).await.ok()?;
        Some(response.starts_with("HTTP/1.0 200") || response.starts_with("HTTP/1.1 200"))
    };

    match tokio::time::timeout(std::time::Duration::from_secs(5), attempt).await {
        Ok(Some(ok)) => ok,
        _ => false,
    }
}

async fn run() -> anyhow::Result<()> {
    let config = config::Config::from_env()?;
    info!(port = config.port, "設定已載入");

    let pool = db::connect(
        &config.database_url,
        config.db_max_connections,
        config.db_connect_timeout,
    )
    .await?;

    db::migrate(&pool).await?;

    let app = Router::new()
        .route("/healthz", get(healthz))
        .layer(TraceLayer::new_for_http())
        .with_state(AppState { pool });

    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("服務啟動於 http://{addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("服務已停止");
    Ok(())
}

/// 健康檢查。實際查詢資料庫，不只回報行程存活——
/// 資料庫斷線時若仍回 200，probe 就失去意義。
async fn healthz(State(state): State<AppState>) -> impl IntoResponse {
    match db::ping(&state.pool).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "status": "ok" }))),
        Err(err) => {
            error!(error = %err, "健康檢查失敗");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "status": "error" })),
            )
        }
    }
}

/// 收到 SIGTERM 或 Ctrl-C 後停止接受新請求，等待進行中的請求完成。
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("無法註冊 Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("無法註冊 SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => info!("收到 Ctrl-C，開始關閉"),
        () = terminate => info!("收到 SIGTERM，開始關閉"),
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("acms_app=info,tower_http=info,sqlx=warn"));

    // 正式環境輸出 JSON 供日誌系統解析，開發環境輸出人眼可讀格式
    if std::env::var("LOG_FORMAT").as_deref() == Ok("json") {
        tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer().json())
            .init();
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer())
            .init();
    }
}
