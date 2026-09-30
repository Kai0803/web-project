//! 公司資料。單列表，由「系統設定 → 公司資料」頁面填寫。

use acms_domain::CompanyProfile;
use sqlx::PgExecutor;

/// 讀取公司資料。migration 已寫入初始列，故必定存在。
pub async fn get(ex: impl PgExecutor<'_>) -> sqlx::Result<CompanyProfile> {
    sqlx::query!(
        r#"
        SELECT name, tax_id, phone, fax, email, address, logo_svg, quote_terms
        FROM company_profile WHERE id = 1
        "#
    )
    .fetch_one(ex)
    .await
    .map(|r| CompanyProfile {
        name: r.name,
        tax_id: r.tax_id,
        phone: r.phone,
        fax: r.fax,
        email: r.email,
        address: r.address,
        logo_svg: r.logo_svg,
        quote_terms: r.quote_terms,
    })
}

pub async fn update(ex: impl PgExecutor<'_>, profile: &CompanyProfile) -> sqlx::Result<()> {
    sqlx::query!(
        r#"
        UPDATE company_profile SET
          name = $1, tax_id = $2, phone = $3, fax = $4,
          email = $5, address = $6, logo_svg = $7, quote_terms = $8
        WHERE id = 1
        "#,
        profile.name,
        profile.tax_id,
        profile.phone,
        profile.fax,
        profile.email,
        profile.address,
        profile.logo_svg,
        profile.quote_terms,
    )
    .execute(ex)
    .await
    .map(|_| ())
}
