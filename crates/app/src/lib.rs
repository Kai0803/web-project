//! 空調配管工程管理系統的應用層。
//!
//! 以 lib 形式匯出，讓 `tests/` 下的整合測試能夠直接呼叫 `db` 層——
//! binary crate 無法被整合測試 import。
//!
//! 層次分工：
//!
//! - [`db`] — sqlx 查詢。函式收 executor，交易由上層開啟
//! - [`service`] — use case：交易邊界、編排、授權決策
//! - [`config`] — 環境變數讀取與啟動驗證
//!
//! 業務規則在 `acms-domain`，那裡沒有 I/O，測試不需要資料庫。

pub mod config;
pub mod db;
pub mod service;
