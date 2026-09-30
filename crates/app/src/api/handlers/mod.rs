//! HTTP handler。
//!
//! 每個 handler 只做三件事：解析輸入、呼叫 `service` 或 `db`、組裝回應。
//! 業務規則不寫在這裡。

pub mod admin;
pub mod auth;
pub mod contracts;
pub mod customers;
pub mod projects;
pub mod quotations;
