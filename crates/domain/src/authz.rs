//! 權限判斷。
//!
//! 權限（[`Permission`]）是程式碼定義的固定集合，角色是資料。使用者可在
//! 系統設定自由組合角色，但不能發明新權限——權限必須有對應的檢查點，
//! 憑空新增的權限不會有任何效果。
//!
//! 新增權限是開發工作：加列舉值、加字串對照、並在實際檢查處使用它。

use std::collections::BTreeSet;

/// 系統定義的權限。字串形式對應 `role_permissions.permission` 欄位。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Permission {
    CustomerRead,
    CustomerWrite,
    ProjectRead,
    ProjectWrite,
    QuotationRead,
    QuotationWrite,
    QuotationApprove,
    /// 檢視成本與毛利。G1 指定為需依角色控管的敏感資料。
    QuotationCostRead,
    ContractRead,
    ContractWrite,
    ReportRead,
    UserManage,
    SettingsManage,
}

impl Permission {
    /// 資料庫中的字串形式。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CustomerRead => "customer.read",
            Self::CustomerWrite => "customer.write",
            Self::ProjectRead => "project.read",
            Self::ProjectWrite => "project.write",
            Self::QuotationRead => "quotation.read",
            Self::QuotationWrite => "quotation.write",
            Self::QuotationApprove => "quotation.approve",
            Self::QuotationCostRead => "quotation.cost.read",
            Self::ContractRead => "contract.read",
            Self::ContractWrite => "contract.write",
            Self::ReportRead => "report.read",
            Self::UserManage => "user.manage",
            Self::SettingsManage => "settings.manage",
        }
    }

    /// 全部權限，供設定頁面列出可勾選項目。
    pub const ALL: [Self; 13] = [
        Self::CustomerRead,
        Self::CustomerWrite,
        Self::ProjectRead,
        Self::ProjectWrite,
        Self::QuotationRead,
        Self::QuotationWrite,
        Self::QuotationApprove,
        Self::QuotationCostRead,
        Self::ContractRead,
        Self::ContractWrite,
        Self::ReportRead,
        Self::UserManage,
        Self::SettingsManage,
    ];
}

impl core::str::FromStr for Permission {
    type Err = UnknownPermission;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|p| p.as_str() == value)
            .ok_or_else(|| UnknownPermission {
                value: value.to_owned(),
            })
    }
}

/// 資料庫中存在但程式碼不認識的權限字串。
///
/// 這不一定是錯誤：權限從程式碼移除後，`role_permissions` 可能仍留有舊字串。
/// [`Permissions::from_strings`] 會忽略它們。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("未知的權限「{value}」")]
pub struct UnknownPermission {
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("權限不足，需要「{required}」")]
pub struct Denied {
    pub required: &'static str,
}

/// 一位使用者由其所有角色聯集而成的權限集合。
///
/// 一人可兼多角色，權限取聯集——任一角色給予的權限即生效。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Permissions(BTreeSet<Permission>);

impl Permissions {
    #[must_use]
    pub fn new() -> Self {
        Self(BTreeSet::new())
    }

    /// 由資料庫讀出的權限字串建立。無法辨識的字串靜默忽略——
    /// 已從程式碼移除的舊權限不該讓整個使用者的授權判斷失敗。
    #[must_use]
    pub fn from_strings<'a>(values: impl IntoIterator<Item = &'a str>) -> Self {
        Self(
            values
                .into_iter()
                .filter_map(|v| v.parse::<Permission>().ok())
                .collect(),
        )
    }

    #[must_use]
    pub fn has(&self, permission: Permission) -> bool {
        self.0.contains(&permission)
    }

    /// 檢查權限，不足時回傳具名錯誤供 API 層轉為 403。
    pub fn require(&self, permission: Permission) -> Result<(), Denied> {
        if self.has(permission) {
            Ok(())
        } else {
            Err(Denied {
                required: permission.as_str(),
            })
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = Permission> + '_ {
        self.0.iter().copied()
    }
}

impl FromIterator<Permission> for Permissions {
    fn from_iter<T: IntoIterator<Item = Permission>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

/// 是否可檢視成本與毛利。
///
/// 此判斷用於 API 的 DTO 序列化階段——無此權限者回應中根本不含成本欄位，
/// 不是由前端隱藏。前端隱藏等於把資料送到瀏覽器再假裝看不到。
#[must_use]
pub fn can_view_cost(permissions: &Permissions) -> bool {
    permissions.has(Permission::QuotationCostRead)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_conversion_round_trips_for_every_permission() {
        for p in Permission::ALL {
            assert_eq!(p.as_str().parse::<Permission>(), Ok(p));
        }
        assert!("nope".parse::<Permission>().is_err());
    }

    #[test]
    fn permission_strings_are_unique() {
        let unique: BTreeSet<&str> = Permission::ALL.iter().map(|p| p.as_str()).collect();
        assert_eq!(
            unique.len(),
            Permission::ALL.len(),
            "有權限共用了同一個字串"
        );
    }

    #[test]
    fn unknown_permission_strings_are_ignored_rather_than_failing() {
        // 資料庫可能留有已從程式碼移除的舊權限字串，
        // 那不該讓整個使用者的授權判斷失敗
        let perms = Permissions::from_strings(["customer.read", "legacy.permission", "報價.看"]);
        assert!(perms.has(Permission::CustomerRead));
        assert_eq!(perms.iter().count(), 1);
    }

    #[test]
    fn require_names_the_missing_permission() {
        let perms = Permissions::from_strings(["customer.read"]);
        assert!(perms.require(Permission::CustomerRead).is_ok());
        assert_eq!(
            perms.require(Permission::QuotationApprove),
            Err(Denied {
                required: "quotation.approve"
            })
        );
    }

    #[test]
    fn empty_permissions_grant_nothing() {
        let perms = Permissions::new();
        assert!(perms.is_empty());
        for p in Permission::ALL {
            assert!(perms.require(p).is_err(), "{p:?} 不該被授予");
        }
    }

    /// 業務預設看不到成本。這是刻意的分工，若要開放在角色管理勾選即可。
    #[test]
    fn sales_role_cannot_view_cost_by_default() {
        let sales = Permissions::from_strings([
            "customer.read",
            "customer.write",
            "project.read",
            "project.write",
            "quotation.read",
            "quotation.write",
            "contract.read",
        ]);
        assert!(!can_view_cost(&sales));
        assert!(sales.require(Permission::QuotationApprove).is_err());
    }

    #[test]
    fn accounting_role_can_view_cost() {
        let accounting = Permissions::from_strings([
            "customer.read",
            "project.read",
            "quotation.read",
            "quotation.cost.read",
            "contract.read",
            "report.read",
        ]);
        assert!(can_view_cost(&accounting));
        assert!(accounting.require(Permission::QuotationWrite).is_err());
    }

    /// 一人兼多角色時權限取聯集。
    #[test]
    fn multiple_roles_union_their_permissions() {
        let sales = Permissions::from_strings(["quotation.write"]);
        let approver = Permissions::from_strings(["quotation.approve"]);

        let combined: Permissions = sales.iter().chain(approver.iter()).collect();

        assert!(combined.has(Permission::QuotationWrite));
        assert!(combined.has(Permission::QuotationApprove));
    }

    #[test]
    fn duplicate_permissions_collapse() {
        let perms = Permissions::from_strings(["customer.read", "customer.read"]);
        assert_eq!(perms.iter().count(), 1);
    }
}
