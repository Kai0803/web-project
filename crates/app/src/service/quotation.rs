//! 報價單的 use case。
//!
//! 四個跨表流程中的三個住在這裡：建立、明細重算、改版。轉合約在 [`super::contract`]。
//! 每個流程都自行開啟交易，內部呼叫 `domain` 判規則、`db` 做 I/O。

use acms_domain::{
    money, quotation as rules, NewQuotationItem, Permission, ProjectId, Quotation, QuotationId,
    QuotationItem, QuotationStatus, User,
};
use rust_decimal::Decimal;
use sqlx::PgPool;

use super::{Result, ServiceError};
use crate::db;

/// 使用者輸入的一列明細。
///
/// **刻意不含 `amount`**：金額一律由 `domain::money::line_amount` 計算。
/// 讓呼叫端送金額進來，等於把進位規則交給前端，而那正是這套系統最不該外包的東西。
#[derive(Debug, Clone)]
pub struct LineInput {
    /// 這一列來自哪個既有明細。新增的列為 `None`。
    ///
    /// 用途是讓伺服器承接呼叫端無權限檢視的欄位（目前是成本）。
    /// 不以陣列位置或 `line_no` 為依據：那兩者都隨使用者刪除、插入、
    /// 調整順序而指向別的列。
    pub source_id: Option<acms_domain::QuotationItemId>,
    pub name: String,
    pub spec: Option<String>,
    pub unit: String,
    pub quantity: Decimal,
    pub unit_price: Decimal,
    pub unit_cost: Option<Decimal>,
    pub note: Option<String>,
}

#[derive(Debug, Clone)]
pub struct QuotationDetail {
    pub quotation: Quotation,
    pub items: Vec<QuotationItem>,
}

/// 無成本權限者送來的成本一律丟棄。
///
/// 成本的權限控管原本只做在讀取側（DTO 序列化時剝除），寫入側是敞開的。
/// 那會造成一個不需要惡意的資料損失：業務開啟報價單時 JSON 裡沒有 `unit_cost`，
/// 表單因此是空的，他改一列品名按儲存，整批成本就被寫成 NULL——
/// 而 `replace_items` 是「刪掉全部再重寫」的語意，成本事後無法回填。
/// 反向也一樣：沒有權限看成本的人不該能寫入假成本去影響毛利判斷。
///
/// 因此成本的去向由伺服器決定：有權限者照送來的值寫，無權限者
/// 一律改用來源列在資料庫中的既有值，新建時則為空。
///
/// 承接的依據是**來源明細的 id**，不是 `line_no`。`line_no` 是依送來的
/// 陣列位置重新編號的，使用者刪掉中間一列、插入一列或調整順序時，
/// 位置就不再對應原本那一列；依 `line_no` 承接會把 A 的成本接到 B 上，
/// 而且是靜默的——操作的人看不到成本，不可能發現接錯，有權限的人
/// 下次打開看到的是一組合法但錯誤的數字。
///
/// `items` 與 `lines` 由 `price_lines` 逐列產生，索引必然對齊。
fn apply_cost_policy(
    items: &mut [NewQuotationItem],
    lines: &[LineInput],
    existing: &[QuotationItem],
    actor_can_write_cost: bool,
) {
    if actor_can_write_cost {
        return;
    }
    debug_assert_eq!(items.len(), lines.len(), "items 由 lines 逐列產生");

    for (item, line) in items.iter_mut().zip(lines) {
        // 找不到來源就是沒有成本：新增的列，或 id 不屬於這張報價單。
        // 查詢範圍限定在 existing，所以送別張單的 id 也取不到值。
        item.unit_cost = line
            .source_id
            .and_then(|id| existing.iter().find(|e| e.id == id))
            .and_then(|e| e.unit_cost);
    }
}

/// 將使用者輸入轉為可寫入的明細，逐列計算金額並編號。
fn price_lines(lines: &[LineInput]) -> Result<Vec<NewQuotationItem>> {
    // 先擋列數，再逐列計算——錯誤訊息才會指向真正的原因
    if lines.len() > money::MAX_LINES {
        return Err(ServiceError::Money(money::MoneyError::TooManyLines));
    }
    lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            money::validate_unit_cost(line.unit_cost)?;
            let amount = money::line_amount(line.quantity, line.unit_price)?;
            Ok(NewQuotationItem {
                // MAX_LINES 已擋在上面，所以這個轉換不可能失敗。
                // 用 unwrap_or(i32::MAX) 會產生重複的 line_no 並撞上
                // UNIQUE (quotation_id, line_no)，把一個不可能的狀況
                // 變成一個難查的 500。
                line_no: i32::try_from(index + 1).expect("列數已由 MAX_LINES 限制"),
                name: line.name.clone(),
                spec: line.spec.clone(),
                unit: line.unit.clone(),
                quantity: line.quantity,
                unit_price: line.unit_price,
                unit_cost: line.unit_cost,
                amount,
                note: line.note.clone(),
            })
        })
        .collect()
}

/// 建立第一版報價單。
///
/// 取號、寫表頭、寫明細、算總額、寫稽核全在同一交易內——
/// 交易中途失敗時單號不該被消耗，明細也不該留下孤兒。
pub async fn create(
    pool: &PgPool,
    actor: &User,
    project_id: ProjectId,
    lines: &[LineInput],
    note: Option<String>,
) -> Result<QuotationDetail> {
    actor.permissions.require(Permission::QuotationWrite)?;

    let mut items = price_lines(lines)?;
    // 新建時沒有既有成本可承接，無權限者送來的值直接歸零
    apply_cost_policy(
        &mut items,
        lines,
        &[],
        acms_domain::authz::can_view_cost(&actor.permissions),
    );
    let year = time::OffsetDateTime::now_utc().year() as i16;

    let mut tx = pool.begin().await?;

    if db::project::find(&mut *tx, project_id).await?.is_none() {
        return Err(ServiceError::NotFound { entity: "專案" });
    }

    let quote_no =
        db::numbering::next(&mut *tx, db::numbering::NumberKind::Quotation, year).await?;
    let tax_rate = money::default_tax_rate();

    let quotation = db::quotation::insert(
        &mut *tx,
        db::quotation::NewQuotation {
            project_id,
            quote_no,
            version: 1,
            tax_rate,
            valid_until: None,
            created_by: actor.id,
            note,
        },
    )
    .await?;

    db::quotation::insert_items(&mut *tx, quotation.id, &items).await?;

    let amounts: Vec<Decimal> = items.iter().map(|i| i.amount).collect();
    let totals = money::totals(&amounts, tax_rate)?;
    db::quotation::update_totals(&mut *tx, quotation.id, totals).await?;

    db::audit::record(
        &mut *tx,
        Some(actor.id),
        "quotation",
        quotation.id.0,
        "create",
        Some(serde_json::json!({ "quote_no": quotation.quote_no, "version": 1 })),
    )
    .await?;

    let items = db::quotation::items(&mut *tx, quotation.id).await?;
    let quotation =
        db::quotation::find(&mut *tx, quotation.id)
            .await?
            .ok_or(ServiceError::NotFound {
                entity: "報價單"
            })?;

    tx.commit().await?;
    Ok(QuotationDetail { quotation, items })
}

/// 取代全部明細並重算表頭。
///
/// 表頭金額一律由 `Σ amount` 重算而非增量更新——增量更新只要漏掉一條路徑，
/// 表頭就會與明細靜默不一致，而金額是 G1 點名的敏感資料。
pub async fn replace_items(
    pool: &PgPool,
    actor: &User,
    id: QuotationId,
    lines: &[LineInput],
) -> Result<QuotationDetail> {
    actor.permissions.require(Permission::QuotationWrite)?;

    let mut new_items = price_lines(lines)?;

    let mut tx = pool.begin().await?;

    let quotation = db::quotation::find(&mut *tx, id)
        .await?
        .ok_or(ServiceError::NotFound {
            entity: "報價單"
        })?;

    // 應用層先擋，資料庫 trigger 是第二道防線
    rules::ensure_mutable(quotation.status)?;

    // 成本必須在刪除舊明細之前讀出來——刪掉就沒有來源可以承接了
    let existing = db::quotation::items(&mut *tx, id).await?;
    apply_cost_policy(
        &mut new_items,
        lines,
        &existing,
        acms_domain::authz::can_view_cost(&actor.permissions),
    );

    db::quotation::delete_items(&mut *tx, id).await?;
    db::quotation::insert_items(&mut *tx, id, &new_items).await?;

    let amounts: Vec<Decimal> = new_items.iter().map(|i| i.amount).collect();
    let totals = money::totals(&amounts, quotation.tax_rate)?;
    db::quotation::update_totals(&mut *tx, id, totals).await?;

    db::audit::record(
        &mut *tx,
        Some(actor.id),
        "quotation",
        id.0,
        "replace_items",
        Some(serde_json::json!({
            "line_count": { "from": existing.len(), "to": new_items.len() },
            "total": { "from": quotation.total.to_string(), "to": totals.total.to_string() },
            // 記筆數而非金額：成本是受權限管制的資料，不該因為寫了稽核
            // 而讓無權限者從稽核紀錄讀回來。筆數足以看出成本是否被清空。
            "lines_with_cost": {
                "from": existing.iter().filter(|i| i.unit_cost.is_some()).count(),
                "to": new_items.iter().filter(|i| i.unit_cost.is_some()).count(),
            }
        })),
    )
    .await?;

    let items = db::quotation::items(&mut *tx, id).await?;
    let quotation = db::quotation::find(&mut *tx, id)
        .await?
        .ok_or(ServiceError::NotFound {
            entity: "報價單"
        })?;

    tx.commit().await?;
    Ok(QuotationDetail { quotation, items })
}

/// 轉狀態前的額外前置條件，在同一個交易內檢查。
///
/// 存在的理由是 `submit` 需要「至少一列明細」，而那個檢查如果在交易外做，
/// 檢查與轉狀態之間就有窗口：A 按送審、檢查通過，B 在同一瞬間把明細清空
/// （狀態還是 draft，trigger 與 `ensure_mutable` 都放行），A 的轉換接著把
/// 狀態改成送審中——結果是一張 0 明細、0 元的報價單進入送審，而它可以
/// 一路核准並轉成金額為 0 的合約。這個不變量只在應用層存在，
/// 資料庫沒有對應的約束可以當第二道防線。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Precondition {
    None,
    /// 必須至少有一列明細。
    HasItems,
}

/// 共用的狀態轉換流程：檢查權限、驗證轉換、以樂觀鎖更新、寫稽核。
async fn transition(
    pool: &PgPool,
    actor: &User,
    id: QuotationId,
    to: QuotationStatus,
    permission: Permission,
    approver: Option<acms_domain::UserId>,
    precondition: Precondition,
) -> Result<Quotation> {
    actor.permissions.require(permission)?;

    let mut tx = pool.begin().await?;

    if precondition == Precondition::HasItems
        && db::quotation::items(&mut *tx, id).await?.is_empty()
    {
        return Err(ServiceError::EmptyQuotation);
    }

    let current = db::quotation::find(&mut *tx, id)
        .await?
        .ok_or(ServiceError::NotFound {
            entity: "報價單"
        })?;

    rules::validate_transition(current.status, to)?;

    let updated = db::quotation::update_status(&mut *tx, id, to, approver, current.lock_version)
        .await
        .map_err(ServiceError::from_db)?;

    if !updated {
        return Err(ServiceError::Conflict);
    }

    db::audit::record(
        &mut *tx,
        Some(actor.id),
        "quotation",
        id.0,
        "status",
        Some(serde_json::json!({
            "status": { "from": current.status.as_str(), "to": to.as_str() }
        })),
    )
    .await?;

    let quotation = db::quotation::find(&mut *tx, id)
        .await?
        .ok_or(ServiceError::NotFound {
            entity: "報價單"
        })?;

    tx.commit().await?;
    Ok(quotation)
}

/// 送審。草稿 → 送審中。
///
/// 要求至少一列明細。空白報價單是刻意允許的起始狀態（前端先建立、再加明細），
/// 但送審代表要給客戶看，而核准之後明細就不可再改、可直接轉成金額為 0 的合約，
/// 還會把專案狀態推進到「已簽約」。
pub async fn submit(pool: &PgPool, actor: &User, id: QuotationId) -> Result<Quotation> {
    transition(
        pool,
        actor,
        id,
        QuotationStatus::Pending,
        Permission::QuotationWrite,
        None,
        Precondition::HasItems,
    )
    .await
}

/// 核准。送審中 → 已核准。
///
/// 同一 `quote_no` 只能有一個核准版本，由 partial unique index 保證。
/// 併發核准兩個版本時，後到者會得到 [`ServiceError::AlreadyApproved`] 而非 500。
pub async fn approve(pool: &PgPool, actor: &User, id: QuotationId) -> Result<Quotation> {
    transition(
        pool,
        actor,
        id,
        QuotationStatus::Approved,
        Permission::QuotationApprove,
        Some(actor.id),
        Precondition::None,
    )
    .await
}

/// 駁回。送審中 → 已駁回。之後可退回草稿修改再送。
pub async fn reject(pool: &PgPool, actor: &User, id: QuotationId) -> Result<Quotation> {
    transition(
        pool,
        actor,
        id,
        QuotationStatus::Rejected,
        Permission::QuotationApprove,
        None,
        Precondition::None,
    )
    .await
}

/// 退回草稿。已駁回 → 草稿。
pub async fn reopen(pool: &PgPool, actor: &User, id: QuotationId) -> Result<Quotation> {
    transition(
        pool,
        actor,
        id,
        QuotationStatus::Draft,
        Permission::QuotationWrite,
        None,
        Precondition::None,
    )
    .await
}

/// 建立新版本。
///
/// 複製來源版本的明細成為新的草稿，並把同 `quote_no` 的其他版本標記為已取代。
/// 舊版內容不受影響——「保留舊版可查」的前提就是舊版不可變。
pub async fn new_version(
    pool: &PgPool,
    actor: &User,
    source: QuotationId,
) -> Result<QuotationDetail> {
    actor.permissions.require(Permission::QuotationWrite)?;

    let mut tx = pool.begin().await?;

    let src = db::quotation::find(&mut *tx, source)
        .await?
        .ok_or(ServiceError::NotFound {
            entity: "報價單"
        })?;

    // 已轉合約的版本不該再被取代，否則合約會指向一份「已被取代」的報價。
    // 範圍是整個 quote_no，與下方 supersede_other_versions 的範圍一致——
    // 只檢查來源版本的話，從一個舊版改版會把已簽約的手足版本標成 superseded。
    if db::contract::find_by_quote_no(&mut *tx, &src.quote_no)
        .await?
        .is_some()
    {
        return Err(ServiceError::AlreadyContracted);
    }

    let latest = db::quotation::latest_version(&mut *tx, &src.quote_no)
        .await?
        .unwrap_or(src.version);

    // 只允許從最新版本改版。從舊版改版會把較新的版本一併標成 superseded，
    // 而操作者畫面上看的是舊版的內容——他取代掉的是自己沒看到的那一版。
    if src.version != latest {
        return Err(ServiceError::NotLatestVersion { latest });
    }

    let next = rules::next_version(latest)?;

    let created = db::quotation::insert(
        &mut *tx,
        db::quotation::NewQuotation {
            project_id: src.project_id,
            quote_no: src.quote_no.clone(),
            version: next,
            tax_rate: src.tax_rate,
            valid_until: src.valid_until,
            created_by: actor.id,
            note: src.note.clone(),
        },
    )
    .await
    // 兩人同時按「建立新版本」時，後到者會撞上 UNIQUE (quote_no, version)。
    // 那是衝突，不是故障——同檔其他兩處都記得轉，這處原本漏了。
    .map_err(ServiceError::from_db)?;

    // 複製來源明細。line_no 與金額原樣沿用，不重算——
    // 重算會讓新版本在來源已核准的情況下與客戶收到的數字不同。
    let source_items = db::quotation::items(&mut *tx, source).await?;
    let copied: Vec<NewQuotationItem> = source_items
        .iter()
        .map(|i| NewQuotationItem {
            line_no: i.line_no,
            name: i.name.clone(),
            spec: i.spec.clone(),
            unit: i.unit.clone(),
            quantity: i.quantity,
            unit_price: i.unit_price,
            unit_cost: i.unit_cost,
            amount: i.amount,
            note: i.note.clone(),
        })
        .collect();

    db::quotation::insert_items(&mut *tx, created.id, &copied).await?;

    let amounts: Vec<Decimal> = copied.iter().map(|i| i.amount).collect();
    let totals = money::totals(&amounts, src.tax_rate)?;
    db::quotation::update_totals(&mut *tx, created.id, totals).await?;

    let superseded = db::quotation::supersede_other_versions(&mut *tx, &src.quote_no, created.id)
        .await
        .map_err(ServiceError::from_db)?;

    db::audit::record(
        &mut *tx,
        Some(actor.id),
        "quotation",
        created.id.0,
        "new_version",
        Some(serde_json::json!({
            "from_version": src.version,
            "to_version": next,
            "superseded_count": superseded
        })),
    )
    .await?;

    let items = db::quotation::items(&mut *tx, created.id).await?;
    let quotation =
        db::quotation::find(&mut *tx, created.id)
            .await?
            .ok_or(ServiceError::NotFound {
                entity: "報價單"
            })?;

    tx.commit().await?;
    Ok(QuotationDetail { quotation, items })
}

/// 讀取報價單與明細。單純查詢，不開交易。
pub async fn detail(pool: &PgPool, actor: &User, id: QuotationId) -> Result<QuotationDetail> {
    actor.permissions.require(Permission::QuotationRead)?;

    let quotation = db::quotation::find(pool, id)
        .await?
        .ok_or(ServiceError::NotFound {
            entity: "報價單"
        })?;
    let items = db::quotation::items(pool, id).await?;

    Ok(QuotationDetail { quotation, items })
}
