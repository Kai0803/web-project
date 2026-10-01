//! 合約與工項存取。

use acms_domain::{Contract, ContractId, ContractKind, ProjectId, QuotationId};
use rust_decimal::Decimal;
use sqlx::PgExecutor;

#[derive(Debug, Clone)]
pub struct NewContract {
    pub project_id: ProjectId,
    pub quotation_id: QuotationId,
    pub contract_no: String,
    pub kind: ContractKind,
    pub parent_contract_id: Option<ContractId>,
    pub signed_date: Option<time::Date>,
    pub total: Decimal,
}

/// 工項。由核准的報價明細於轉合約時實體化。
#[derive(Debug, Clone)]
pub struct NewWorkItem {
    pub seq: i32,
    pub name: String,
    pub spec: Option<String>,
    pub unit: String,
    pub quantity: Decimal,
    pub unit_price: Decimal,
    pub unit_cost: Option<Decimal>,
    pub amount: Decimal,
}

fn parse_kind(raw: &str) -> Result<ContractKind, sqlx::Error> {
    match raw {
        "original" => Ok(ContractKind::Original),
        "variation" => Ok(ContractKind::Variation),
        other => Err(sqlx::Error::Decode(
            format!("未知的合約類型「{other}」").into(),
        )),
    }
}

pub async fn insert(ex: impl PgExecutor<'_>, new: NewContract) -> sqlx::Result<Contract> {
    let r = sqlx::query!(
        r#"
        INSERT INTO contracts (project_id, quotation_id, contract_no, kind,
                               parent_contract_id, signed_date, total)
        VALUES ($1, $2, $3, ($4::text)::contract_kind, $5, $6, $7)
        RETURNING id, public_id, project_id, quotation_id, contract_no,
                  kind::text AS "kind!", parent_contract_id, signed_date, total,
                  created_at, updated_at
        "#,
        new.project_id.0,
        new.quotation_id.0,
        new.contract_no,
        new.kind.as_str(),
        new.parent_contract_id.map(|c| c.0),
        new.signed_date,
        new.total,
    )
    .fetch_one(ex)
    .await?;

    Ok(Contract {
        id: ContractId(r.id),
        public_id: r.public_id,
        project_id: ProjectId(r.project_id),
        quotation_id: QuotationId(r.quotation_id),
        contract_no: r.contract_no,
        kind: parse_kind(&r.kind)?,
        parent_contract_id: r.parent_contract_id.map(ContractId),
        signed_date: r.signed_date,
        total: r.total,
        created_at: r.created_at,
        updated_at: r.updated_at,
    })
}

pub async fn find(ex: impl PgExecutor<'_>, id: ContractId) -> sqlx::Result<Option<Contract>> {
    let row = sqlx::query!(
        r#"
        SELECT id, public_id, project_id, quotation_id, contract_no,
               kind::text AS "kind!", parent_contract_id, signed_date, total,
               created_at, updated_at
        FROM contracts WHERE id = $1
        "#,
        id.0,
    )
    .fetch_optional(ex)
    .await?;

    row.map(|r| {
        Ok(Contract {
            id: ContractId(r.id),
            public_id: r.public_id,
            project_id: ProjectId(r.project_id),
            quotation_id: QuotationId(r.quotation_id),
            contract_no: r.contract_no,
            kind: parse_kind(&r.kind)?,
            parent_contract_id: r.parent_contract_id.map(ContractId),
            signed_date: r.signed_date,
            total: r.total,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
    })
    .transpose()
}

pub async fn find_by_quotation(
    ex: impl PgExecutor<'_>,
    quotation_id: QuotationId,
) -> sqlx::Result<Option<ContractId>> {
    sqlx::query_scalar!(
        "SELECT id FROM contracts WHERE quotation_id = $1",
        quotation_id.0,
    )
    .fetch_optional(ex)
    .await
    .map(|opt| opt.map(ContractId))
}

/// 同一 `quote_no` 的任一版本是否已轉合約。
///
/// 改版會把同單號的**其他所有版本**一併標成 superseded，所以檢查範圍
/// 必須是整個 quote_no，不能只看來源版本：從一個早已被取代的舊版改版，
/// 會把另一個已簽約的版本變成「已被取代」，合約就指向一份被取代的報價。
/// 檢查的範圍與動作的範圍必須一致。
pub async fn find_by_quote_no(
    ex: impl PgExecutor<'_>,
    quote_no: &str,
) -> sqlx::Result<Option<ContractId>> {
    sqlx::query_scalar!(
        r#"
        SELECT c.id
        FROM contracts c
        JOIN quotations q ON q.id = c.quotation_id
        WHERE q.quote_no = $1
        LIMIT 1
        "#,
        quote_no,
    )
    .fetch_optional(ex)
    .await
    .map(|opt| opt.map(ContractId))
}

/// 以單次多列 INSERT 寫入工項，與報價明細相同的理由。
pub async fn insert_work_items(
    ex: impl PgExecutor<'_>,
    contract_id: ContractId,
    items: &[NewWorkItem],
) -> sqlx::Result<u64> {
    if items.is_empty() {
        return Ok(0);
    }

    let seqs: Vec<i32> = items.iter().map(|i| i.seq).collect();
    let names: Vec<String> = items.iter().map(|i| i.name.clone()).collect();
    let specs: Vec<Option<String>> = items.iter().map(|i| i.spec.clone()).collect();
    let units: Vec<String> = items.iter().map(|i| i.unit.clone()).collect();
    let quantities: Vec<Decimal> = items.iter().map(|i| i.quantity).collect();
    let unit_prices: Vec<Decimal> = items.iter().map(|i| i.unit_price).collect();
    let unit_costs: Vec<Option<Decimal>> = items.iter().map(|i| i.unit_cost).collect();
    let amounts: Vec<Decimal> = items.iter().map(|i| i.amount).collect();

    sqlx::query!(
        r#"
        INSERT INTO work_items
          (contract_id, seq, name, spec, unit, quantity, unit_price, unit_cost, amount)
        SELECT $1, * FROM UNNEST(
          $2::int[], $3::text[], $4::text[], $5::text[],
          $6::numeric[], $7::numeric[], $8::numeric[], $9::numeric[]
        )
        "#,
        contract_id.0,
        &seqs,
        &names,
        &specs as &[Option<String>],
        &units,
        &quantities,
        &unit_prices,
        &unit_costs as &[Option<Decimal>],
        &amounts,
    )
    .execute(ex)
    .await
    .map(|r| r.rows_affected())
}

pub async fn count_work_items(
    ex: impl PgExecutor<'_>,
    contract_id: ContractId,
) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM work_items WHERE contract_id = $1"#,
        contract_id.0,
    )
    .fetch_one(ex)
    .await
}
