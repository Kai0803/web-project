//! 帳號與系統設定 handler。
//!
//! 這兩組端點是系統可被交付的前提：沒有帳號管理，管理員只能下 SQL 開帳號；
//! 沒有公司資料設定，報價單印不出抬頭。

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use acms_domain::{authz::Permission, CompanyProfile, Permissions, UserId};

use crate::api::{ApiError, ApiJson, ApiPath, AppState, CsrfGuard, CurrentUser};
use crate::service::ServiceError;
use crate::{auth, db};

// ==================== 帳號 ====================

#[derive(Debug, Serialize, ToSchema)]
pub struct UserSummary {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub is_active: bool,
    pub roles: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    pub display_name: String,
    /// 角色代碼，例如 `["sales"]`。可給多個，權限取聯集。
    pub roles: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateUserRequest {
    pub display_name: Option<String>,
    pub is_active: Option<bool>,
    pub roles: Option<Vec<String>>,
    /// 給值即重設密碼。不給則不動。
    pub password: Option<String>,
}

/// 密碼最短長度。太短的密碼配上 argon2 仍然擋不住字典攻擊。
const MIN_PASSWORD_LEN: usize = 10;

fn require(perms: &Permissions, p: Permission) -> Result<(), ApiError> {
    perms
        .require(p)
        .map_err(|d| ApiError::forbidden(d.to_string()))
}

#[utoipa::path(
    get, path = "/api/v1/users",
    responses((status = 200, body = Vec<UserSummary>), (status = 403)),
    tag = "admin",
)]
pub async fn list_users(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Vec<UserSummary>>, ApiError> {
    require(&user.permissions, Permission::UserManage)?;

    let rows = sqlx::query!(
        r#"
        SELECT u.id, u.username, u.display_name, u.is_active,
               COALESCE(array_agg(ur.role_code) FILTER (WHERE ur.role_code IS NOT NULL),
                        ARRAY[]::text[]) AS "roles!: Vec<String>"
        FROM users u
        LEFT JOIN user_roles ur ON ur.user_id = u.id
        WHERE u.kind = 'staff'
        GROUP BY u.id
        ORDER BY u.username, u.id
        "#
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(
        rows.into_iter()
            .map(|r| UserSummary {
                id: r.id,
                username: r.username,
                display_name: r.display_name,
                is_active: r.is_active,
                roles: r.roles,
            })
            .collect(),
    ))
}

#[utoipa::path(
    post, path = "/api/v1/users",
    request_body = CreateUserRequest,
    responses(
        (status = 200, body = UserSummary),
        (status = 400, description = "密碼過短或角色不存在"),
        (status = 409, description = "帳號已存在"),
    ),
    tag = "admin",
)]
pub async fn create_user(
    State(state): State<AppState>,
    CurrentUser(actor): CurrentUser,
    _csrf: CsrfGuard,
    ApiJson(body): ApiJson<CreateUserRequest>,
) -> Result<Json<UserSummary>, ApiError> {
    require(&actor.permissions, Permission::UserManage)?;

    if body.username.trim().is_empty() {
        return Err(ApiError::validation(
            "請填寫帳號",
            [("username".to_owned(), "不可為空".to_owned())].into(),
        ));
    }
    if body.password.chars().count() < MIN_PASSWORD_LEN {
        return Err(ApiError::validation(
            format!("密碼至少需要 {MIN_PASSWORD_LEN} 個字元"),
            [(
                "password".to_owned(),
                format!("至少 {MIN_PASSWORD_LEN} 字元"),
            )]
            .into(),
        ));
    }

    let hash = auth::hash_password_blocking(body.password.clone()).await?;
    let username = body.username.trim().to_owned();
    // 寫入與回應共用同一個正規化後的值，否則同一次請求的回應與儲存值不一致
    let display_name = body.display_name.trim().to_owned();
    if display_name.is_empty() {
        return Err(ApiError::validation(
            "請填寫顯示名稱",
            [("display_name".to_owned(), "不可為空".to_owned())].into(),
        ));
    }

    let mut tx = state.pool.begin().await?;

    let id = sqlx::query_scalar!(
        "INSERT INTO users (username, password_hash, display_name) VALUES ($1,$2,$3) RETURNING id",
        username,
        hash,
        display_name,
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(
        |e| match e.as_database_error().and_then(|d| d.constraint()) {
            // 不借用 ServiceError::Conflict 的訊息：那句是樂觀鎖用的，
            // 「請重新載入後再試」對「帳號已存在」是錯的指示
            Some("users_username_key") => ApiError::conflict("此帳號已存在，請改用其他帳號"),
            _ => ApiError::from(e),
        },
    )?;

    assign_roles(&mut tx, UserId(id), &body.roles).await?;

    db::audit::record(
        &mut *tx,
        Some(actor.id),
        "user",
        id,
        "create",
        Some(serde_json::json!({ "username": username, "roles": body.roles })),
    )
    .await?;

    tx.commit().await?;

    Ok(Json(UserSummary {
        id,
        username,
        display_name,
        is_active: true,
        roles: body.roles,
    }))
}

/// 指派角色。不存在的角色由外鍵擋下，轉成可讀的驗證錯誤。
async fn assign_roles(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: UserId,
    roles: &[String],
) -> Result<(), ApiError> {
    sqlx::query!("DELETE FROM user_roles WHERE user_id = $1", user_id.0)
        .execute(&mut **tx)
        .await?;

    if roles.is_empty() {
        return Ok(());
    }

    sqlx::query!(
        "INSERT INTO user_roles (user_id, role_code) SELECT $1, unnest($2::text[])",
        user_id.0,
        roles,
    )
    .execute(&mut **tx)
    .await
    .map_err(|e| {
        if e.as_database_error().and_then(|d| d.constraint()).is_some() {
            ApiError::validation(
                "指定的角色不存在",
                [("roles".to_owned(), "含未知的角色代碼".to_owned())].into(),
            )
        } else {
            ApiError::from(e)
        }
    })?;

    Ok(())
}

#[utoipa::path(
    patch, path = "/api/v1/users/{id}",
    params(("id" = i64, Path)),
    request_body = UpdateUserRequest,
    responses((status = 204), (status = 404)),
    tag = "admin",
)]
pub async fn update_user(
    State(state): State<AppState>,
    CurrentUser(actor): CurrentUser,
    _csrf: CsrfGuard,
    ApiPath(id): ApiPath<i64>,
    ApiJson(body): ApiJson<UpdateUserRequest>,
) -> Result<StatusCode, ApiError> {
    require(&actor.permissions, Permission::UserManage)?;

    // 停用自己會把自己鎖在系統外，而且可能是最後一個管理員
    if id == actor.id.0 && body.is_active == Some(false) {
        return Err(ApiError::validation(
            "不可停用自己的帳號",
            [("is_active".to_owned(), "不可停用自己".to_owned())].into(),
        ));
    }

    // 拔掉自己的角色等於拔掉 user.manage，與停用自己同一個失效模式；
    // 唯一的管理員這麼做之後只能靠容器指令 create-admin 救援
    if id == actor.id.0 && body.roles.as_deref().is_some_and(|r| r.is_empty()) {
        return Err(ApiError::validation(
            "不可清空自己的角色",
            [("roles".to_owned(), "至少保留一個角色".to_owned())].into(),
        ));
    }

    let hash = match body.password.as_deref() {
        None => None,
        Some(pw) if pw.chars().count() < MIN_PASSWORD_LEN => {
            return Err(ApiError::validation(
                format!("密碼至少需要 {MIN_PASSWORD_LEN} 個字元"),
                [(
                    "password".to_owned(),
                    format!("至少 {MIN_PASSWORD_LEN} 字元"),
                )]
                .into(),
            ))
        }
        Some(pw) => Some(auth::hash_password_blocking(pw.to_owned()).await?),
    };

    let mut tx = state.pool.begin().await?;

    let affected = sqlx::query!(
        r#"
        UPDATE users SET
          display_name  = COALESCE($2, display_name),
          is_active     = COALESCE($3, is_active),
          password_hash = COALESCE($4, password_hash)
        WHERE id = $1
        "#,
        id,
        body.display_name,
        body.is_active,
        hash,
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();

    if affected == 0 {
        return Err(ApiError::from(ServiceError::NotFound { entity: "帳號" }));
    }

    if let Some(roles) = &body.roles {
        assign_roles(&mut tx, UserId(id), roles).await?;
    }

    // 停用或改密碼時撤銷既有 session，否則對方仍能繼續操作
    if body.is_active == Some(false) || hash.is_some() {
        sqlx::query!("DELETE FROM sessions WHERE user_id = $1", id)
            .execute(&mut *tx)
            .await?;
    }

    db::audit::record(
        &mut *tx,
        Some(actor.id),
        "user",
        id,
        "update",
        Some(serde_json::json!({
            "is_active": body.is_active,
            "roles": body.roles,
            "password_changed": hash.is_some(),
        })),
    )
    .await?;

    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get, path = "/api/v1/roles",
    responses((status = 200, body = Vec<RoleSummary>)),
    tag = "admin",
)]
pub async fn list_roles(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Vec<RoleSummary>>, ApiError> {
    require(&user.permissions, Permission::UserManage)?;

    let rows = sqlx::query!(
        r#"
        SELECT r.code, r.name, r.description, r.is_system,
               COALESCE(array_agg(rp.permission) FILTER (WHERE rp.permission IS NOT NULL),
                        ARRAY[]::text[]) AS "permissions!: Vec<String>"
        FROM roles r
        LEFT JOIN role_permissions rp ON rp.role_code = r.code
        GROUP BY r.code
        ORDER BY r.code
        "#
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(
        rows.into_iter()
            .map(|r| RoleSummary {
                code: r.code,
                name: r.name,
                description: r.description,
                is_system: r.is_system,
                permissions: r.permissions,
            })
            .collect(),
    ))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RoleSummary {
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    /// 系統預設角色不可刪除，但權限可調整。
    pub is_system: bool,
    pub permissions: Vec<String>,
}

/// 系統定義的全部權限，供角色設定頁列出可勾選項目。
#[utoipa::path(
    get, path = "/api/v1/permissions",
    responses((status = 200, body = Vec<String>)),
    tag = "admin",
)]
pub async fn list_permissions(
    CurrentUser(user): CurrentUser,
) -> Result<Json<Vec<String>>, ApiError> {
    require(&user.permissions, Permission::UserManage)?;
    Ok(Json(
        Permission::ALL
            .iter()
            .map(|p| p.as_str().to_owned())
            .collect(),
    ))
}

// ==================== 公司資料 ====================

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CompanyProfileBody {
    pub name: String,
    pub tax_id: Option<String>,
    pub phone: Option<String>,
    pub fax: Option<String>,
    pub email: Option<String>,
    pub address: Option<String>,
    /// Logo 的 SVG 原始碼。網頁與報價單 PDF 共用同一份。
    pub logo_svg: Option<String>,
    /// 報價單條款，逐字印在報價單末頁。
    pub quote_terms: String,
}

impl From<CompanyProfile> for CompanyProfileBody {
    fn from(p: CompanyProfile) -> Self {
        Self {
            name: p.name,
            tax_id: p.tax_id,
            phone: p.phone,
            fax: p.fax,
            email: p.email,
            address: p.address,
            logo_svg: p.logo_svg,
            quote_terms: p.quote_terms,
        }
    }
}

/// Logo SVG 的大小上限。實際的 logo 不到 1 KiB，64 KiB 已經很寬鬆。
const MAX_LOGO_BYTES: usize = 64 * 1024;

/// 不允許出現在 Logo SVG 裡的構造。
///
/// 畫面上的 sink 已改成 `<img src=data:...>`（SVG 在 `<img>` 內無腳本能力），
/// 這裡是第二道：不讓危險內容進得了資料庫。同一份 SVG 還會交給
/// 報價單 PDF 的 usvg 解析，禁掉 `href` / `xlink:` 也一併關掉
/// 「外部參照把本機檔案讀進圖片」這條不確定的路。
const FORBIDDEN: [&str; 8] = [
    "<script",
    "javascript:",
    "<foreignobject",
    "<iframe",
    "<use",
    "<image",
    "href",
    "xlink:",
];

/// 檢查 Logo SVG。
fn validate_logo_svg(svg: &str) -> Result<(), ApiError> {
    let reject = |detail: String| {
        Err(ApiError::validation(
            "Logo SVG 不被接受",
            [("logo_svg".to_owned(), detail)].into_iter().collect(),
        ))
    };

    if svg.trim().is_empty() {
        return Ok(()); // 空字串代表不設 logo
    }
    if svg.len() > MAX_LOGO_BYTES {
        return reject(format!("超過上限 {MAX_LOGO_BYTES} 位元組"));
    }

    let lower = svg.to_ascii_lowercase();
    if !lower.contains("<svg") {
        return reject("需為 SVG 原始碼，應包含 <svg> 標籤".to_owned());
    }
    if let Some(bad) = FORBIDDEN.iter().find(|b| lower.contains(**b)) {
        return reject(format!("不可包含 {bad}"));
    }
    if has_event_attribute(&lower) {
        return reject("不可包含 on* 事件屬性".to_owned());
    }
    Ok(())
}

/// 偵測 `on<字母>…=` 形式的事件屬性，例如 `onload=`、`onerror =`。
///
/// 只認「屬性位置」的 on：前一個字元必須是空白或引號，否則
/// `stroke-linecap="butt"` 之類含 on 的正常內容會被誤判。
fn has_event_attribute(lower: &str) -> bool {
    let bytes = lower.as_bytes();
    for (index, window) in bytes.windows(2).enumerate() {
        if window != b"on" {
            continue;
        }
        let at_attribute_position = match index.checked_sub(1) {
            None => false,
            Some(prev) => bytes[prev].is_ascii_whitespace() || matches!(bytes[prev], b'"' | b'\''),
        };
        if !at_attribute_position {
            continue;
        }
        let mut cursor = index + 2;
        let name_start = cursor;
        while cursor < bytes.len() && bytes[cursor].is_ascii_alphabetic() {
            cursor += 1;
        }
        if cursor == name_start {
            continue; // 只有 "on" 本身，不是事件名
        }
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if bytes.get(cursor) == Some(&b'=') {
            return true;
        }
    }
    false
}

/// 公司資料為列印報價單所需，任何登入者皆可讀取；修改才需 `settings.manage`。
#[utoipa::path(
    get, path = "/api/v1/company",
    responses((status = 200, body = CompanyProfileBody)),
    tag = "admin",
)]
pub async fn get_company(
    State(state): State<AppState>,
    CurrentUser(_user): CurrentUser,
) -> Result<Json<CompanyProfileBody>, ApiError> {
    Ok(Json(db::company::get(&state.pool).await?.into()))
}

#[utoipa::path(
    put, path = "/api/v1/company",
    request_body = CompanyProfileBody,
    responses((status = 204), (status = 403)),
    tag = "admin",
)]
pub async fn update_company(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiJson(body): ApiJson<CompanyProfileBody>,
) -> Result<StatusCode, ApiError> {
    require(&user.permissions, Permission::SettingsManage)?;

    if let Some(svg) = body.logo_svg.as_deref() {
        validate_logo_svg(svg)?;
    }

    let profile = CompanyProfile {
        name: body.name,
        tax_id: body.tax_id,
        phone: body.phone,
        fax: body.fax,
        email: body.email,
        address: body.address,
        logo_svg: body.logo_svg,
        quote_terms: body.quote_terms,
    };

    db::company::update(&state.pool, &profile).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 專案實際採用的 logo，必須通過驗證——
    /// 擋得住攻擊但擋掉自己的 logo 等於沒修好。
    const REAL_LOGO: &str = include_str!("../../../../../assets/brand/logo-elbow.svg");

    #[test]
    fn accepts_the_project_logo() {
        assert!(validate_logo_svg(REAL_LOGO).is_ok());
    }

    #[test]
    fn accepts_an_empty_value_as_no_logo() {
        assert!(validate_logo_svg("").is_ok());
        assert!(validate_logo_svg("   ").is_ok());
    }

    #[test]
    fn rejects_script_bearing_payloads() {
        for payload in [
            r#"<img src=x onerror="fetch('/api/v1/users')">"#,
            r#"<svg xmlns="http://www.w3.org/2000/svg" onload="alert(1)"></svg>"#,
            r#"<svg><script>alert(1)</script></svg>"#,
            r#"<svg><a href="javascript:alert(1)">x</a></svg>"#,
            r#"<svg><use xlink:href="/etc/passwd"/></svg>"#,
            r#"<svg><foreignObject><body>x</body></foreignObject></svg>"#,
        ] {
            assert!(
                validate_logo_svg(payload).is_err(),
                "應被拒絕但通過了：{payload}"
            );
        }
    }

    #[test]
    fn rejects_content_that_is_not_svg() {
        assert!(validate_logo_svg("<p>我只是一段文字</p>").is_err());
    }

    #[test]
    fn rejects_oversized_input() {
        let huge = format!("<svg>{}</svg>", "a".repeat(MAX_LOGO_BYTES));
        assert!(validate_logo_svg(&huge).is_err());
    }

    /// 屬性名含 on 的正常 SVG 不該被誤判，否則驗證會擋掉合法的 logo。
    #[test]
    fn does_not_mistake_ordinary_attributes_for_event_handlers() {
        assert!(!has_event_attribute(r#"<path stroke-linecap="butt"/>"#));
        assert!(!has_event_attribute(r#"<path d="M 20 80 H 40"/>"#));
        assert!(!has_event_attribute("<svg><title>冷氣配管</title></svg>"));
        assert!(has_event_attribute(r#"<svg onload ="x">"#));
    }
}
