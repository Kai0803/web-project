//! 對外的資料傳輸型別。
//!
//! # 金額一律為字串
//!
//! `rust_decimal` 預設序列化為 JSON number，前端 `JSON.parse` 會得到
//! IEEE754 double，整套 `NUMERIC` / `Decimal` 建立的精度保證會在邊界上蒸發。
//! 報價單正是前端會即時顯示金額的畫面，`0.1 + 0.2` 那類問題會直接出現在
//! 使用者眼前的合計欄，而且與後端算出來的值不一致。
//!
//! 因此所有金額與數量都以 [`decimal_str`] 序列化為字串（並正規化尾零），
//! utoipa 的 schema 也標為 `String`，產生的 TS 型別才會是 `string` 而非 `number`。
//!
//! # 請求型別一律 `deny_unknown_fields`
//!
//! serde 預設忽略不認識的欄位，所以欄位名拼錯會靜默生效為「沒帶那個欄位」。
//! 在這套系統裡那是會造成資料損失的失效模式：若前端把 `unit_cost` 拼成
//! `unitCost`，成本就會無聲變成 NULL 而不是收到一個錯誤。拒絕未知欄位
//! 讓契約漂移在第一個請求就以 400 現形。

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use acms_domain::{Contract, Customer, Project, ProjectListRow, Quotation, QuotationItem, User};

// ==================== 使用者 ====================

#[derive(Debug, Serialize, ToSchema)]
pub struct MeResponse {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    /// 此使用者的權限字串，供前端決定顯示哪些操作。
    /// 前端隱藏只是體驗，實際管制在後端。
    pub permissions: Vec<String>,
}

impl From<&User> for MeResponse {
    fn from(u: &User) -> Self {
        Self {
            id: u.id.0,
            username: u.username.clone(),
            display_name: u.display_name.clone(),
            permissions: u
                .permissions
                .iter()
                .map(|p| p.as_str().to_owned())
                .collect(),
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

// ==================== 客戶 ====================

#[derive(Debug, Serialize, ToSchema)]
pub struct CustomerResponse {
    pub id: i64,
    pub public_id: uuid::Uuid,
    pub code: String,
    pub name: String,
    pub tax_id: Option<String>,
    pub phone: Option<String>,
    pub address: Option<String>,
    pub note: Option<String>,
    pub is_active: bool,
}

impl From<Customer> for CustomerResponse {
    fn from(c: Customer) -> Self {
        Self {
            id: c.id.0,
            public_id: c.public_id,
            code: c.code,
            name: c.name,
            tax_id: c.tax_id,
            phone: c.phone,
            address: c.address,
            note: c.note,
            is_active: c.is_active,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateCustomerRequest {
    pub name: String,
    pub tax_id: Option<String>,
    pub phone: Option<String>,
    pub address: Option<String>,
    pub note: Option<String>,
}

/// 客戶的部分更新。
///
/// 欄位缺席與明確給 `null` 是兩件事：缺席代表不動，`null` 代表清空。
/// 兩者若都收斂成 `None`，`PATCH {"name":"新名字"}` 會把統編、電話、
/// 地址、備註一併寫成 NULL，而呼叫端從 200 回應裡看不出自己刪了四個欄位。
/// 統編遺失對這個業務是實質損害——報價單與發票都要用。
///
/// `db::customer::CustomerPatch` 本來就是三態語意，這裡只是讓 DTO
/// 不要把三態壓成兩態。
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateCustomerRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    pub tax_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    pub phone: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    pub address: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    pub note: Option<Option<String>>,
}

/// 供 `Option<Option<T>>` 區分「欄位缺席」與「明確給 null」。
///
/// serde 對 `Option<Option<T>>` 的預設行為會把兩者都收成 `None`：
/// 外層的 `deserialize_option` 碰到 `null` 就走 `visit_none`。
/// `deserialize_with` 只在欄位實際出現時才被呼叫，所以這裡一律包一層
/// `Some`；缺席的情況由 `#[serde(default)]` 給 `None`。
fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

// ==================== 專案 ====================

#[derive(Debug, Serialize, ToSchema)]
pub struct ProjectResponse {
    pub id: i64,
    pub public_id: uuid::Uuid,
    pub code: String,
    pub customer_id: i64,
    pub name: String,
    pub site_address: Option<String>,
    pub status: String,
    pub note: Option<String>,
}

impl From<Project> for ProjectResponse {
    fn from(p: Project) -> Self {
        Self {
            id: p.id.0,
            public_id: p.public_id,
            code: p.code,
            customer_id: p.customer_id.0,
            name: p.name,
            site_address: p.site_address,
            status: p.status.as_str().to_owned(),
            note: p.note,
        }
    }
}

/// 列表列。客戶名稱由單一 JOIN 取得，不由前端再逐筆查詢。
#[derive(Debug, Serialize, ToSchema)]
pub struct ProjectListItem {
    #[serde(flatten)]
    pub project: ProjectResponse,
    pub customer_name: String,
}

impl From<ProjectListRow> for ProjectListItem {
    fn from(r: ProjectListRow) -> Self {
        Self {
            project: r.project.into(),
            customer_name: r.customer_name,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateProjectRequest {
    pub customer_id: i64,
    pub name: String,
    pub site_address: Option<String>,
    pub note: Option<String>,
}

// ==================== 報價單 ====================

#[derive(Debug, Serialize, ToSchema)]
pub struct QuotationItemResponse {
    pub id: i64,
    pub line_no: i32,
    pub name: String,
    pub spec: Option<String>,
    pub unit: String,
    #[serde(with = "decimal_str")]
    #[schema(value_type = String, example = "120.0000")]
    pub quantity: Decimal,
    #[serde(with = "decimal_str")]
    #[schema(value_type = String, example = "180.0000")]
    pub unit_price: Decimal,
    /// 成本。**無 `quotation.cost.read` 權限時此欄位不存在**，
    /// 不是回傳 null 也不是由前端隱藏——資料根本不離開伺服器。
    #[serde(skip_serializing_if = "Option::is_none", with = "decimal_opt_str")]
    #[schema(value_type = Option<String>)]
    pub unit_cost: Option<Decimal>,
    #[serde(with = "decimal_str")]
    #[schema(value_type = String, example = "21600")]
    pub amount: Decimal,
    pub note: Option<String>,
}

impl QuotationItemResponse {
    /// `can_view_cost` 為 false 時剝除成本欄位。
    pub fn from_item(item: QuotationItem, can_view_cost: bool) -> Self {
        Self {
            id: item.id.0,
            line_no: item.line_no,
            name: item.name,
            spec: item.spec,
            unit: item.unit,
            quantity: item.quantity,
            unit_price: item.unit_price,
            unit_cost: if can_view_cost { item.unit_cost } else { None },
            amount: item.amount,
            note: item.note,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct QuotationResponse {
    pub id: i64,
    pub public_id: uuid::Uuid,
    pub project_id: i64,
    pub quote_no: String,
    pub version: i32,
    pub status: String,
    #[serde(with = "decimal_str")]
    #[schema(value_type = String, example = "0.0500")]
    pub tax_rate: Decimal,
    /// 稅率的顯示標籤，例如 `5%`。
    ///
    /// 由伺服器算好而非前端自行轉換——這個轉換原本在前端有一份自己的
    /// 實作，而它在小數位數為 1 時算錯（`0.1` 顯示成 `1%`）。
    /// 與「金額一律由伺服器計算」同一條原則。
    pub tax_percent: String,
    #[serde(with = "decimal_str")]
    #[schema(value_type = String, example = "75000")]
    pub subtotal: Decimal,
    #[serde(with = "decimal_str")]
    #[schema(value_type = String, example = "3750")]
    pub tax_amount: Decimal,
    #[serde(with = "decimal_str")]
    #[schema(value_type = String, example = "78750")]
    pub total: Decimal,
    /// 樂觀鎖版本。更新時需帶回，不符表示他人已先行修改。
    pub lock_version: i32,
    pub note: Option<String>,
}

impl From<Quotation> for QuotationResponse {
    fn from(q: Quotation) -> Self {
        Self {
            id: q.id.0,
            public_id: q.public_id,
            project_id: q.project_id.0,
            quote_no: q.quote_no,
            version: q.version,
            status: q.status.as_str().to_owned(),
            tax_percent: acms_domain::money::tax_percent_label(q.tax_rate),
            tax_rate: q.tax_rate,
            subtotal: q.subtotal,
            tax_amount: q.tax_amount,
            total: q.total,
            lock_version: q.lock_version,
            note: q.note,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct QuotationDetailResponse {
    #[serde(flatten)]
    pub quotation: QuotationResponse,
    pub items: Vec<QuotationItemResponse>,
}

/// 明細輸入。**沒有 `amount` 欄位**——金額由伺服器依進位規則計算，
/// 呼叫端無從指定。
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LineRequest {
    /// 這一列載入時的明細 id。重寫明細時原樣送回，伺服器據此承接
    /// 呼叫端無權限檢視的欄位（目前是成本）。新增的列不帶此欄位。
    ///
    /// 不以陣列位置或 `line_no` 承接：那兩者都會隨使用者刪除、插入、
    /// 調整順序而指向別的列，而承接錯誤的人正是看不到成本的人。
    #[serde(default)]
    pub id: Option<i64>,
    pub name: String,
    pub spec: Option<String>,
    pub unit: String,
    #[serde(with = "decimal_str")]
    #[schema(value_type = String, example = "120")]
    pub quantity: Decimal,
    #[serde(with = "decimal_str")]
    #[schema(value_type = String, example = "180")]
    pub unit_price: Decimal,
    #[serde(default, with = "decimal_opt_str")]
    #[schema(value_type = Option<String>)]
    pub unit_cost: Option<Decimal>,
    pub note: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateQuotationRequest {
    pub lines: Vec<LineRequest>,
    pub note: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplaceItemsRequest {
    pub lines: Vec<LineRequest>,
}

// ==================== 合約 ====================

#[derive(Debug, Serialize, ToSchema)]
pub struct ContractResponse {
    pub id: i64,
    pub public_id: uuid::Uuid,
    pub project_id: i64,
    pub quotation_id: i64,
    pub contract_no: String,
    pub kind: String,
    #[serde(with = "decimal_str")]
    #[schema(value_type = String)]
    pub total: Decimal,
}

impl From<Contract> for ContractResponse {
    fn from(c: Contract) -> Self {
        Self {
            id: c.id.0,
            public_id: c.public_id,
            project_id: c.project_id.0,
            quotation_id: c.quotation_id.0,
            contract_no: c.contract_no,
            kind: c.kind.as_str().to_owned(),
            total: c.total,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateContractRequest {
    /// ISO 8601 日期，例如 `2026-10-01`。
    pub signed_date: Option<String>,
}

// ==================== 共用 ====================

#[derive(Debug, Serialize, ToSchema)]
pub struct PageResponse<T> {
    pub items: Vec<T>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PageQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// `Decimal` 的字串序列化，輸出前先正規化。
///
/// 資料庫欄位是 `NUMERIC(14,4)`，讀回來的值帶著四位小數——
/// 未稅總額 66600 會變成 `"66600.0000"`。依進位規則這些金額本來就是整數元，
/// 多出來的零只會讓前端顯示成 `66600.0000`，或誘使前端自己去截字串。
///
/// `normalize()` 去掉尾端的零，得到該數值的標準表示：
/// `66600.0000` → `66600`，`3.3300` → `3.33`。數值本身不變。
pub(crate) mod decimal_str {
    use rust_decimal::Decimal;
    use serde::{Deserialize, Deserializer, Serializer};
    use std::str::FromStr;

    pub fn serialize<S: Serializer>(value: &Decimal, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.normalize().to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Decimal, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Decimal::from_str(&raw).map_err(serde::de::Error::custom)
    }
}

/// `Option<Decimal>` 的字串序列化。同樣正規化。
///
/// `serde(with)` 對 `Option<T>` 需要自己的模組，不能沿用上面那個。
mod decimal_opt_str {
    use rust_decimal::Decimal;
    use serde::{Deserialize, Deserializer, Serializer};
    use std::str::FromStr;

    pub fn serialize<S: Serializer>(
        value: &Option<Decimal>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(d) => serializer.serialize_str(&d.normalize().to_string()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Decimal>, D::Error> {
        let raw: Option<String> = Option::deserialize(deserializer)?;
        raw.map(|s| Decimal::from_str(&s).map_err(serde::de::Error::custom))
            .transpose()
    }
}
