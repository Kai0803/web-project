//! 報價單 PDF 產生的整合測試。
//!
//! 排版輸出不容易用斷言驗證長相，因此這裡驗的是「產得出來、而且是有效的 PDF、
//! 內容取自資料庫」。版面本身由人眼檢查，產物留在 target/ 供檢視。

use acms_domain::{money, NewQuotationItem, UserId};
use rust_decimal_macros::dec;
use sqlx::PgPool;

use acms_app::{db, pdf};

#[sqlx::test]
async fn renders_a_quotation_to_pdf(pool: PgPool) {
    let user_id = sqlx::query_scalar!(
        "INSERT INTO users (username,password_hash,display_name) VALUES ('t','x','測試') RETURNING id"
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let customer = db::customer::insert(
        &pool,
        db::customer::NewCustomer {
            code: "C2026-001".into(),
            name: "宏昌建設股份有限公司".into(),
            tax_id: Some("53912345".into()),
            phone: None,
            address: None,
            note: None,
        },
    )
    .await
    .unwrap();

    let project = db::project::insert(
        &pool,
        db::project::NewProject {
            code: "P2026-001".into(),
            customer_id: customer.id,
            name: "內湖科技園區 B 棟空調配管".into(),
            site_address: Some("台北市內湖區瑞光路 123 號".into()),
            manager_id: None,
            expected_start: None,
            expected_end: None,
            note: None,
        },
    )
    .await
    .unwrap();

    let quotation = db::quotation::insert(
        &pool,
        db::quotation::NewQuotation {
            project_id: project.id,
            quote_no: "Q2026-001".into(),
            version: 1,
            tax_rate: money::default_tax_rate(),
            valid_until: None,
            created_by: UserId(user_id),
            note: None,
        },
    )
    .await
    .unwrap();

    let lines = [
        ("冷媒銅管 3/8\"", "CU-038", "米", dec!(120), dec!(180)),
        ("保溫材 13mm", "", "米", dec!(120), dec!(70)),
        ("配管施工", "", "式", dec!(1), dec!(45000)),
        ("零星工料", "", "米", dec!(3.33), dec!(85.5)),
    ];
    let items: Vec<NewQuotationItem> = lines
        .iter()
        .enumerate()
        .map(|(i, (name, spec, unit, qty, price))| NewQuotationItem {
            line_no: i as i32 + 1,
            name: (*name).into(),
            spec: if spec.is_empty() {
                None
            } else {
                Some((*spec).into())
            },
            unit: (*unit).into(),
            quantity: *qty,
            unit_price: *price,
            unit_cost: None,
            amount: money::line_amount(*qty, *price).unwrap(),
            note: None,
        })
        .collect();

    db::quotation::insert_items(&pool, quotation.id, &items)
        .await
        .unwrap();
    let amounts: Vec<_> = items.iter().map(|i| i.amount).collect();
    let totals = money::totals(&amounts, quotation.tax_rate).unwrap();
    db::quotation::update_totals(&pool, quotation.id, totals)
        .await
        .unwrap();

    let mut company = db::company::get(&pool).await.unwrap();
    company.name = "欣翔空調工程有限公司".into();
    company.tax_id = Some("12345678".into());
    company.phone = Some("02-2712-3456".into());
    company.address = Some("台北市內湖區陽光街 88 號 3 樓".into());
    db::company::update(&pool, &company).await.unwrap();

    let company = db::company::get(&pool).await.unwrap();
    let quotation = db::quotation::find(&pool, quotation.id)
        .await
        .unwrap()
        .unwrap();
    let stored = db::quotation::items(&pool, quotation.id).await.unwrap();

    let bytes =
        pdf::render(&company, &customer, &project, &quotation, &stored).expect("應產生 PDF");

    assert!(bytes.starts_with(b"%PDF-"), "輸出應為有效的 PDF 檔頭");
    assert!(bytes.len() > 10_000, "含中文字型子集的 PDF 不該這麼小");

    // 留一份供人眼檢查版面
    let out = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("quotation-sample.pdf");
    std::fs::write(&out, &bytes).unwrap();
    eprintln!("PDF 已輸出：{}", out.display());
}

/// 沒有明細的報價單也要能印——使用者可能先印空白單再手寫。
#[sqlx::test]
async fn renders_an_empty_quotation(pool: PgPool) {
    let user_id = sqlx::query_scalar!(
        "INSERT INTO users (username,password_hash,display_name) VALUES ('t','x','測試') RETURNING id"
    )
    .fetch_one(&pool).await.unwrap();
    let customer = db::customer::insert(
        &pool,
        db::customer::NewCustomer {
            code: "C1".into(),
            name: "甲公司".into(),
            tax_id: None,
            phone: None,
            address: None,
            note: None,
        },
    )
    .await
    .unwrap();
    let project = db::project::insert(
        &pool,
        db::project::NewProject {
            code: "P1".into(),
            customer_id: customer.id,
            name: "案場".into(),
            site_address: None,
            manager_id: None,
            expected_start: None,
            expected_end: None,
            note: None,
        },
    )
    .await
    .unwrap();
    let quotation = db::quotation::insert(
        &pool,
        db::quotation::NewQuotation {
            project_id: project.id,
            quote_no: "Q1".into(),
            version: 1,
            tax_rate: money::default_tax_rate(),
            valid_until: None,
            created_by: UserId(user_id),
            note: None,
        },
    )
    .await
    .unwrap();

    let company = db::company::get(&pool).await.unwrap();
    let bytes = pdf::render(&company, &customer, &project, &quotation, &[]).unwrap();
    assert!(bytes.starts_with(b"%PDF-"));
}
