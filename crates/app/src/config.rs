//! 設定讀取。
//!
//! 所有設定在啟動時一次讀齊並驗證，缺少必要項目即明確失敗——
//! 不使用預設值默默啟動，那會讓設定錯誤在營運中才浮現。

use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub port: u16,
    /// 透過 HTTPS 提供服務時為 true，session cookie 才會帶 Secure。
    pub cookie_secure: bool,
    /// 對外的來源，例如 `https://acms.example.com`。CSRF 的來源比對以此為準。
    ///
    /// 置於反向代理之後時**必須**設定：代理預設會把 `Host` 改寫成 upstream
    /// 的名字，而退回比對 `Host` 的路徑屆時會拒絕每一個寫入請求。
    pub public_origin: Option<String>,
    /// 等待資料庫就緒的上限。容器啟動順序不保證，因此以重試處理。
    pub db_connect_timeout: Duration,
    pub db_max_connections: u32,
    /// 前端建置產物的位置。容器內為映像中的固定路徑，開發時指向 web/dist。
    pub static_dir: std::path::PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("缺少必要的環境變數 {name}")]
    Missing { name: &'static str },

    #[error("環境變數 {name} 的值無法解析：{value}")]
    Invalid { name: &'static str, value: String },
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            database_url: require("DATABASE_URL")?,
            port: parse_or("PORT", 8080)?,
            cookie_secure: flag("COOKIE_SECURE"),
            public_origin: std::env::var("PUBLIC_ORIGIN")
                .ok()
                .filter(|v| !v.trim().is_empty()),
            db_connect_timeout: Duration::from_secs(parse_or("DB_CONNECT_TIMEOUT_SECS", 30)?),
            db_max_connections: parse_or("DB_MAX_CONNECTIONS", 10)?,
            static_dir: std::env::var("STATIC_DIR")
                .unwrap_or_else(|_| "web/dist".to_owned())
                .into(),
        })
    }
}

fn require(name: &'static str) -> Result<String, ConfigError> {
    std::env::var(name).map_err(|_| ConfigError::Missing { name })
}

fn parse_or<T: std::str::FromStr>(name: &'static str, default: T) -> Result<T, ConfigError> {
    match std::env::var(name) {
        Err(_) => Ok(default),
        Ok(raw) => raw.parse().map_err(|_| ConfigError::Invalid {
            name,
            value: raw.clone(),
        }),
    }
}

/// 僅 "true" 視為開啟，其餘（含未設定）皆為關閉。
fn flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| v == "true")
}
