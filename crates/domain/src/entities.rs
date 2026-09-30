//! 領域實體。
//!
//! `db` 層一律回傳這些型別，不讓帶 `sqlx::FromRow` 的 row struct 外流——
//! row struct 一旦漏出去就會變成事實上的領域模型，而它綁著資料庫。
//!
//! 這裡的欄位型別刻意只用值型別（`Decimal`、`OffsetDateTime`、`Uuid`）。
//! 它們與 `sqlx` 的整合由 sqlx 那邊的 feature 提供，domain 不需要知道。

use rust_decimal::Decimal;
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use crate::ids::{ContractId, CustomerId, ProjectId, QuotationId, QuotationItemId, UserId};
use crate::quotation::QuotationStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Customer {
    pub id: CustomerId,
    pub public_id: Uuid,
    pub code: String,
    pub name: String,
    pub tax_id: Option<String>,
    pub phone: Option<String>,
    pub address: Option<String>,
    pub note: Option<String>,
    pub is_active: bool,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomerContact {
    pub id: i64,
    pub customer_id: CustomerId,
    pub name: String,
    pub title: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub is_primary: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProjectStatus {
    Lead,
    Quoting,
    Contracted,
    Closed,
    Cancelled,
}

impl ProjectStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lead => "lead",
            Self::Quoting => "quoting",
            Self::Contracted => "contracted",
            Self::Closed => "closed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("未知的專案狀態「{value}」")]
pub struct UnknownProjectStatus {
    pub value: String,
}

impl core::str::FromStr for ProjectStatus {
    type Err = UnknownProjectStatus;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "lead" => Ok(Self::Lead),
            "quoting" => Ok(Self::Quoting),
            "contracted" => Ok(Self::Contracted),
            "closed" => Ok(Self::Closed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(UnknownProjectStatus {
                value: other.to_owned(),
            }),
        }
    }
}

impl core::fmt::Display for ProjectStatus {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Lead => "潛在",
            Self::Quoting => "報價中",
            Self::Contracted => "已簽約",
            Self::Closed => "已結案",
            Self::Cancelled => "已取消",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub id: ProjectId,
    pub public_id: Uuid,
    pub code: String,
    pub customer_id: CustomerId,
    pub name: String,
    pub site_address: Option<String>,
    pub status: ProjectStatus,
    pub manager_id: Option<UserId>,
    pub expected_start: Option<Date>,
    pub expected_end: Option<Date>,
    pub note: Option<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// 列表頁用。客戶名稱以單一 JOIN 取得，不在 handler 迴圈內逐列查詢。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectListRow {
    pub project: Project,
    pub customer_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quotation {
    pub id: QuotationId,
    pub public_id: Uuid,
    pub project_id: ProjectId,
    pub quote_no: String,
    pub version: i32,
    pub status: QuotationStatus,
    pub tax_rate: Decimal,
    pub valid_until: Option<Date>,
    pub subtotal: Decimal,
    pub tax_amount: Decimal,
    pub total: Decimal,
    pub lock_version: i32,
    pub created_by: UserId,
    pub approved_by: Option<UserId>,
    pub approved_at: Option<OffsetDateTime>,
    pub note: Option<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotationItem {
    pub id: QuotationItemId,
    pub quotation_id: QuotationId,
    pub line_no: i32,
    pub name: String,
    pub spec: Option<String>,
    pub unit: String,
    pub quantity: Decimal,
    pub unit_price: Decimal,
    /// 成本。無 `quotation.cost.read` 權限者，API 的 DTO 不含此欄位。
    pub unit_cost: Option<Decimal>,
    pub amount: Decimal,
    pub note: Option<String>,
}

/// 新增或取代報價明細時的輸入。尚未有 id 與 quotation_id。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewQuotationItem {
    pub line_no: i32,
    pub name: String,
    pub spec: Option<String>,
    pub unit: String,
    pub quantity: Decimal,
    pub unit_price: Decimal,
    pub unit_cost: Option<Decimal>,
    pub amount: Decimal,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContractKind {
    Original,
    Variation,
}

impl ContractKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Original => "original",
            Self::Variation => "variation",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contract {
    pub id: ContractId,
    pub public_id: Uuid,
    pub project_id: ProjectId,
    pub quotation_id: QuotationId,
    pub contract_no: String,
    pub kind: ContractKind,
    pub parent_contract_id: Option<ContractId>,
    pub signed_date: Option<Date>,
    pub total: Decimal,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// 登入中的使用者。權限為其所有角色的聯集。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: UserId,
    pub username: String,
    pub display_name: String,
    pub is_active: bool,
    pub customer_id: Option<CustomerId>,
    pub permissions: crate::authz::Permissions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanyProfile {
    pub name: String,
    pub tax_id: Option<String>,
    pub phone: Option<String>,
    pub fax: Option<String>,
    pub email: Option<String>,
    pub address: Option<String>,
    pub logo_svg: Option<String>,
    pub quote_terms: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_status_round_trips() {
        for s in [
            ProjectStatus::Lead,
            ProjectStatus::Quoting,
            ProjectStatus::Contracted,
            ProjectStatus::Closed,
            ProjectStatus::Cancelled,
        ] {
            assert_eq!(s.as_str().parse::<ProjectStatus>(), Ok(s));
        }
        assert!("Lead".parse::<ProjectStatus>().is_err());
        assert!("".parse::<ProjectStatus>().is_err());
    }

    #[test]
    fn contract_kind_strings_match_the_database_enum() {
        assert_eq!(ContractKind::Original.as_str(), "original");
        assert_eq!(ContractKind::Variation.as_str(), "variation");
    }
}
