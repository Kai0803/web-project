//! HTTP handler。
//!
//! 每個 handler 只做三件事：解析輸入、呼叫 `service` 或 `db`、組裝回應。
//! 業務規則不寫在這裡。

pub mod admin;
pub mod auth;
pub mod contracts;
pub mod customers;
pub mod pdf;
pub mod projects;
pub mod quotations;

/// `/api` 底下未匹配的路徑。
///
/// 少了這條，打錯的 API 路徑會流到 SPA 的靜態檔 fallback，得到
/// **200 + index.html**：前端拿 HTML 去 `JSON.parse` 會炸，而對任何
/// API 客戶端來說，把 404 變成 200 更糟。
pub async fn not_found() -> crate::api::ApiError {
    crate::api::ApiError::from(crate::service::ServiceError::NotFound { entity: "端點" })
}
