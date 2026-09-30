//! 帳號與系統設定 handler。
//!
//! 這兩組端點是系統可被交付的前提：沒有帳號管理，管理員只能下 SQL 開帳號；
//! 沒有公司資料設定，報價單印不出抬頭。

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use acms_domain::{authz::Permission, CompanyProfile, Permissions, UserId};

use crate::api::{ApiError, AppState, CsrfGuard, CurrentUser};
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
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    pub display_name: String,
    /// 角色代碼，例如 `["sales"]`。可給多個，權限取聯集。
    pub roles: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
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
    Json(body): Json<CreateUserRequest>,
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

    let hash = auth::hash_password(&body.password)?;
    let username = body.username.trim().to_owned();

    let mut tx = state.pool.begin().await?;

    let id = sqlx::query_scalar!(
        "INSERT INTO users (username, password_hash, display_name) VALUES ($1,$2,$3) RETURNING id",
        username,
        hash,
        body.display_name.trim(),
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(
        |e| match e.as_database_error().and_then(|d| d.constraint()) {
            Some("users_username_key") => ApiError::from(ServiceError::Conflict),
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
        display_name: body.display_name,
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
    Path(id): Path<i64>,
    Json(body): Json<UpdateUserRequest>,
) -> Result<StatusCode, ApiError> {
    require(&actor.permissions, Permission::UserManage)?;

    // 停用自己會把自己鎖在系統外，而且可能是最後一個管理員
    if id == actor.id.0 && body.is_active == Some(false) {
        return Err(ApiError::validation(
            "不可停用自己的帳號",
            [("is_active".to_owned(), "不可停用自己".to_owned())].into(),
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
        Some(pw) => Some(auth::hash_password(pw)?),
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
    Json(body): Json<CompanyProfileBody>,
) -> Result<StatusCode, ApiError> {
    require(&user.permissions, Permission::SettingsManage)?;

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
