//! 人類可讀單號的產生。
//!
//! 不用 `SELECT max(...) + 1`：在 read committed 隔離級別下，兩個並行交易
//! 會讀到同一個最大值並產生重複。UNIQUE 約束雖擋得下，但使用者看到的是 500。
//!
//! 這裡以年度序號表取號，用 `INSERT ... ON CONFLICT DO UPDATE RETURNING`：
//! UPSERT 本身取得該列的排他鎖，不需要另外 `SELECT ... FOR UPDATE`。
//! （migration 0001 的註解還寫著 FOR UPDATE，那是舊版的做法；
//! 已套用的 migration 不能再改，以此處為準。）
//!
//! 取號必須與使用該號碼的 INSERT 落在同一個交易裡，否則交易回滾時
//! 號碼會留下缺口。列鎖因此持有到 COMMIT——實測在每年約 1,000 張單據的
//! 需求下有 5 個數量級的餘裕，不是瓶頸。

use sqlx::PgExecutor;

/// 單號種類。前綴同時決定了格式，例如 `P2026-001`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberKind {
    Customer,
    Project,
    Quotation,
    Contract,
}

impl NumberKind {
    const fn key(self) -> &'static str {
        match self {
            Self::Customer => "customer",
            Self::Project => "project",
            Self::Quotation => "quotation",
            Self::Contract => "contract",
        }
    }

    const fn prefix(self) -> &'static str {
        match self {
            Self::Customer => "C",
            Self::Project => "P",
            Self::Quotation => "Q",
            Self::Contract => "CT",
        }
    }
}

/// 取得下一個單號，格式為 `<前綴><西元年>-<三位流水號>`。
///
/// 流水號逐年重置。超過 999 時不截斷，直接變成四位數。
pub async fn next(ex: impl PgExecutor<'_>, kind: NumberKind, year: i16) -> sqlx::Result<String> {
    // ON CONFLICT DO UPDATE 讓「首次使用該年度」與「後續遞增」共用一句 SQL，
    // 且 UPSERT 本身取得列鎖，不需要額外的 SELECT ... FOR UPDATE。
    let seq = sqlx::query_scalar!(
        r#"
        INSERT INTO number_sequences (kind, year, next_value)
        VALUES ($1, $2, 2)
        ON CONFLICT (kind, year) DO UPDATE
          SET next_value = number_sequences.next_value + 1
        RETURNING (next_value - 1) AS "seq!"
        "#,
        kind.key(),
        year,
    )
    .fetch_one(ex)
    .await?;

    Ok(format!("{}{year}-{seq:03}", kind.prefix()))
}
