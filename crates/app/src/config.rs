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
    /// 置於反向代理之後時為 true，才信任 `X-Forwarded-*` 標頭。
    ///
    /// 目前無消費端：CSRF 的來源比對用的是請求自帶的 `Host`，還沒有處理
    /// 代理改寫 `Host` 的情形。欄位先留著，因為部署 manifest 已經設了這個變數。
    #[allow(dead_code)]
    pub trust_proxy: bool,
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
            trust_proxy: flag("TRUST_PROXY"),
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
