//! 報價單的狀態機與可變性規則。
//!
//! 報價一旦核准就送到客戶手上，此後不可再改——「修改報價產生新版本、
//! 保留舊版可查」的整個前提就是舊版不可變。這裡的 [`QuotationStatus::is_mutable`]
//! 是所有寫入路徑的前置檢查，資料庫另有 trigger 擋掉繞過應用層的手動 SQL。

/// 報價單狀態。
///
/// ```text
/// draft ──► pending ──► approved ──► (轉合約)
///   ▲          │
///   │          └──► rejected ──► draft
///   │
/// draft/pending/approved ──► superseded（產生新版本時舊版轉此）
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuotationStatus {
    /// 草稿。唯一可編輯明細與金額的狀態。
    Draft,
    /// 送審中，等待核准。
    Pending,
    /// 已核准，可轉合約。
    Approved,
    /// 已駁回，可退回草稿修改後重送。
    Rejected,
    /// 已被新版本取代。
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QuotationError {
    #[error("無法將報價單從「{from}」變更為「{to}」")]
    InvalidTransition {
        from: QuotationStatus,
        to: QuotationStatus,
    },

    #[error("「{status}」狀態的報價單不可修改，請建立新版本")]
    NotMutable { status: QuotationStatus },

    #[error("版本號必須大於 0")]
    InvalidVersion,

    #[error("未知的報價單狀態「{value}」")]
    UnknownStatus { value: String },
}

impl QuotationStatus {
    /// 資料庫 `quotation_status` enum 的字面值。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Superseded => "superseded",
        }
    }

    /// 是否可修改明細與金額。只有草稿可以。
    #[must_use]
    pub const fn is_mutable(self) -> bool {
        matches!(self, Self::Draft)
    }

    /// 是否可轉為合約。只有已核准的可以。
    #[must_use]
    pub const fn can_become_contract(self) -> bool {
        matches!(self, Self::Approved)
    }

    /// 是否為終態（不會再變動，除非被新版本取代）。
    #[must_use]
    pub const fn is_final(self) -> bool {
        matches!(self, Self::Superseded)
    }

    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Draft, Self::Pending)
                | (Self::Pending, Self::Approved)
                | (Self::Pending, Self::Rejected)
                | (Self::Rejected, Self::Draft)
                // 產生新版本時，任何非終態的舊版都轉為 superseded
                | (Self::Draft, Self::Superseded)
                | (Self::Pending, Self::Superseded)
                | (Self::Approved, Self::Superseded)
                | (Self::Rejected, Self::Superseded)
        )
    }
}

impl core::str::FromStr for QuotationStatus {
    type Err = QuotationError;

    /// 由資料庫 `quotation_status` enum 的字面值還原。
    /// 未知字串為真正的錯誤：資料庫的 enum 型別不該產生程式碼不認識的值。
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "draft" => Ok(Self::Draft),
            "pending" => Ok(Self::Pending),
            "approved" => Ok(Self::Approved),
            "rejected" => Ok(Self::Rejected),
            "superseded" => Ok(Self::Superseded),
            other => Err(QuotationError::UnknownStatus {
                value: other.to_owned(),
            }),
        }
    }
}

impl core::fmt::Display for QuotationStatus {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let label = match self {
            Self::Draft => "草稿",
            Self::Pending => "送審中",
            Self::Approved => "已核准",
            Self::Rejected => "已駁回",
            Self::Superseded => "已被取代",
        };
        f.write_str(label)
    }
}

/// 檢查狀態轉換是否合法。
pub fn validate_transition(
    from: QuotationStatus,
    to: QuotationStatus,
) -> Result<(), QuotationError> {
    if from.can_transition_to(to) {
        Ok(())
    } else {
        Err(QuotationError::InvalidTransition { from, to })
    }
}

/// 檢查報價單是否可修改明細或金額。
pub fn ensure_mutable(status: QuotationStatus) -> Result<(), QuotationError> {
    if status.is_mutable() {
        Ok(())
    } else {
        Err(QuotationError::NotMutable { status })
    }
}

/// 由現行版本號算出新版本號。
pub fn next_version(current: i32) -> Result<i32, QuotationError> {
    if current < 1 {
        return Err(QuotationError::InvalidVersion);
    }
    Ok(current + 1)
}

#[cfg(test)]
mod tests {
    use super::QuotationStatus::{Approved, Draft, Pending, Rejected, Superseded};
    use super::*;

    const ALL: [QuotationStatus; 5] = [Draft, Pending, Approved, Rejected, Superseded];

    #[test]
    fn only_draft_is_mutable() {
        assert!(Draft.is_mutable());
        for s in ALL.into_iter().filter(|s| *s != Draft) {
            assert!(!s.is_mutable(), "{s} 不該可修改");
        }
    }

    #[test]
    fn only_approved_can_become_a_contract() {
        assert!(Approved.can_become_contract());
        for s in ALL.into_iter().filter(|s| *s != Approved) {
            assert!(!s.can_become_contract(), "{s} 不該可轉合約");
        }
    }

    #[test]
    fn allows_the_designed_transitions() {
        for (from, to) in [
            (Draft, Pending),
            (Pending, Approved),
            (Pending, Rejected),
            (Rejected, Draft),
            (Draft, Superseded),
            (Pending, Superseded),
            (Approved, Superseded),
            (Rejected, Superseded),
        ] {
            assert!(
                validate_transition(from, to).is_ok(),
                "{from} → {to} 應為合法"
            );
        }
    }

    #[test]
    fn rejects_skipping_review() {
        // 草稿不可直接核准，必須先送審
        assert_eq!(
            validate_transition(Draft, Approved),
            Err(QuotationError::InvalidTransition {
                from: Draft,
                to: Approved
            })
        );
    }

    #[test]
    fn rejects_reviving_an_approved_quotation() {
        // 已核准的報價不可退回草稿，只能產生新版本
        assert!(validate_transition(Approved, Draft).is_err());
        assert!(validate_transition(Approved, Pending).is_err());
        assert!(validate_transition(Approved, Rejected).is_err());
    }

    #[test]
    fn superseded_is_terminal() {
        assert!(Superseded.is_final());
        for to in ALL {
            assert!(
                validate_transition(Superseded, to).is_err(),
                "已被取代的報價不該能轉為 {to}"
            );
        }
    }

    #[test]
    fn no_status_transitions_to_itself() {
        for s in ALL {
            assert!(validate_transition(s, s).is_err(), "{s} 不該轉為自己");
        }
    }

    #[test]
    fn string_conversion_round_trips() {
        for s in ALL {
            assert_eq!(s.as_str().parse::<QuotationStatus>(), Ok(s));
        }
    }

    /// 大小寫必須完全一致。若放寬，資料庫的 partial unique index
    /// `WHERE status = 'approved'` 就會被 'Approved' 這類值繞過。
    #[test]
    fn string_conversion_is_case_sensitive_and_reports_unknown_values() {
        assert_eq!(
            "Approved".parse::<QuotationStatus>(),
            Err(QuotationError::UnknownStatus {
                value: "Approved".to_owned()
            })
        );
        assert!("".parse::<QuotationStatus>().is_err());
        assert!("approve".parse::<QuotationStatus>().is_err());
    }

    #[test]
    fn ensure_mutable_names_the_offending_status() {
        assert!(ensure_mutable(Draft).is_ok());
        assert_eq!(
            ensure_mutable(Approved),
            Err(QuotationError::NotMutable { status: Approved })
        );
    }

    #[test]
    fn next_version_increments_and_rejects_invalid_input() {
        assert_eq!(next_version(1).unwrap(), 2);
        assert_eq!(next_version(7).unwrap(), 8);
        assert_eq!(next_version(0), Err(QuotationError::InvalidVersion));
        assert_eq!(next_version(-1), Err(QuotationError::InvalidVersion));
    }
}
