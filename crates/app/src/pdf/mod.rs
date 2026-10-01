//! 報價單 PDF。
//!
//! 以 typst 排版。選它而非 headless 瀏覽器的理由是映像大小：
//! 純 Rust 排版引擎只增加數 MB，瀏覽器方案要數百 MB 外加 process 管理。
//!
//! 這裡**不做任何金額運算**——所有數字都是 `acms_domain::money` 算好、
//! 存在資料庫裡的字串，照印即可。進位規則只有一份實作，
//! 因此列印出來的數字與畫面上、與 API 回傳的完全一致。

mod world;

use acms_domain::{CompanyProfile, Customer, Project, Quotation, QuotationItem};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    /// 環境缺中文字型。與其產出一整片豆腐方塊的廢紙，不如明確失敗。
    #[error("找不到中文字型，無法產生報價單。請確認 FONT_DIR 指向含中文字型的目錄")]
    NoCjkFont,

    #[error("排版失敗：{0}")]
    Typst(String),

    #[error("資料序列化失敗")]
    Encode(#[from] serde_json::Error),
}

#[derive(Serialize)]
struct CompanyData {
    name: String,
    tax_id: String,
    phone: String,
    fax: String,
    address: String,
    logo_svg: String,
}

#[derive(Serialize)]
struct CustomerData {
    name: String,
    tax_id: String,
}

#[derive(Serialize)]
struct ProjectData {
    name: String,
    site_address: String,
}

#[derive(Serialize)]
struct ItemData {
    line_no: i32,
    name: String,
    spec: String,
    unit: String,
    quantity: String,
    unit_price: String,
    amount: String,
}

#[derive(Serialize)]
struct TemplateData {
    company: CompanyData,
    customer: CustomerData,
    project: ProjectData,
    quote_no: String,
    version: i32,
    issued_on: String,
    valid_until: String,
    items: Vec<ItemData>,
    subtotal: String,
    tax_amount: String,
    tax_percent: String,
    total: String,
    terms: String,
    /// 字型 fallback 順序。中文字型排前面，typst 內附的西文字型墊底。
    font_stack: Vec<String>,
}

/// 千分位。純字串處理，不經過數值型別——
/// 這裡若 `parse::<f64>()` 再格式化，整條鏈刻意維持的精度就白費了。
fn group(value: &str) -> String {
    let (sign, rest) = value.strip_prefix('-').map_or(("", value), |r| ("-", r));
    let (whole, fraction) = rest.split_once('.').map_or((rest, ""), |(w, f)| (w, f));

    let mut grouped = String::with_capacity(whole.len() + whole.len() / 3);
    for (index, ch) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }

    if fraction.is_empty() {
        format!("{sign}{grouped}")
    } else {
        format!("{sign}{grouped}.{fraction}")
    }
}

/// 稅率字串轉百分比標籤，例如 `0.0500` → `5%`。純字串處理。
fn tax_percent(rate: &str) -> String {
    let (whole, fraction) = rate.split_once('.').map_or((rate, ""), |(w, f)| (w, f));
    let digits: String = format!("{whole}{fraction}");
    let trimmed = digits.trim_start_matches('0');
    let digits = if trimmed.is_empty() { "0" } else { trimmed };

    // 乘以 100 等於小數點右移兩位
    let shift = fraction.len().saturating_sub(2);
    let padded = format!("{:0>width$}", digits, width = shift + 1);
    let cut = padded.len() - shift;
    let result = if shift == 0 {
        padded
    } else {
        format!("{}.{}", &padded[..cut], &padded[cut..])
    };

    let result = result.trim_end_matches('0').trim_end_matches('.');
    format!("{}%", if result.is_empty() { "0" } else { result })
}

fn optional(value: &Option<String>) -> String {
    value.clone().unwrap_or_default()
}

/// 產生報價單 PDF。
///
/// 排版是 CPU 密集工作，呼叫端應以 `spawn_blocking` 執行，
/// 否則會佔住 async runtime 的 worker。
pub fn render(
    company: &CompanyProfile,
    customer: &Customer,
    project: &Project,
    quotation: &Quotation,
    items: &[QuotationItem],
) -> Result<Vec<u8>, PdfError> {
    let cjk = world::cjk_families();
    if cjk.is_empty() {
        return Err(PdfError::NoCjkFont);
    }

    let mut font_stack: Vec<String> = cjk.to_vec();
    font_stack.push("Libertinus Serif".to_owned());

    let data = TemplateData {
        company: CompanyData {
            name: if company.name.is_empty() {
                "（尚未設定公司抬頭）".to_owned()
            } else {
                company.name.clone()
            },
            tax_id: optional(&company.tax_id),
            phone: optional(&company.phone),
            fax: optional(&company.fax),
            address: optional(&company.address),
            logo_svg: optional(&company.logo_svg),
        },
        customer: CustomerData {
            name: customer.name.clone(),
            tax_id: optional(&customer.tax_id),
        },
        project: ProjectData {
            name: project.name.clone(),
            site_address: optional(&project.site_address),
        },
        quote_no: quotation.quote_no.clone(),
        version: quotation.version,
        issued_on: quotation.created_at.date().to_string(),
        valid_until: quotation
            .valid_until
            .map(|d| d.to_string())
            .unwrap_or_default(),
        items: items
            .iter()
            .map(|i| ItemData {
                line_no: i.line_no,
                name: i.name.clone(),
                spec: optional(&i.spec),
                unit: i.unit.clone(),
                quantity: group(&i.quantity.normalize().to_string()),
                unit_price: group(&i.unit_price.normalize().to_string()),
                amount: group(&i.amount.normalize().to_string()),
            })
            .collect(),
        subtotal: group(&quotation.subtotal.normalize().to_string()),
        tax_amount: group(&quotation.tax_amount.normalize().to_string()),
        tax_percent: tax_percent(&quotation.tax_rate.to_string()),
        total: group(&quotation.total.normalize().to_string()),
        terms: company.quote_terms.clone(),
        font_stack,
    };

    let json = serde_json::to_string(&data)?;
    let world = world::QuotationWorld::new(json);

    let document = typst::compile::<typst_layout::PagedDocument>(&world)
        .output
        .map_err(
            |errors: typst::diag::EcoVec<typst::diag::SourceDiagnostic>| {
                let detail = errors
                    .iter()
                    .map(|e| e.message.to_string())
                    .collect::<Vec<_>>()
                    .join("；");
                PdfError::Typst(detail)
            },
        )?;

    typst_pdf::pdf(&document, &typst_pdf::PdfOptions::default())
        .map_err(|errors| PdfError::Typst(format!("{errors:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_thousands_without_touching_precision() {
        assert_eq!(group("0"), "0");
        assert_eq!(group("285"), "285");
        assert_eq!(group("75285"), "75,285");
        assert_eq!(group("1234567"), "1,234,567");
        assert_eq!(group("-5000"), "-5,000");
        assert_eq!(group("3.33"), "3.33");
        assert_eq!(group("12345.6789"), "12,345.6789");
    }

    #[test]
    fn renders_tax_rate_as_a_percentage() {
        assert_eq!(tax_percent("0.0500"), "5%");
        assert_eq!(tax_percent("0.05"), "5%");
        assert_eq!(tax_percent("0.0725"), "7.25%");
        assert_eq!(tax_percent("0"), "0%");
    }
}
