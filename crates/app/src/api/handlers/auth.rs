//! 登入、登出與目前使用者。

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};

use crate::api::dto::{LoginRequest, MeResponse};
use crate::api::{ApiError, AppState, CsrfGuard, CurrentUser};
use crate::auth;

/// 建立 session cookie。
///
/// `HttpOnly` 讓 JavaScript 讀不到它，因此 XSS 無法竊取 session；
/// `SameSite=Strict` 是 CSRF 的第一道防線。`Secure` 依部署而定——
/// 本機以 HTTP 開發時若強制 Secure，cookie 根本不會被送出。
fn session_cookie(token: String, secure: bool) -> Cookie<'static> {
    Cookie::build((auth::COOKIE_NAME, token))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Strict)
        .secure(secure)
        .max_age(auth::SESSION_TTL)
        .build()
}

#[utoipa::path(
    post, path = "/api/v1/auth/login",
    request_body = LoginRequest,
    responses(
        (status = 204, description = "登入成功，session 寫入 cookie"),
        (status = 401, description = "帳號或密碼錯誤"),
        (status = 403, description = "帳號已停用"),
    ),
    tag = "auth",
)]
pub async fn login(
    State(state): State<AppState>,
    // 登入也掛 CSRF。它目前靠「`Json` 要求 application/json → 跨源 fetch 觸發
    // preflight → 無 CorsLayer → 瀏覽器攔下」擋住登入 CSRF，但那是間接的保護。
    // 前端對所有方法都送 X-Requested-With，加這個參數不會改變行為。
    _csrf: CsrfGuard,
    jar: CookieJar,
    Json(body): Json<LoginRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let token = auth::login(&state.pool, &body.username, &body.password).await?;
    let jar = jar.add(session_cookie(token, state.cookie_secure));
    Ok((StatusCode::NO_CONTENT, jar))
}

#[utoipa::path(
    post, path = "/api/v1/auth/logout",
    responses((status = 204, description = "已登出")),
    tag = "auth",
)]
pub async fn logout(
    State(state): State<AppState>,
    _csrf: CsrfGuard,
    jar: CookieJar,
) -> Result<impl IntoResponse, ApiError> {
    if let Some(cookie) = jar.get(auth::COOKIE_NAME) {
        auth::logout(&state.pool, cookie.value()).await?;
    }

    // 以同樣的屬性移除，否則瀏覽器可能留著原 cookie
    let jar = jar.remove(Cookie::build(auth::COOKIE_NAME).path("/").build());
    Ok((StatusCode::NO_CONTENT, jar))
}

#[utoipa::path(
    get, path = "/api/v1/auth/me",
    responses(
        (status = 200, body = MeResponse),
        (status = 401, description = "尚未登入"),
    ),
    tag = "auth",
)]
pub async fn me(CurrentUser(user): CurrentUser) -> Json<MeResponse> {
    Json(MeResponse::from(&user))
}
