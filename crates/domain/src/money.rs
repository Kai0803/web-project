//! 金額計算與進位。
//!
//! 全專案的金額運算只能經由此模組。直接呼叫 `Decimal::round()` 或
//! `Decimal::round_dp()` 是錯的——它們預設是銀行家進位（half to even），
//! 不是台灣會計慣例的四捨五入。實測 `Decimal::from_str("500.5").round()`
//! 得到 500 而非 501，而 Postgres 的 `round()` 得到 501，兩邊預設不一致。
//!
//! # 進位規則
//!
//! 報價單列印時每列小計顯示整數元，因此規則必須讓客戶逐列相加能對上合計：
//!
//! 1. 列小計 `amount = round_half_up(quantity × unit_price, 0)`
//! 2. 未稅總額 `subtotal = Σ amount`（各列已是整數，直接相加）
//! 3. 稅額 `tax_amount = round_half_up(subtotal × tax_rate, 0)`
//! 4. 含稅總額 `total = subtotal + tax_amount`
//!
//! 關鍵是**逐列進位**而非先加總再進位。先加總再進位配上整數元顯示，
//! 客戶逐列相加會與印出的合計差到列數的一半元。

use rust_decimal::{Decimal, RoundingStrategy};

/// 數量上限。超過此值視為輸入錯誤而非合法報價。
pub const MAX_QUANTITY: i64 = 1_000_000;

/// 單價絕對值上限。取絕對值是因為折讓列的單價為負數。
pub const MAX_UNIT_PRICE_ABS: i64 = 100_000_000;

/// 單列小計與報價單合計的絕對值上限。
///
/// 這個值由 schema 決定而非業務判斷：`quotation_items.amount` 與
/// `quotations.subtotal / tax_amount / total` 都是 `numeric(14,4)`，
/// 整數部分只有 `14 - 4 = 10` 位，寫得進去的最大值是 9,999,999,999
/// （Postgres 的錯誤訊息寫得很直接：must round to an absolute value
/// less than 10^10）。
///
/// 若只擋 `MAX_QUANTITY × MAX_UNIT_PRICE_ABS`（1e14），通得過 domain
/// 驗證的值會在 INSERT 時以 `22003 numeric_field_overflow` 失敗，
/// 使用者拿到 500 加一個追蹤 ID，而原因只是數量多打了幾個零。
/// 上限屬於領域規則，不該由欄位型別代為表達——這裡把兩者對齊。
pub const MAX_AMOUNT: i64 = 9_999_999_999;

/// 單張報價單的明細列數上限。
///
/// G1 的規模是每張 10–50 列。設上限的理由不是效能而是界定 `subtotal`
/// 的值域：每列都合法（≤ `MAX_AMOUNT`）時，數百列相加仍可能超出欄位容量，
/// 所以逐列與合計兩道都要有。
pub const MAX_LINES: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MoneyError {
    #[error("數量必須大於 0")]
    QuantityNotPositive,

    #[error("數量超過上限 {MAX_QUANTITY}")]
    QuantityTooLarge,

    #[error("單價絕對值超過上限 {MAX_UNIT_PRICE_ABS}")]
    UnitPriceTooLarge,

    #[error("稅率必須介於 0 與 1 之間")]
    InvalidTaxRate,

    #[error("列小計超過上限 {MAX_AMOUNT}，請確認數量與單價")]
    AmountTooLarge,

    #[error("報價總額超過上限 {MAX_AMOUNT}，請拆分為多張報價單")]
    TotalTooLarge,

    #[error("明細列數超過上限 {MAX_LINES}")]
    TooManyLines,

    /// 在業務上限內不可能發生，但金額來源若為已損毀的資料仍需回報而非 panic。
    #[error("金額運算溢位")]
    Overflow,
}

/// 預設營業稅率（台灣 5%）。
///
/// 實際計算一律讀 `quotations.tax_rate` 欄位而非此常數——稅率調整過後，
/// 歷史報價重印必須沿用當初的稅率。此值僅用於建立新報價時的預設。
#[must_use]
pub fn default_tax_rate() -> Decimal {
    Decimal::new(5, 2)
}

/// 台灣會計慣例的四捨五入（half away from zero）。
///
/// 全專案唯一的進位入口。負數往絕對值大的方向進位（-0.5 → -1）。
#[must_use]
pub fn round_half_up(value: Decimal, dp: u32) -> Decimal {
    value.round_dp_with_strategy(dp, RoundingStrategy::MidpointAwayFromZero)
}

/// 驗證數量。數量必須為正——折讓以負單價表達，不以負數量表達。
pub fn validate_quantity(quantity: Decimal) -> Result<(), MoneyError> {
    if quantity <= Decimal::ZERO {
        return Err(MoneyError::QuantityNotPositive);
    }
    if quantity > Decimal::from(MAX_QUANTITY) {
        return Err(MoneyError::QuantityTooLarge);
    }
    Ok(())
}

/// 驗證單價。允許負數以表達折讓列。
pub fn validate_unit_price(unit_price: Decimal) -> Result<(), MoneyError> {
    if unit_price.abs() > Decimal::from(MAX_UNIT_PRICE_ABS) {
        return Err(MoneyError::UnitPriceTooLarge);
    }
    Ok(())
}

/// 驗證稅率落在 [0, 1]。
pub fn validate_tax_rate(tax_rate: Decimal) -> Result<(), MoneyError> {
    if tax_rate < Decimal::ZERO || tax_rate > Decimal::ONE {
        return Err(MoneyError::InvalidTaxRate);
    }
    Ok(())
}

/// 計算單列小計，四捨五入至整數元。
///
/// 這是寫入 `quotation_items.amount` 的權威值，資料庫有
/// `CHECK (amount = trunc(amount))` 保證它確實是整數。
pub fn line_amount(quantity: Decimal, unit_price: Decimal) -> Result<Decimal, MoneyError> {
    validate_quantity(quantity)?;
    validate_unit_price(unit_price)?;

    let raw = quantity
        .checked_mul(unit_price)
        .ok_or(MoneyError::Overflow)?;
    let amount = round_half_up(raw, 0);

    // 數量 × 單價的業務上限允許到 1e14，但 amount 欄位只容得下 10 位整數。
    // 擋在這裡是 400；擋在 Postgres 就是 500。
    if amount.abs() > Decimal::from(MAX_AMOUNT) {
        return Err(MoneyError::AmountTooLarge);
    }
    Ok(amount)
}

/// 稅率的百分比標籤，例如 `0.0500` → `5%`、`0.1` → `10%`、`0.0725` → `7.25%`。
///
/// 純字串運算，不經過 `f64`——整條鏈刻意避開浮點，這裡不開例外。
///
/// 放在 domain 而非各自的顯示層，是因為這個轉換原本有兩份實作（一份給
/// PDF、一份給畫面），而畫面那份在小數位數為 1 時算錯：`0.1` 顯示成 `1%`。
/// 今天稅率固定 5%（兩位小數）剛好正確，但稅率是隨報價儲存、未來會調整的，
/// 屆時那會是個印在報價單上的錯誤數字。
#[must_use]
pub fn tax_percent_label(rate: Decimal) -> String {
    // 乘 100 等於小數點右移兩位。用 Decimal 自己乘，不自行搬小數點。
    let percent = rate
        .checked_mul(Decimal::ONE_HUNDRED)
        .unwrap_or(Decimal::ZERO)
        .normalize();
    format!("{percent}%")
}

/// 一張報價單的三個金額。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Totals {
    /// 未稅總額
    pub subtotal: Decimal,
    /// 稅額
    pub tax_amount: Decimal,
    /// 含稅總額
    pub total: Decimal,
}

/// 由各列已進位的小計彙總出報價單的三個金額。
///
/// `amounts` 必須是 [`line_amount`] 的輸出（已進位至整數元）。
/// 傳入未進位的值不會被拒絕，但算出的 `subtotal` 將與列印的逐列加總不符。
pub fn totals(amounts: &[Decimal], tax_rate: Decimal) -> Result<Totals, MoneyError> {
    validate_tax_rate(tax_rate)?;
    if amounts.len() > MAX_LINES {
        return Err(MoneyError::TooManyLines);
    }

    let mut subtotal = Decimal::ZERO;
    for amount in amounts {
        subtotal = subtotal.checked_add(*amount).ok_or(MoneyError::Overflow)?;
    }
    // 逐列都合法，加總仍可能超出欄位容量（數百列各近上限）
    if subtotal.abs() > Decimal::from(MAX_AMOUNT) {
        return Err(MoneyError::TotalTooLarge);
    }

    let raw_tax = subtotal.checked_mul(tax_rate).ok_or(MoneyError::Overflow)?;
    let tax_amount = round_half_up(raw_tax, 0);

    let total = subtotal
        .checked_add(tax_amount)
        .ok_or(MoneyError::Overflow)?;
    // subtotal 貼著上限時，加上稅額就會越界
    if total.abs() > Decimal::from(MAX_AMOUNT) {
        return Err(MoneyError::TotalTooLarge);
    }

    Ok(Totals {
        subtotal,
        tax_amount,
        total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    // ---- 進位策略 ----

    #[test]
    fn rounds_half_away_from_zero() {
        assert_eq!(round_half_up(dec!(0.5), 0), dec!(1));
        assert_eq!(round_half_up(dec!(1.5), 0), dec!(2));
        assert_eq!(round_half_up(dec!(2.5), 0), dec!(3));
        assert_eq!(round_half_up(dec!(500.5), 0), dec!(501));
        assert_eq!(round_half_up(dec!(-0.5), 0), dec!(-1));
        assert_eq!(round_half_up(dec!(-2.5), 0), dec!(-3));
    }

    #[test]
    fn rounds_normally_away_from_the_midpoint() {
        assert_eq!(round_half_up(dec!(2.4), 0), dec!(2));
        assert_eq!(round_half_up(dec!(2.6), 0), dec!(3));
        assert_eq!(round_half_up(dec!(2.0), 0), dec!(2));
    }

    /// 說明為何不能用 `Decimal::round()`：它是銀行家進位。
    /// 這個測試若失敗，代表 rust_decimal 改了預設行為，屆時此模組的存在理由要重新檢視。
    #[test]
    fn differs_from_the_builtin_bankers_rounding() {
        assert_eq!(dec!(2.5).round(), dec!(2), "內建 round() 是 half-to-even");
        assert_eq!(
            round_half_up(dec!(2.5), 0),
            dec!(3),
            "我們要 half-away-from-zero"
        );

        assert_eq!(dec!(500.5).round(), dec!(500));
        assert_eq!(round_half_up(dec!(500.5), 0), dec!(501));

        // 整數部分為奇數時兩者相同——這是分歧率只有 2.5% 而非 5% 的原因
        assert_eq!(dec!(501.5).round(), dec!(502));
        assert_eq!(round_half_up(dec!(501.5), 0), dec!(502));
    }

    // ---- 單列小計 ----

    #[test]
    fn line_amount_rounds_to_whole_dollars() {
        // 3.5 米 × 85.5 元 = 299.25 → 299
        assert_eq!(line_amount(dec!(3.5), dec!(85.5)).unwrap(), dec!(299));
        // 正好落在 .5
        assert_eq!(line_amount(dec!(0.5), dec!(1)).unwrap(), dec!(1));
        assert_eq!(line_amount(dec!(1), dec!(0.5)).unwrap(), dec!(1));
    }

    #[test]
    fn line_amount_accepts_negative_unit_price_for_discounts() {
        // 折讓列：數量 1 式，單價 -5000
        assert_eq!(line_amount(dec!(1), dec!(-5000)).unwrap(), dec!(-5000));
        // 折讓的進位方向為遠離零
        assert_eq!(line_amount(dec!(1), dec!(-500.5)).unwrap(), dec!(-501));
    }

    #[test]
    fn line_amount_rejects_non_positive_quantity() {
        assert_eq!(
            line_amount(dec!(0), dec!(100)),
            Err(MoneyError::QuantityNotPositive)
        );
        assert_eq!(
            line_amount(dec!(-1), dec!(100)),
            Err(MoneyError::QuantityNotPositive)
        );
    }

    #[test]
    fn line_amount_rejects_out_of_range_inputs() {
        assert_eq!(
            line_amount(Decimal::from(MAX_QUANTITY + 1), dec!(1)),
            Err(MoneyError::QuantityTooLarge)
        );
        assert_eq!(
            line_amount(dec!(1), Decimal::from(MAX_UNIT_PRICE_ABS + 1)),
            Err(MoneyError::UnitPriceTooLarge)
        );
        assert_eq!(
            line_amount(dec!(1), -Decimal::from(MAX_UNIT_PRICE_ABS + 1)),
            Err(MoneyError::UnitPriceTooLarge)
        );
    }

    // ---- 彙總 ----

    #[test]
    fn totals_sums_lines_and_applies_tax() {
        let amounts = [dec!(21600), dec!(8400), dec!(45000)];
        let t = totals(&amounts, default_tax_rate()).unwrap();
        assert_eq!(t.subtotal, dec!(75000));
        assert_eq!(t.tax_amount, dec!(3750));
        assert_eq!(t.total, dec!(78750));
    }

    /// 稅額落在 .5 的邊界。未稅 10,010 × 5% = 500.5。
    /// 銀行家進位會得到 500，開立發票時差 1 元會被客戶退件。
    #[test]
    fn tax_on_the_half_dollar_boundary_rounds_up() {
        let t = totals(&[dec!(10010)], default_tax_rate()).unwrap();
        assert_eq!(t.tax_amount, dec!(501));
        assert_eq!(t.total, dec!(10511));
    }

    /// 未稅金額 ≡ 10 (mod 20) 時稅額必定落在 .5。
    #[test]
    fn every_subtotal_congruent_to_ten_mod_twenty_hits_the_boundary() {
        for n in [10, 30, 50, 1010, 10010, 20030] {
            let t = totals(&[Decimal::from(n)], default_tax_rate()).unwrap();
            let raw = Decimal::from(n) * default_tax_rate();
            assert_eq!(raw.fract(), dec!(0.5), "n={n} 應落在 .5 邊界");
            assert_eq!(t.tax_amount, round_half_up(raw, 0), "n={n} 應四捨五入進位");
        }
    }

    /// 客戶拿計算機逐列相加，必須等於報價單上印的未稅合計。
    /// 這是選擇「逐列進位」而非「先加總再進位」的唯一理由。
    #[test]
    fn printed_lines_add_up_to_the_printed_subtotal() {
        let lines = [
            (dec!(3.33), dec!(85.5)),
            (dec!(12.7), dec!(61.25)),
            (dec!(0.5), dec!(999.99)),
            (dec!(7.125), dec!(43.3)),
        ];

        let amounts: Vec<Decimal> = lines
            .iter()
            .map(|(q, p)| line_amount(*q, *p).unwrap())
            .collect();

        // 每列都是整數元
        for a in &amounts {
            assert_eq!(a.fract(), Decimal::ZERO, "列小計 {a} 不是整數");
        }

        let t = totals(&amounts, default_tax_rate()).unwrap();
        let hand_sum: Decimal = amounts.iter().copied().sum();
        assert_eq!(t.subtotal, hand_sum);
    }

    /// 客戶拿印出的未稅金額自行乘 5%，應得到印出的稅額。
    #[test]
    fn customer_can_verify_tax_from_the_printed_subtotal() {
        for n in [75000, 10010, 1, 999_999] {
            let t = totals(&[Decimal::from(n)], default_tax_rate()).unwrap();
            let customer_check = round_half_up(t.subtotal * default_tax_rate(), 0);
            assert_eq!(t.tax_amount, customer_check, "n={n}");
            assert_eq!(t.total, t.subtotal + t.tax_amount, "n={n}");
        }
    }

    #[test]
    fn totals_of_no_lines_are_zero() {
        let t = totals(&[], default_tax_rate()).unwrap();
        assert_eq!(t.subtotal, Decimal::ZERO);
        assert_eq!(t.tax_amount, Decimal::ZERO);
        assert_eq!(t.total, Decimal::ZERO);
    }

    #[test]
    fn totals_handles_a_net_negative_quotation() {
        // 折讓大於品項，理論上不該送出，但計算本身不得 panic
        let t = totals(&[dec!(1000), dec!(-3000)], default_tax_rate()).unwrap();
        assert_eq!(t.subtotal, dec!(-2000));
        assert_eq!(t.tax_amount, dec!(-100));
        assert_eq!(t.total, dec!(-2100));
    }

    #[test]
    fn totals_rejects_invalid_tax_rate() {
        assert_eq!(
            totals(&[dec!(100)], dec!(-0.01)),
            Err(MoneyError::InvalidTaxRate)
        );
        assert_eq!(
            totals(&[dec!(100)], dec!(1.5)),
            Err(MoneyError::InvalidTaxRate)
        );
    }

    /// 溢位必須回報錯誤而非 panic。`Decimal` 的 `+` 與 `*` 在溢位時會 panic，
    /// 這在 HTTP handler 內等同單一請求層級的 DoS。
    #[test]
    fn totals_reports_overflow_instead_of_panicking() {
        let huge = Decimal::MAX;
        assert_eq!(
            totals(&[huge, huge], default_tax_rate()),
            Err(MoneyError::Overflow)
        );
    }

    /// 數量與單價各自合法，乘積卻寫不進 `numeric(14,4)` 的情況必須在
    /// domain 被擋下，否則使用者會因為多打幾個零而拿到 500。
    #[test]
    fn line_amount_rejects_products_too_large_for_the_column() {
        // 兩個上限相乘 = 1e14，欄位只容得下 9,999,999,999
        assert_eq!(
            line_amount(
                Decimal::from(MAX_QUANTITY),
                Decimal::from(MAX_UNIT_PRICE_ABS)
            ),
            Err(MoneyError::AmountTooLarge)
        );

        // 最小的可觸達組合：數量 100 × 單價 1e8 = 1e10，只差 1 就越界
        assert_eq!(
            line_amount(dec!(100), Decimal::from(MAX_UNIT_PRICE_ABS)),
            Err(MoneyError::AmountTooLarge)
        );

        // 上限之內要能通過（99 × 1e8 = 9.9e9，仍寫得進欄位）
        assert_eq!(
            line_amount(dec!(99), Decimal::from(MAX_UNIT_PRICE_ABS)).unwrap(),
            dec!(9900000000)
        );

        // 折讓列的負數同樣以絕對值判斷
        assert_eq!(
            line_amount(dec!(100), -Decimal::from(MAX_UNIT_PRICE_ABS)),
            Err(MoneyError::AmountTooLarge)
        );
    }

    /// 每列都合法，加總仍可能超出欄位容量。
    #[test]
    fn totals_rejects_a_subtotal_too_large_for_the_column() {
        let near_max = Decimal::from(MAX_AMOUNT);
        assert_eq!(
            totals(&[near_max, near_max], default_tax_rate()),
            Err(MoneyError::TotalTooLarge)
        );

        // subtotal 剛好在上限上，但加了 5% 稅就越界
        assert_eq!(
            totals(&[near_max], default_tax_rate()),
            Err(MoneyError::TotalTooLarge)
        );

        // 留足稅額空間就該通過
        let ok = totals(&[dec!(1000000000)], default_tax_rate()).unwrap();
        assert_eq!(ok.subtotal, dec!(1000000000));
        assert_eq!(ok.total, dec!(1050000000));
    }

    #[test]
    fn tax_percent_label_handles_every_scale() {
        assert_eq!(tax_percent_label(dec!(0.05)), "5%");
        assert_eq!(tax_percent_label(dec!(0.0500)), "5%");
        // 舊的前端實作在這一列算出 1%
        assert_eq!(tax_percent_label(dec!(0.1)), "10%");
        assert_eq!(tax_percent_label(dec!(0.1000)), "10%");
        assert_eq!(tax_percent_label(dec!(0.0725)), "7.25%");
        assert_eq!(tax_percent_label(dec!(0.0025)), "0.25%");
        assert_eq!(tax_percent_label(dec!(0)), "0%");
        assert_eq!(tax_percent_label(dec!(1)), "100%");
    }

    #[test]
    fn totals_rejects_too_many_lines() {
        let many = vec![dec!(1); MAX_LINES + 1];
        assert_eq!(
            totals(&many, default_tax_rate()),
            Err(MoneyError::TooManyLines)
        );
        let at_limit = vec![dec!(1); MAX_LINES];
        assert!(totals(&at_limit, default_tax_rate()).is_ok());
    }
}
