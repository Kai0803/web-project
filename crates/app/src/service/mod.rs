//! Use case 層：交易邊界、編排、授權決策。
//!
//! 這一層存在的理由是第一階段有四個「規則判斷 + 多次 I/O + 單一交易」的流程：
//! 報價改版、核准、轉合約、明細重算。`domain` 不能做（不碰 I/O），
//! `db` 不該做（等於把業務規則搬進 SQL 層），沒有這層就只剩 handler。
//!
//! **規則**：開啟交易或跨表即進此層；單純的單表 CRUD 允許 `api → db` 直連。

pub mod contract;
pub mod quotation;

use acms_domain::{Denied, MoneyError, QuotationError};

/// Use case 的失敗原因。
///
/// 刻意不是扁平的字串：`api` 層要據此決定 HTTP 狀態碼與對外的錯誤代碼，
/// 而呼叫端要能分辨「使用者做錯」與「系統壞掉」。
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("找不到{entity}")]
    NotFound { entity: &'static str },

    #[error(transparent)]
    Denied(#[from] Denied),

    /// 樂觀鎖衝突或狀態已被他人變更。使用者重新載入後即可重試。
    #[error("資料已被他人修改，請重新載入後再試")]
    Conflict,

    /// 違反業務規則，例如試圖核准一份草稿。
    #[error(transparent)]
    Rule(#[from] QuotationError),

    /// 同一報價單已有其他版本被核准。
    /// 狀態轉換本身合法，衝突在於「唯一核准版本」這個不變量。
    #[error("此報價單已有核准的版本，請先將該版本取代或改以新版本核准")]
    AlreadyApproved,

    /// 報價單已轉為合約，不可再變更。
    #[error("此報價單已轉為合約")]
    AlreadyContracted,

    /// 沒有明細的報價單不可送審。
    #[error("請先新增至少一列明細再送審")]
    EmptyQuotation,

    /// 只能從最新版本改版。
    ///
    /// 改版會把同單號的其他版本全部標成 superseded，所以從舊版改版
    /// 等於取代掉操作者沒看到的那一版。
    #[error("此報價單已有更新的版本（第 {latest} 版），請從最新版本建立新版本")]
    NotLatestVersion { latest: i32 },

    #[error(transparent)]
    Money(#[from] MoneyError),

    #[error("資料庫錯誤")]
    Database(#[from] sqlx::Error),
}

impl ServiceError {
    /// 把資料庫的唯一性違反轉成可讀的衝突，而不是讓使用者看到 500。
    ///
    /// 具體會觸發的情境：兩人同時核准同一報價單的不同版本時，
    /// `one_approved_version_per_quote` 會擋下後到者。
    pub(crate) fn from_db(err: sqlx::Error) -> Self {
        match err.as_database_error().and_then(|e| e.constraint()) {
            Some("one_approved_version_per_quote") => Self::AlreadyApproved,
            Some(_) => Self::Conflict,
            None => Self::Database(err),
        }
    }
}

pub type Result<T> = std::result::Result<T, ServiceError>;
