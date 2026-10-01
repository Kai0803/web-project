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
pub use extract::{CsrfGuard, CurrentUser};
pub use openapi::ApiDoc;

use axum::routing::{get, patch, post, put};
use axum::Router;
use sqlx::PgPool;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub cookie_secure: bool,
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
        .with_state(state)
}
