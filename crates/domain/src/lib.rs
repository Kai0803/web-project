//! 空調配管工程管理系統的領域邏輯。
//!
//! 這個 crate 不依賴任何框架或資料庫函式庫——`Cargo.toml` 裡沒有 sqlx、
//! axum、serde，因此寫下 `use sqlx::...` 會直接編譯失敗。所有規則都是純函式，
//! 不需要資料庫即可測試。
//!
//! # 模組
//!
//! - [`entities`] — 領域實體。`db` 層一律回傳這些型別
//! - [`money`] — 金額計算與進位。全專案唯一的進位入口
//! - [`quotation`] — 報價單狀態機與可變性規則
//! - [`authz`] — 權限判斷
//! - [`ids`] — 領域識別字 newtype

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod authz;
pub mod entities;
pub mod ids;
pub mod money;
pub mod quotation;

pub use authz::{Denied, Permission, Permissions};
pub use entities::{
    CompanyProfile, Contract, ContractKind, Customer, CustomerContact, NewQuotationItem, Project,
    ProjectListRow, ProjectStatus, Quotation, QuotationItem, User,
};
pub use ids::{
    ContractId, CustomerId, ProjectId, QuotationId, QuotationItemId, UserId, WorkItemId,
};
pub use money::{MoneyError, Totals};
pub use quotation::{QuotationError, QuotationStatus};
