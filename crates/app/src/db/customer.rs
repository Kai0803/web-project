//! 客戶資料存取。

use acms_domain::{Customer, CustomerId};
use sqlx::PgExecutor;

use super::Page;

/// 資料庫列。`pub(super)` 讓它不會外流成為事實上的領域模型——
/// 一旦漏出去，綁著資料庫的型別就會擴散到 service 與 api。
#[derive(Debug)]
pub(super) struct CustomerRow {
    id: i64,
    public_id: uuid::Uuid,
    code: String,
    name: String,
    tax_id: Option<String>,
    phone: Option<String>,
    address: Option<String>,
    note: Option<String>,
    is_active: bool,
    created_at: time::OffsetDateTime,
    updated_at: time::OffsetDateTime,
}

impl From<CustomerRow> for Customer {
    fn from(r: CustomerRow) -> Self {
        Self {
            id: CustomerId(r.id),
            public_id: r.public_id,
            code: r.code,
            name: r.name,
            tax_id: r.tax_id,
            phone: r.phone,
            address: r.address,
            note: r.note,
            is_active: r.is_active,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewCustomer {
    pub code: String,
    pub name: String,
    pub tax_id: Option<String>,
    pub phone: Option<String>,
    pub address: Option<String>,
    pub note: Option<String>,
}

/// 部分更新。外層 `None` 表示不動該欄位，`Some(None)` 表示清空。
#[derive(Debug, Clone, Default)]
pub struct CustomerPatch {
    pub name: Option<String>,
    pub tax_id: Option<Option<String>>,
    pub phone: Option<Option<String>>,
    pub address: Option<Option<String>>,
    pub note: Option<Option<String>>,
    pub is_active: Option<bool>,
}

pub async fn insert(ex: impl PgExecutor<'_>, new: NewCustomer) -> sqlx::Result<Customer> {
    sqlx::query_as!(
        CustomerRow,
        r#"
        INSERT INTO customers (code, name, tax_id, phone, address, note)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id, public_id, code, name, tax_id, phone, address, note,
                  is_active, created_at, updated_at
        "#,
        new.code,
        new.name,
        new.tax_id,
        new.phone,
        new.address,
        new.note,
    )
    .fetch_one(ex)
    .await
    .map(Into::into)
}

pub async fn find(ex: impl PgExecutor<'_>, id: CustomerId) -> sqlx::Result<Option<Customer>> {
    sqlx::query_as!(
        CustomerRow,
        r#"
        SELECT id, public_id, code, name, tax_id, phone, address, note,
               is_active, created_at, updated_at
        FROM customers WHERE id = $1
        "#,
        id.0,
    )
    .fetch_optional(ex)
    .await
    .map(|opt| opt.map(Into::into))
}

pub async fn find_by_code(ex: impl PgExecutor<'_>, code: &str) -> sqlx::Result<Option<Customer>> {
    sqlx::query_as!(
        CustomerRow,
        r#"
        SELECT id, public_id, code, name, tax_id, phone, address, note,
               is_active, created_at, updated_at
        FROM customers WHERE code = $1
        "#,
        code,
    )
    .fetch_optional(ex)
    .await
    .map(|opt| opt.map(Into::into))
}

/// 列表。`ORDER BY` 附 `id` 作為決勝鍵：排序欄位若可重複，
/// Postgres 不保證兩次查詢的相對順序一致，使用者翻頁會看到某筆重複、另一筆消失。
pub async fn list(ex: impl PgExecutor<'_>, page: Page) -> sqlx::Result<Vec<Customer>> {
    sqlx::query_as!(
        CustomerRow,
        r#"
        SELECT id, public_id, code, name, tax_id, phone, address, note,
               is_active, created_at, updated_at
        FROM customers
        ORDER BY code, id
        LIMIT $1 OFFSET $2
        "#,
        page.limit,
        page.offset,
    )
    .fetch_all(ex)
    .await
    .map(|rows| rows.into_iter().map(Into::into).collect())
}

pub async fn count(ex: impl PgExecutor<'_>) -> sqlx::Result<i64> {
    sqlx::query_scalar!(r#"SELECT count(*) AS "count!" FROM customers"#)
        .fetch_one(ex)
        .await
}

pub async fn update(
    ex: impl PgExecutor<'_>,
    id: CustomerId,
    patch: CustomerPatch,
) -> sqlx::Result<Option<Customer>> {
    sqlx::query_as!(
        CustomerRow,
        r#"
        UPDATE customers SET
          name      = COALESCE($2, name),
          tax_id    = CASE WHEN $3::bool THEN $4 ELSE tax_id END,
          phone     = CASE WHEN $5::bool THEN $6 ELSE phone END,
          address   = CASE WHEN $7::bool THEN $8 ELSE address END,
          note      = CASE WHEN $9::bool THEN $10 ELSE note END,
          is_active = COALESCE($11, is_active)
        WHERE id = $1
        RETURNING id, public_id, code, name, tax_id, phone, address, note,
                  is_active, created_at, updated_at
        "#,
        id.0,
        patch.name,
        patch.tax_id.is_some(),
        patch.tax_id.flatten(),
        patch.phone.is_some(),
        patch.phone.flatten(),
        patch.address.is_some(),
        patch.address.flatten(),
        patch.note.is_some(),
        patch.note.flatten(),
        patch.is_active,
    )
    .fetch_optional(ex)
    .await
    .map(|opt| opt.map(Into::into))
}
