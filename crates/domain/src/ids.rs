//! 領域識別字。
//!
//! 每個實體有自己的型別，避免把 `ProjectId` 傳進吃 `CustomerId` 的參數——
//! 兩者都是 `i64`，裸用原始型別時編譯器攔不住。
//!
//! 這些型別**不加任何外部 derive**（`sqlx::Type`、`serde::Serialize` 等）。
//! 孤兒規則會逼著那些 impl 寫在這個檔案裡，一加就等於把 domain 綁上框架。
//! `db` 層以 `id.0` 傳入 `i64`、以 `CustomerId(row.id)` 建構回來。

macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub i64);

        impl $name {
            #[must_use]
            pub const fn value(self) -> i64 {
                self.0
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

define_id!(/// 客戶
    CustomerId);
define_id!(/// 專案 / 案場
    ProjectId);
define_id!(/// 報價單（單一版本）
    QuotationId);
define_id!(/// 報價單明細列
    QuotationItemId);
define_id!(/// 合約
    ContractId);
define_id!(/// 工項
    WorkItemId);
define_id!(/// 使用者
    UserId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinct_entities_have_distinct_types() {
        // 這個測試的價值在於它「能編譯」本身：
        // 若把下一行改成 take_customer(ProjectId(1)) 會編譯失敗。
        fn take_customer(_: CustomerId) {}
        take_customer(CustomerId(1));
    }

    #[test]
    fn exposes_inner_value_and_displays_it() {
        let id = ProjectId(42);
        assert_eq!(id.value(), 42);
        assert_eq!(id.to_string(), "42");
    }
}
