//! 使用者與權限查詢。

use acms_domain::{CustomerId, Permissions, UnknownUserKind, User, UserId, UserKind};
use sqlx::PgExecutor;

/// 登入驗證用。密碼雜湊不進領域型別 [`User`]——
/// 它只該存在於認證流程，不該跟著使用者物件在系統裡流動。
#[derive(Debug, Clone)]
pub struct Credentials {
    pub user_id: UserId,
    pub password_hash: String,
    pub is_active: bool,
}

pub async fn credentials_for(
    ex: impl PgExecutor<'_>,
    username: &str,
) -> sqlx::Result<Option<Credentials>> {
    sqlx::query!(
        "SELECT id, password_hash, is_active FROM users WHERE username = $1",
        username,
    )
    .fetch_optional(ex)
    .await
    .map(|opt| {
        opt.map(|r| Credentials {
            user_id: UserId(r.id),
            password_hash: r.password_hash,
            is_active: r.is_active,
        })
    })
}

/// 載入使用者及其權限。
///
/// 權限為所有角色的聯集，以單一查詢的 `array_agg` 取得，
/// 不在呼叫端迴圈內逐個角色查詢。重複的權限由 [`Permissions`] 的集合語意收斂。
pub async fn load(ex: impl PgExecutor<'_>, id: UserId) -> sqlx::Result<Option<User>> {
    let row = sqlx::query!(
        r#"
        SELECT u.id, u.kind::text AS "kind!", u.username, u.display_name,
               u.is_active, u.customer_id,
               COALESCE(
                 array_agg(rp.permission) FILTER (WHERE rp.permission IS NOT NULL),
                 ARRAY[]::text[]
               ) AS "permissions!: Vec<String>"
        FROM users u
        LEFT JOIN user_roles ur ON ur.user_id = u.id
        LEFT JOIN role_permissions rp ON rp.role_code = ur.role_code
        WHERE u.id = $1
        GROUP BY u.id
        "#,
        id.0,
    )
    .fetch_optional(ex)
    .await?;

    let Some(r) = row else { return Ok(None) };

    // 無法解讀的 kind 一律當成錯誤，不猜測。猜 staff 會給客戶帳號
    // 全系統的可見範圍，猜 customer_contact 會把同仁鎖在外面。
    let kind: UserKind = r.kind.parse().map_err(|e: UnknownUserKind| {
        sqlx::Error::Decode(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            e.to_string(),
        )))
    })?;

    Ok(Some(User {
        id: UserId(r.id),
        kind,
        username: r.username,
        display_name: r.display_name,
        is_active: r.is_active,
        customer_id: r.customer_id.map(CustomerId),
        permissions: Permissions::from_strings(r.permissions.iter().map(String::as_str)),
    }))
}
