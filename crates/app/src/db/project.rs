//! 專案資料存取。

use acms_domain::{CustomerId, Project, ProjectId, ProjectListRow, ProjectStatus, UserId};
use sqlx::PgExecutor;

use super::Page;

#[derive(Debug, Clone)]
pub struct NewProject {
    pub code: String,
    pub customer_id: CustomerId,
    pub name: String,
    pub site_address: Option<String>,
    pub manager_id: Option<UserId>,
    pub expected_start: Option<time::Date>,
    pub expected_end: Option<time::Date>,
    pub note: Option<String>,
}

/// 狀態以 `::text` 取出再由 [`ProjectStatus`] 的 `FromStr` 還原。
///
/// 另一條路是幫 enum 加 `#[derive(sqlx::Type)]`，但孤兒規則會逼著那個 derive
/// 寫在定義型別的 crate 裡，也就是 `acms-domain`——那正是兩個 crate 要避免的事。
fn parse_status(raw: &str) -> Result<ProjectStatus, sqlx::Error> {
    raw.parse()
        .map_err(|e: acms_domain::entities::UnknownProjectStatus| sqlx::Error::Decode(Box::new(e)))
}

pub async fn insert(ex: impl PgExecutor<'_>, new: NewProject) -> sqlx::Result<Project> {
    let r = sqlx::query!(
        r#"
        INSERT INTO projects (code, customer_id, name, site_address, manager_id,
                              expected_start, expected_end, note)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        RETURNING id, public_id, code, customer_id, name, site_address,
                  status::text AS "status!", manager_id, expected_start, expected_end,
                  note, created_at, updated_at
        "#,
        new.code,
        new.customer_id.0,
        new.name,
        new.site_address,
        new.manager_id.map(|u| u.0),
        new.expected_start,
        new.expected_end,
        new.note,
    )
    .fetch_one(ex)
    .await?;

    Ok(Project {
        id: ProjectId(r.id),
        public_id: r.public_id,
        code: r.code,
        customer_id: CustomerId(r.customer_id),
        name: r.name,
        site_address: r.site_address,
        status: parse_status(&r.status)?,
        manager_id: r.manager_id.map(UserId),
        expected_start: r.expected_start,
        expected_end: r.expected_end,
        note: r.note,
        created_at: r.created_at,
        updated_at: r.updated_at,
    })
}

pub async fn find(ex: impl PgExecutor<'_>, id: ProjectId) -> sqlx::Result<Option<Project>> {
    let row = sqlx::query!(
        r#"
        SELECT id, public_id, code, customer_id, name, site_address,
               status::text AS "status!", manager_id, expected_start, expected_end,
               note, created_at, updated_at
        FROM projects WHERE id = $1
        "#,
        id.0,
    )
    .fetch_optional(ex)
    .await?;

    row.map(|r| {
        Ok(Project {
            id: ProjectId(r.id),
            public_id: r.public_id,
            code: r.code,
            customer_id: CustomerId(r.customer_id),
            name: r.name,
            site_address: r.site_address,
            status: parse_status(&r.status)?,
            manager_id: r.manager_id.map(UserId),
            expected_start: r.expected_start,
            expected_end: r.expected_end,
            note: r.note,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
    })
    .transpose()
}

/// 專案列表。客戶名稱以單一 JOIN 取得。
///
/// 若改為逐列查 `customers`，50 筆一頁就是 51 次 round-trip（約 50 ms）——
/// 仍在預算內，但完全可避免。
pub async fn list(
    ex: impl PgExecutor<'_>,
    status: Option<ProjectStatus>,
    page: Page,
) -> sqlx::Result<Vec<ProjectListRow>> {
    let rows = sqlx::query!(
        r#"
        SELECT p.id, p.public_id, p.code, p.customer_id, p.name, p.site_address,
               p.status::text AS "status!", p.manager_id, p.expected_start, p.expected_end,
               p.note, p.created_at, p.updated_at,
               c.name AS customer_name
        FROM projects p
        JOIN customers c ON c.id = p.customer_id
        WHERE $1::text IS NULL OR p.status::text = $1
        ORDER BY p.created_at DESC, p.id DESC
        LIMIT $2 OFFSET $3
        "#,
        status.map(ProjectStatus::as_str),
        page.limit,
        page.offset,
    )
    .fetch_all(ex)
    .await?;

    rows.into_iter()
        .map(|r| {
            Ok(ProjectListRow {
                project: Project {
                    id: ProjectId(r.id),
                    public_id: r.public_id,
                    code: r.code,
                    customer_id: CustomerId(r.customer_id),
                    name: r.name,
                    site_address: r.site_address,
                    status: parse_status(&r.status)?,
                    manager_id: r.manager_id.map(UserId),
                    expected_start: r.expected_start,
                    expected_end: r.expected_end,
                    note: r.note,
                    created_at: r.created_at,
                    updated_at: r.updated_at,
                },
                customer_name: r.customer_name,
            })
        })
        .collect()
}

pub async fn count(ex: impl PgExecutor<'_>, status: Option<ProjectStatus>) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"
        SELECT count(*) AS "count!" FROM projects
        WHERE $1::text IS NULL OR status::text = $1
        "#,
        status.map(ProjectStatus::as_str),
    )
    .fetch_one(ex)
    .await
}

pub async fn set_status(
    ex: impl PgExecutor<'_>,
    id: ProjectId,
    status: ProjectStatus,
) -> sqlx::Result<bool> {
    let affected = sqlx::query!(
        "UPDATE projects SET status = ($2::text)::project_status WHERE id = $1",
        id.0,
        status.as_str(),
    )
    .execute(ex)
    .await?
    .rows_affected();

    Ok(affected == 1)
}
