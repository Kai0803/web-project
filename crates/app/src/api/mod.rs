//! HTTP 介面層：handler、DTO、輸入驗證。
//!
//! 這一層只做解析與組裝，業務規則在 `acms_domain`，交易編排在 `service`。
//! 開啟交易或跨表的操作一律經 `service`；單表 CRUD 允許直接呼叫 `db`。

pub mod dto;
pub mod error;
mod extract;
mod handlers;
mod openapi;

pub use error::{ApiError, ErrorCode};
pub use extract::{ApiJson, ApiPath, ApiQuery, CsrfGuard, CurrentUser};
pub use openapi::ApiDoc;

use axum::routing::{any, get, patch, post, put};
use axum::Router;
use sqlx::PgPool;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub cookie_secure: bool,
    /// 預期的前端來源，例如 `https://acms.example.com`。
    ///
    /// CSRF 的來源比對以此為準。`None` 時退回與請求自帶的 `Host` 比對，
    /// 那只適用於本機直連——詳見 `CsrfGuard`。
    pub public_origin: Option<String>,
}

/// 掛載所有 `/api/v1` 路由。
pub fn routes(state: AppState) -> Router {
    Router::new()
        // 認證
        .route("/api/v1/auth/login", post(handlers::auth::login))
        .route("/api/v1/auth/logout", post(handlers::auth::logout))
        .route("/api/v1/auth/me", get(handlers::auth::me))
        // 客戶
        .route(
            "/api/v1/customers",
            get(handlers::customers::list).post(handlers::customers::create),
        )
        // 靜態片段的路由寫在 {id} 之前：matchit 本身是靜態優先，
        // 但順序寫對讓讀的人不必知道這件事
        .route(
            "/api/v1/customers/options",
            get(handlers::customers::options),
        )
        .route("/api/v1/customers/{id}", get(handlers::customers::detail))
        .route("/api/v1/customers/{id}", patch(handlers::customers::update))
        // 專案
        .route(
            "/api/v1/projects",
            get(handlers::projects::list).post(handlers::projects::create),
        )
        .route("/api/v1/projects/{id}", get(handlers::projects::detail))
        .route(
            "/api/v1/projects/{id}/quotations",
            get(handlers::quotations::list_for_project).post(handlers::quotations::create),
        )
        // 報價單
        .route("/api/v1/quotations/{id}", get(handlers::quotations::detail))
        .route(
            "/api/v1/quotations/{id}/pdf",
            get(handlers::pdf::quotation_pdf),
        )
        .route(
            "/api/v1/quotations/{id}/items",
            put(handlers::quotations::replace_items),
        )
        .route(
            "/api/v1/quotations/{id}/submit",
            post(handlers::quotations::submit),
        )
        .route(
            "/api/v1/quotations/{id}/approve",
            post(handlers::quotations::approve),
        )
        .route(
            "/api/v1/quotations/{id}/reject",
            post(handlers::quotations::reject),
        )
        .route(
            "/api/v1/quotations/{id}/reopen",
            post(handlers::quotations::reopen),
        )
        .route(
            "/api/v1/quotations/{id}/versions",
            post(handlers::quotations::new_version),
        )
        .route(
            "/api/v1/quotations/{id}/contract",
            post(handlers::contracts::create_from_quotation),
        )
        // 合約
        .route("/api/v1/contracts/{id}", get(handlers::contracts::detail))
        // 帳號與系統設定
        .route(
            "/api/v1/users",
            get(handlers::admin::list_users).post(handlers::admin::create_user),
        )
        .route("/api/v1/users/{id}", patch(handlers::admin::update_user))
        .route("/api/v1/roles", get(handlers::admin::list_roles))
        .route(
            "/api/v1/permissions",
            get(handlers::admin::list_permissions),
        )
        .route(
            "/api/v1/company",
            get(handlers::admin::get_company).put(handlers::admin::update_company),
        )
        // 打錯的 API 路徑一律回 404 信封，不讓它流到 SPA 的靜態檔 fallback
        .route("/api/{*path}", any(handlers::not_found))
        .with_state(state)
}
