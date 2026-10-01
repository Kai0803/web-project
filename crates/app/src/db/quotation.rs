//! 報價單與明細存取。

use acms_domain::{
    NewQuotationItem, ProjectId, Quotation, QuotationId, QuotationItem, QuotationItemId,
    QuotationStatus, Totals, UserId,
};
use rust_decimal::Decimal;
use sqlx::PgExecutor;

use super::Page;

fn parse_status(raw: &str) -> Result<QuotationStatus, sqlx::Error> {
    raw.parse()
        .map_err(|e: acms_domain::QuotationError| sqlx::Error::Decode(Box::new(e)))
}

#[derive(Debug, Clone)]
pub struct NewQuotation {
    pub project_id: ProjectId,
    pub quote_no: String,
    pub version: i32,
    pub tax_rate: Decimal,
    pub valid_until: Option<time::Date>,
    pub created_by: UserId,
    pub note: Option<String>,
}

/// `query!` 產生的 record 型別是匿名的，無法寫成共用函式的參數，
/// 因此以巨集共用欄位對應，避免每個查詢各抄一次而逐漸走樣。
macro_rules! to_quotation {
    ($r:expr) => {{
        let r = $r;
        Ok::<Quotation, sqlx::Error>(Quotation {
            id: QuotationId(r.id),
            public_id: r.public_id,
            project_id: ProjectId(r.project_id),
            quote_no: r.quote_no,
            version: r.version,
            status: parse_status(&r.status)?,
            tax_rate: r.tax_rate,
            valid_until: r.valid_until,
            subtotal: r.subtotal,
            tax_amount: r.tax_amount,
            total: r.total,
            lock_version: r.lock_version,
            created_by: UserId(r.created_by),
            approved_by: r.approved_by.map(UserId),
            approved_at: r.approved_at,
            note: r.note,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
    }};
}

pub async fn insert(ex: impl PgExecutor<'_>, new: NewQuotation) -> sqlx::Result<Quotation> {
    let r = sqlx::query!(
        r#"
        INSERT INTO quotations (project_id, quote_no, version, tax_rate, valid_until,
                                created_by, note)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING id, public_id, project_id, quote_no, version, status::text AS "status!",
                  tax_rate, valid_until, subtotal, tax_amount, total, lock_version,
                  created_by, approved_by, approved_at, note, created_at, updated_at
        "#,
        new.project_id.0,
        new.quote_no,
        new.version,
        new.tax_rate,
        new.valid_until,
        new.created_by.0,
        new.note,
    )
    .fetch_one(ex)
    .await?;

    to_quotation!(r)
}

pub async fn find(ex: impl PgExecutor<'_>, id: QuotationId) -> sqlx::Result<Option<Quotation>> {
    let row = sqlx::query!(
        r#"
        SELECT id, public_id, project_id, quote_no, version, status::text AS "status!",
               tax_rate, valid_until, subtotal, tax_amount, total, lock_version,
               created_by, approved_by, approved_at, note, created_at, updated_at
        FROM quotations WHERE id = $1
        "#,
        id.0,
    )
    .fetch_optional(ex)
    .await?;

    row.map(|r| to_quotation!(r)).transpose()
}

/// 同一 `quote_no` 的最新版本號。用於決定新版本的編號。
pub async fn latest_version(ex: impl PgExecutor<'_>, quote_no: &str) -> sqlx::Result<Option<i32>> {
    sqlx::query_scalar!(
        "SELECT max(version) FROM quotations WHERE quote_no = $1",
        quote_no,
    )
    .fetch_one(ex)
    .await
}

/// 某專案的報價單總數，供分頁計算頁數。
pub async fn count_for_project(
    ex: impl PgExecutor<'_>,
    project_id: ProjectId,
) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM quotations WHERE project_id = $1"#,
        project_id.0,
    )
    .fetch_one(ex)
    .await
}

pub async fn list_for_project(
    ex: impl PgExecutor<'_>,
    project_id: ProjectId,
    page: Page,
) -> sqlx::Result<Vec<Quotation>> {
    let rows = sqlx::query!(
        r#"
        SELECT id, public_id, project_id, quote_no, version, status::text AS "status!",
               tax_rate, valid_until, subtotal, tax_amount, total, lock_version,
               created_by, approved_by, approved_at, note, created_at, updated_at
        FROM quotations WHERE project_id = $1
        ORDER BY quote_no, version DESC, id DESC
        LIMIT $2 OFFSET $3
        "#,
        project_id.0,
        page.limit,
        page.offset,
    )
    .fetch_all(ex)
    .await?;

    rows.into_iter().map(|r| to_quotation!(r)).collect()
}

pub async fn items(ex: impl PgExecutor<'_>, id: QuotationId) -> sqlx::Result<Vec<QuotationItem>> {
    sqlx::query!(
        r#"
        SELECT id, quotation_id, line_no, name, spec, unit, quantity, unit_price,
               unit_cost, amount, note
        FROM quotation_items WHERE quotation_id = $1 ORDER BY line_no
        "#,
        id.0,
    )
    .fetch_all(ex)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(|r| QuotationItem {
                id: QuotationItemId(r.id),
                quotation_id: QuotationId(r.quotation_id),
                line_no: r.line_no,
                name: r.name,
                spec: r.spec,
                unit: r.unit,
                quantity: r.quantity,
                unit_price: r.unit_price,
                unit_cost: r.unit_cost,
                amount: r.amount,
                note: r.note,
            })
            .collect()
    })
}

/// 以單次多列 INSERT 寫入明細。
///
/// 逐列 execute 的話 100 列就是 100 次 round-trip（約 50–100 ms），
/// 是第一階段最貴的單一使用者操作，而成因純粹是往返次數。
/// 改用 `UNNEST` 展開陣列參數後降至一次往返。
pub async fn insert_items(
    ex: impl PgExecutor<'_>,
    quotation_id: QuotationId,
    items: &[NewQuotationItem],
) -> sqlx::Result<u64> {
    if items.is_empty() {
        return Ok(0);
    }

    let line_nos: Vec<i32> = items.iter().map(|i| i.line_no).collect();
    let names: Vec<String> = items.iter().map(|i| i.name.clone()).collect();
    let specs: Vec<Option<String>> = items.iter().map(|i| i.spec.clone()).collect();
    let units: Vec<String> = items.iter().map(|i| i.unit.clone()).collect();
    let quantities: Vec<Decimal> = items.iter().map(|i| i.quantity).collect();
    let unit_prices: Vec<Decimal> = items.iter().map(|i| i.unit_price).collect();
    let unit_costs: Vec<Option<Decimal>> = items.iter().map(|i| i.unit_cost).collect();
    let amounts: Vec<Decimal> = items.iter().map(|i| i.amount).collect();
    let notes: Vec<Option<String>> = items.iter().map(|i| i.note.clone()).collect();

    let affected = sqlx::query!(
        r#"
        INSERT INTO quotation_items
          (quotation_id, line_no, name, spec, unit, quantity, unit_price, unit_cost, amount, note)
        SELECT $1, * FROM UNNEST(
          $2::int[], $3::text[], $4::text[], $5::text[],
          $6::numeric[], $7::numeric[], $8::numeric[], $9::numeric[], $10::text[]
        )
        "#,
        quotation_id.0,
        &line_nos,
        &names,
        &specs as &[Option<String>],
        &units,
        &quantities,
        &unit_prices,
        &unit_costs as &[Option<Decimal>],
        &amounts,
        &notes as &[Option<String>],
    )
    .execute(ex)
    .await?
    .rows_affected();

    Ok(affected)
}

pub async fn delete_items(ex: impl PgExecutor<'_>, quotation_id: QuotationId) -> sqlx::Result<u64> {
    sqlx::query!(
        "DELETE FROM quotation_items WHERE quotation_id = $1",
        quotation_id.0,
    )
    .execute(ex)
    .await
    .map(|r| r.rows_affected())
}

/// 回寫表頭的三個金額。
///
/// 必須與明細變動落在同一交易內，且金額一律由 `acms_domain::money::totals`
/// 重算後傳入——SQL 裡不得有第二份 `SUM` 實作。
pub async fn update_totals(
    ex: impl PgExecutor<'_>,
    id: QuotationId,
    totals: Totals,
) -> sqlx::Result<bool> {
    let affected = sqlx::query!(
        r#"
        UPDATE quotations
           SET subtotal = $2, tax_amount = $3, total = $4, lock_version = lock_version + 1
         WHERE id = $1
        "#,
        id.0,
        totals.subtotal,
        totals.tax_amount,
        totals.total,
    )
    .execute(ex)
    .await?
    .rows_affected();

    Ok(affected == 1)
}

/// 以樂觀鎖更新狀態。`lock_version` 不符時回傳 `false`，代表他人已先行修改。
pub async fn update_status(
    ex: impl PgExecutor<'_>,
    id: QuotationId,
    status: QuotationStatus,
    approver: Option<UserId>,
    expected_lock_version: i32,
) -> sqlx::Result<bool> {
    let affected = sqlx::query!(
        r#"
        UPDATE quotations
           SET status = ($2::text)::quotation_status,
               approved_by = $3,
               approved_at = CASE WHEN $3::bigint IS NULL THEN NULL ELSE now() END,
               lock_version = lock_version + 1
         WHERE id = $1 AND lock_version = $4
        "#,
        id.0,
        status.as_str(),
        approver.map(|u| u.0),
        expected_lock_version,
    )
    .execute(ex)
    .await?
    .rows_affected();

    Ok(affected == 1)
}

/// 將同一 `quote_no` 的其他版本標記為已被取代。
///
/// 產生新版本時呼叫。已為 `superseded` 者不再更動，避免無謂的 lock_version 遞增。
pub async fn supersede_other_versions(
    ex: impl PgExecutor<'_>,
    quote_no: &str,
    keep: QuotationId,
) -> sqlx::Result<u64> {
    sqlx::query!(
        r#"
        UPDATE quotations
           SET status = 'superseded', lock_version = lock_version + 1
         WHERE quote_no = $1 AND id <> $2 AND status <> 'superseded'
        "#,
        quote_no,
        keep.0,
    )
    .execute(ex)
    .await
    .map(|r| r.rows_affected())
}
