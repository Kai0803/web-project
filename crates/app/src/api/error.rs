//! 對外的錯誤格式與狀態碼映射。

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use std::collections::BTreeMap;
use tracing::error;
use utoipa::ToSchema;

use crate::auth::AuthError;
use crate::service::ServiceError;

/// 機器可判讀的錯誤代碼。
///
/// 是列舉而非自由字串，因此會進 OpenAPI 成為前端的 union type——
/// 前端才知道全集，不必猜有哪些可能。
#[derive(Debug, Clone, Copy, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    /// 輸入不合法，`fields` 會指出是哪些欄位。
    ValidationFailed,
    /// 未登入或登入已逾期。
    Unauthenticated,
    /// 已登入但權限不足。
    Forbidden,
    NotFound,
    /// 違反業務規則，例如核准一份草稿。
    RuleViolation,
    /// 資料已被他人修改，重新載入後可重試。
    Conflict,
    /// 系統內部錯誤。`message` 僅含追蹤 ID。
    Internal,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    pub code: ErrorCode,
    /// 給人看的繁中訊息。
    pub message: String,
    /// 欄位層級的錯誤，供表單直接標示。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorResponse {
    pub error: ErrorBody,
}

/// API 層的錯誤。`service` 與 `auth` 的錯誤在此收斂並決定狀態碼。
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: ErrorCode,
    message: String,
    fields: Option<BTreeMap<String, String>>,
}

impl ApiError {
    pub fn validation(message: impl Into<String>, fields: BTreeMap<String, String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: ErrorCode::ValidationFailed,
            message: message.into(),
            fields: Some(fields),
        }
    }

    pub fn unauthenticated(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: ErrorCode::Unauthenticated,
            message: message.into(),
            fields: None,
        }
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code: ErrorCode::Forbidden,
            message: message.into(),
            fields: None,
        }
    }

    /// 業務上的衝突。與 `ServiceError::Conflict`（樂觀鎖）共用狀態碼，
    /// 但訊息自訂——樂觀鎖那句「請重新載入後再試」對「帳號已存在」
    /// 之類的衝突是錯誤的指示。
    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: ErrorCode::Conflict,
            message: message.into(),
            fields: None,
        }
    }

    /// 內部錯誤。詳情只進日誌，對外僅給追蹤 ID——
    /// 錯誤訊息裡的 SQL 或檔案路徑對攻擊者很有價值。
    pub fn internal(context: &str, detail: impl std::fmt::Display) -> Self {
        let trace_id = uuid::Uuid::new_v4();
        error!(%trace_id, context, error = %detail, "內部錯誤");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: ErrorCode::Internal,
            message: format!("系統發生錯誤，請提供此代碼供查詢：{trace_id}"),
            fields: None,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorResponse {
                error: ErrorBody {
                    code: self.code,
                    message: self.message,
                    fields: self.fields,
                },
            }),
        )
            .into_response()
    }
}

impl From<ServiceError> for ApiError {
    fn from(err: ServiceError) -> Self {
        match err {
            ServiceError::NotFound { entity } => Self {
                status: StatusCode::NOT_FOUND,
                code: ErrorCode::NotFound,
                message: format!("找不到{entity}"),
                fields: None,
            },
            ServiceError::Denied(ref denied) => Self::forbidden(denied.to_string()),
            ServiceError::Conflict => Self {
                status: StatusCode::CONFLICT,
                code: ErrorCode::Conflict,
                message: err.to_string(),
                fields: None,
            },
            ServiceError::AlreadyApproved
            | ServiceError::AlreadyContracted
            | ServiceError::NotLatestVersion { .. } => Self {
                status: StatusCode::CONFLICT,
                code: ErrorCode::RuleViolation,
                message: err.to_string(),
                fields: None,
            },
            ServiceError::EmptyQuotation => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                code: ErrorCode::RuleViolation,
                message: err.to_string(),
                fields: None,
            },
            ServiceError::Rule(ref rule) => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                code: ErrorCode::RuleViolation,
                message: rule.to_string(),
                fields: None,
            },
            // 金額錯誤源於使用者輸入的數量或單價，屬驗證失敗而非系統故障
            ServiceError::Money(ref money) => Self {
                status: StatusCode::BAD_REQUEST,
                code: ErrorCode::ValidationFailed,
                message: money.to_string(),
                fields: None,
            },
            ServiceError::Database(ref db) => Self::internal("service", db),
        }
    }
}

impl From<AuthError> for ApiError {
    fn from(err: AuthError) -> Self {
        match err {
            // 帳號不存在與密碼錯誤回相同訊息，避免洩漏哪些帳號存在
            AuthError::InvalidCredentials => Self::unauthenticated(err.to_string()),
            // 停用是在密碼驗證通過之後才判斷的，此時帳號存在對當事人已非秘密。
            // 回傳明確原因，否則使用者只會一直重試密碼。
            AuthError::Inactive => Self::forbidden(err.to_string()),
            AuthError::NoSession => Self::unauthenticated(err.to_string()),
            AuthError::Hashing => Self::internal("auth", err),
            AuthError::Database(ref db) => Self::internal("auth", db),
        }
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        Self::internal("db", err)
    }
}
