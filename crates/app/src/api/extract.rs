//! Axum extractor：登入使用者與 CSRF 防護。

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum_extra::extract::CookieJar;

use acms_domain::User;

use super::{error::ApiError, AppState};
use crate::{auth, db};

/// 已登入的使用者。放進 handler 參數即代表該端點需要登入。
///
/// 權限檢查不在這裡——那是業務規則，由 `service` 層依 `User.permissions` 判斷。
#[derive(Debug, Clone)]
pub struct CurrentUser(pub User);

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let jar = CookieJar::from_headers(&parts.headers);
        let token = jar
            .get(auth::COOKIE_NAME)
            .map(|c| c.value().to_owned())
            .ok_or_else(|| ApiError::unauthenticated("尚未登入"))?;

        let user_id = auth::resolve(&state.pool, &token).await?;

        let user = db::user::load(&state.pool, user_id)
            .await?
            .ok_or_else(|| ApiError::unauthenticated("登入資料已失效"))?;

        // 停用的帳號即使持有有效 session 也不得繼續操作
        if !user.is_active {
            return Err(ApiError::forbidden("此帳號已停用"));
        }

        Ok(Self(user))
    }
}

/// CSRF 防護。放進任何會改變資料的 handler 參數即生效。
///
/// cookie 認證的代價就是 CSRF。三道組合：
///
/// 1. cookie 設 `SameSite=Strict`（在 `handlers::auth` 設定）
/// 2. 檢查 `Origin` 與本次請求的 `Host` 相符
/// 3. 要求自訂標頭 `X-Requested-With`——跨站的表單送出無法設定自訂標頭
///
/// 前一版 Node 系統用的是 CSRF token。這裡改用上述組合，因為前後端同源，
/// 不需要把 token 傳來傳去。
#[derive(Debug, Clone, Copy)]
pub struct CsrfGuard;

const REQUESTED_WITH: &str = "x-requested-with";

impl FromRequestParts<AppState> for CsrfGuard {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if parts.headers.get(REQUESTED_WITH).is_none() {
            return Err(ApiError::forbidden(
                "缺少 X-Requested-With 標頭，請求已被拒絕",
            ));
        }

        // Origin 缺席的情況：同源的非瀏覽器用戶端（如 curl）。
        // 既然已要求自訂標頭，這裡只在 Origin 存在時比對它。
        if let Some(origin) = parts.headers.get(axum::http::header::ORIGIN) {
            let origin = origin
                .to_str()
                .map_err(|_| ApiError::forbidden("Origin 標頭格式不正確"))?;

            let host = parts
                .headers
                .get(axum::http::header::HOST)
                .and_then(|h| h.to_str().ok())
                .ok_or_else(|| ApiError::forbidden("缺少 Host 標頭"))?;

            let origin_host = origin
                .rsplit_once("//")
                .map(|(_, rest)| rest)
                .unwrap_or(origin);

            if origin_host != host {
                return Err(ApiError::forbidden("請求來源不符，已被拒絕"));
            }
        }

        Ok(Self)
    }
}
