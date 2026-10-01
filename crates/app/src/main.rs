//! 空調配管工程管理系統。
//!
//! 此處僅組裝：讀設定、建連線池、跑 migration、掛路由、啟動。
//! 業務規則在 `acms-domain`，交易編排在 `service`，資料庫存取在 `db`。

use acms_app::{api, auth, config, db};
use axum::http::{header, HeaderValue, StatusCode};
use axum::{extract::State, response::IntoResponse, routing::get, Json, Router};
use sqlx::PgPool;
use std::time::Duration;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[derive(Clone)]
struct AppState {
    pool: PgPool,
}

/// Content-Security-Policy。XSS 的縱深防護：`script-src 'self'` 讓
/// inline 的 `on*` handler 與外部 script 都無法執行，即使日後某處
/// 誤用了 `v-html`。
///
/// `style-src` 目前需要 `'unsafe-inline'`，因為幾個 `.vue` 檔還有
/// inline `style=`；把那些搬進 CSS class 之後就可以收緊這一條。
const CSP: &str = "default-src 'self'; script-src 'self'; \
                   style-src 'self' 'unsafe-inline'; img-src 'self' data:; \
                   object-src 'none'; frame-ancestors 'none'; base-uri 'none'; \
                   form-action 'self'";

#[tokio::main]
async fn main() {
    // 容器 probe 用。執行映像內既有的 binary，不必為了健康檢查在映像裡裝 curl。
    if std::env::args().nth(1).as_deref() == Some("healthcheck") {
        std::process::exit(i32::from(!healthcheck().await));
    }

    // 匯出 OpenAPI 規格。前端的 TypeScript 型別由此產生，
    // 契約的單一事實來源因而是 Rust 的型別定義。
    if std::env::args().nth(1).as_deref() == Some("openapi") {
        use utoipa::OpenApi;
        match serde_json::to_string_pretty(&api::ApiDoc::openapi()) {
            Ok(spec) => println!("{spec}"),
            Err(err) => {
                eprintln!("無法產生 OpenAPI 規格：{err}");
                std::process::exit(1);
            }
        }
        return;
    }

    // 建立第一個管理員帳號。
    //
    // 沒有這個指令，全新部署的系統無法登入——而在 migration 裡塞一組
    // 預設密碼等於把它公告在原始碼裡。這條路徑只在有人能執行容器指令時可用。
    if std::env::args().nth(1).as_deref() == Some("create-admin") {
        init_tracing();
        let args: Vec<String> = std::env::args().collect();
        let (Some(username), Some(password)) = (args.get(2), args.get(3)) else {
            eprintln!("用法：acms-app create-admin <帳號> <密碼>");
            std::process::exit(2);
        };
        if let Err(err) = create_admin(username, password).await {
            eprintln!("建立失敗：{err:#}");
            std::process::exit(1);
        }
        return;
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

/// 建立一個帶 admin 角色的帳號。帳號已存在時只重設密碼並補上角色，
/// 讓忘記密碼時也有一條救援路徑。
async fn create_admin(username: &str, password: &str) -> anyhow::Result<()> {
    const MIN_LEN: usize = 10;
    if password.chars().count() < MIN_LEN {
        anyhow::bail!("密碼至少需要 {MIN_LEN} 個字元");
    }

    let config = config::Config::from_env()?;
    let pool = db::connect(&config.database_url, 2, config.db_connect_timeout).await?;
    db::migrate(&pool).await?;

    let hash = auth::hash_password_blocking(password.to_owned())
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let mut tx = pool.begin().await?;
    let id = sqlx::query_scalar!(
        r#"
        INSERT INTO users (username, password_hash, display_name)
        VALUES ($1, $2, $1)
        ON CONFLICT (username) DO UPDATE SET password_hash = EXCLUDED.password_hash
        RETURNING id
        "#,
        username,
        hash,
    )
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query!(
        "INSERT INTO user_roles (user_id, role_code) VALUES ($1,'admin') ON CONFLICT DO NOTHING",
        id,
    )
    .execute(&mut *tx)
    .await?;

    // 改密碼即撤銷既有 session，與帳號管理端點的行為一致
    sqlx::query!("DELETE FROM sessions WHERE user_id = $1", id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    info!(username, "管理員帳號已就緒");
    Ok(())
}

async fn run() -> anyhow::Result<()> {
    let config = config::Config::from_env()?;
    info!(port = config.port, "設定已載入");

    // 對外提供服務卻沒設 PUBLIC_ORIGIN 時直接拒絕啟動。
    // 那個組合下 CSRF 的來源比對會退回與請求自帶的 Host 比對，
    // 而反向代理會改寫 Host——結果是每個寫入請求都 403。
    // 與「不使用預設值默默啟動」同一條原則：這種錯誤要在啟動時就講清楚。
    if config.cookie_secure && config.public_origin.is_none() {
        anyhow::bail!(
            "COOKIE_SECURE=true 時必須設定 PUBLIC_ORIGIN（例如 https://acms.example.com），\
             否則置於反向代理之後時所有寫入請求都會被 CSRF 檢查拒絕"
        );
    }

    if !config.cookie_secure {
        // 誤設不該悄悄過去：少了 Secure，session cookie 會在明文 HTTP 上傳輸
        tracing::warn!(
            "COOKIE_SECURE=false：session cookie 不帶 Secure，僅適用於本機 HTTP 開發。\
             以 HTTPS 對外提供服務時必須設為 true"
        );
    }

    let pool = db::connect(
        &config.database_url,
        config.db_max_connections,
        config.db_connect_timeout,
    )
    .await?;

    db::migrate(&pool).await?;

    // 啟動時清一次過期 session。存量小，不需要排程。
    match auth::purge_expired(&pool).await {
        Ok(n) if n > 0 => info!(count = n, "已清除過期 session"),
        Ok(_) => {}
        Err(err) => error!(error = %err, "清除過期 session 失敗，不影響啟動"),
    }

    let app = Router::new()
        .route("/healthz", get(healthz))
        .with_state(AppState { pool: pool.clone() })
        .merge(api::routes(api::AppState {
            pool,
            cookie_secure: config.cookie_secure,
            public_origin: config.public_origin.clone(),
        }))
        // 前端由後端同源提供：省掉 CORS，也讓 cookie 認證單純。
        // 找不到的路徑一律回 index.html，交給前端路由處理——
        // 使用者直接開 /quotations/12 或重新整理時才不會拿到 404。
        .fallback_service(
            ServeDir::new(&config.static_dir)
                .fallback(ServeFile::new(config.static_dir.join("index.html"))),
        )
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CSP),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        // 路徑含識別字（/quotations/12），不該隨 Referer 外流
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("same-origin"),
        ))
        // 任何請求都不該無限期佔住連線。typst 排版是最慢的路徑，
        // 30 秒遠大於實測的數百毫秒。
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(30),
        ))
        .layer(TraceLayer::new_for_http());

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
