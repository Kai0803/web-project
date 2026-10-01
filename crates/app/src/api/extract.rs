//! Axum extractor：登入使用者與 CSRF 防護。

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum_extra::extract::CookieJar;

use acms_domain::{User, UserKind};

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

        // 第一階段只開放同仁登入。客戶入口是第二階段的工作，在那之前
        // customer_contact 帳號不得通過任何端點——少了這道，客戶入口
        // 上線的那天所有既有端點會一次全部對客戶開放，因為它們只檢查
        // 權限、不檢查「這筆資料是不是你的」（見 authz::can_access_customer）。
        if user.kind != UserKind::Staff {
            return Err(ApiError::forbidden("此帳號類型尚不支援登入"));
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

// ==================== 錯誤格式一致的 extractor ====================

/// `axum::Json` 的替代品，差別只在**失敗時**的回應格式。
///
/// axum 內建的 rejection 以 `text/plain` 回傳英文的 serde 訊息，不走
/// [`ApiError`] 的 `{"error":{…}}` 信封（見 docs/g2-design.md 的錯誤契約）。
/// 前端的 `JSON.parse` 因此直接丟 SyntaxError，使用者看到的是
/// `Unexpected token 'F'…` 而不是我們寫的繁中訊息。
///
/// 最常見的觸發不是欄位拼錯，而是**數量欄位多打一個空格**——
/// 金額與數量以字串反序列化成 `Decimal`，解析失敗走的就是 `JsonDataError`。
///
/// 所有請求體一律用這個型別解析，對外的錯誤格式才只有一種。
pub struct ApiJson<T>(pub T);

impl<T, S> axum::extract::FromRequest<S> for ApiJson<T>
where
    T: serde::de::DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Self::Rejection> {
        axum::Json::<T>::from_request(req, state)
            .await
            .map(|axum::Json(value)| Self(value))
            .map_err(|rejection: axum::extract::rejection::JsonRejection| {
                // 請求體解析失敗一律是呼叫端的輸入問題 → 400。
                // 細節放 fields.body：內容是請求自己的欄位名與型別，
                // 不含伺服器內部資訊，而少了它前端無從得知是哪裡錯。
                ApiError::validation(
                    "請求內容格式不正確",
                    [("body".to_owned(), rejection.body_text())]
                        .into_iter()
                        .collect(),
                )
            })
    }
}

/// `axum::extract::Path` 的替代品。理由同 [`ApiJson`]：
/// `/api/v1/quotations/abc` 原本回 400 `text/plain`。
pub struct ApiPath<T>(pub T);

impl<T, S> FromRequestParts<S> for ApiPath<T>
where
    T: serde::de::DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        axum::extract::Path::<T>::from_request_parts(parts, state)
            .await
            .map(|axum::extract::Path(value)| Self(value))
            .map_err(|rejection: axum::extract::rejection::PathRejection| {
                ApiError::validation(
                    "路徑參數不正確",
                    [("path".to_owned(), rejection.body_text())]
                        .into_iter()
                        .collect(),
                )
            })
    }
}

/// `axum::extract::Query` 的替代品。理由同 [`ApiJson`]。
pub struct ApiQuery<T>(pub T);

impl<T, S> FromRequestParts<S> for ApiQuery<T>
where
    T: serde::de::DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        axum::extract::Query::<T>::from_request_parts(parts, state)
            .await
            .map(|axum::extract::Query(value)| Self(value))
            .map_err(|rejection: axum::extract::rejection::QueryRejection| {
                ApiError::validation(
                    "查詢參數不正確",
                    [("query".to_owned(), rejection.body_text())]
                        .into_iter()
                        .collect(),
                )
            })
    }
}
