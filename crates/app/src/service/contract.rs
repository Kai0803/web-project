//! 報價轉合約。
//!
//! 第四個跨表流程。這裡是工項（`work_items`）被實體化的地方——
//! 從此第二階段的 BOM 與第三階段的派工、工時、進度都有穩定的掛載點，
//! 不必依附於會被 superseded 的報價明細。

use acms_domain::{Contract, ContractKind, Permission, ProjectStatus, QuotationId, User};
use sqlx::PgPool;

use super::{Result, ServiceError};
use crate::db;

/// 將已核准的報價轉為合約。
///
/// 同一交易內完成：取合約編號、建立合約、由報價明細實體化工項、
/// 將專案標記為已簽約、寫稽核。
pub async fn create_from_quotation(
    pool: &PgPool,
    actor: &User,
    quotation_id: QuotationId,
    signed_date: Option<time::Date>,
) -> Result<Contract> {
    actor.permissions.require(Permission::ContractWrite)?;

    let mut tx = pool.begin().await?;

    let quotation =
        db::quotation::find(&mut *tx, quotation_id)
            .await?
            .ok_or(ServiceError::NotFound {
                entity: "報價單"
            })?;

    if !quotation.status.can_become_contract() {
        return Err(ServiceError::Rule(
            acms_domain::QuotationError::InvalidTransition {
                from: quotation.status,
                to: acms_domain::QuotationStatus::Approved,
            },
        ));
    }

    // 資料庫的 UNIQUE 也會擋，但先查能給出可讀的訊息而非約束名稱
    if db::contract::find_by_quotation(&mut *tx, quotation_id)
        .await?
        .is_some()
    {
        return Err(ServiceError::AlreadyContracted);
    }

    let year = time::OffsetDateTime::now_utc().year() as i16;
    let contract_no =
        db::numbering::next(&mut *tx, db::numbering::NumberKind::Contract, year).await?;

    let contract = db::contract::insert(
        &mut *tx,
        db::contract::NewContract {
            project_id: quotation.project_id,
            quotation_id,
            contract_no,
            kind: ContractKind::Original,
            parent_contract_id: None,
            signed_date,
            total: quotation.total,
        },
    )
    .await
    .map_err(ServiceError::from_db)?;

    // 工項：核准版本的明細在此固化為版本無關的列
    let items = db::quotation::items(&mut *tx, quotation_id).await?;
    let work_items: Vec<db::contract::NewWorkItem> = items
        .iter()
        .map(|i| db::contract::NewWorkItem {
            seq: i.line_no,
            name: i.name.clone(),
            spec: i.spec.clone(),
            unit: i.unit.clone(),
            quantity: i.quantity,
            unit_price: i.unit_price,
            unit_cost: i.unit_cost,
            amount: i.amount,
        })
        .collect();

    db::contract::insert_work_items(&mut *tx, contract.id, &work_items).await?;

    db::project::set_status(&mut *tx, quotation.project_id, ProjectStatus::Contracted).await?;

    db::audit::record(
        &mut *tx,
        Some(actor.id),
        "contract",
        contract.id.0,
        "create",
        Some(serde_json::json!({
            "contract_no": contract.contract_no,
            "from_quotation": quotation.quote_no,
            "version": quotation.version,
            "work_item_count": work_items.len(),
        })),
    )
    .await?;

    tx.commit().await?;
    Ok(contract)
}

pub async fn find(pool: &PgPool, actor: &User, id: acms_domain::ContractId) -> Result<Contract> {
    actor.permissions.require(Permission::ContractRead)?;

    db::contract::find(pool, id)
        .await?
        .ok_or(ServiceError::NotFound { entity: "合約" })
}
