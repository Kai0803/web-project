//! Service 層（use case）的整合測試。
//!
//! 重點在交易邊界與授權：流程中途失敗時是否整筆回滾、
//! 權限不足時是否在寫入任何資料前就擋下。

use acms_domain::{money, ProjectId, QuotationStatus, User, UserId};
use rust_decimal_macros::dec;
use sqlx::PgPool;

use acms_app::db;
use acms_app::service::{contract, quotation, ServiceError};

/// 建立一位帶指定角色的使用者，以及一個客戶與專案。
async fn setup(pool: &PgPool, roles: &[&str]) -> (User, ProjectId) {
    let user_id = sqlx::query_scalar!(
        "INSERT INTO users (username, password_hash, display_name) VALUES ($1,$2,$3) RETURNING id",
        format!("u{}", roles.join("_")),
        "x",
        "測試者",
    )
    .fetch_one(pool)
    .await
    .unwrap();

    for role in roles {
        sqlx::query!(
            "INSERT INTO user_roles (user_id, role_code) VALUES ($1,$2)",
            user_id,
            role,
        )
        .execute(pool)
        .await
        .unwrap();
    }

    let user = db::user::load(pool, UserId(user_id))
        .await
        .unwrap()
        .unwrap();

    let customer = db::customer::insert(
        pool,
        db::customer::NewCustomer {
            code: "C2026-001".into(),
            name: "測試客戶".into(),
            tax_id: None,
            phone: None,
            address: None,
            note: None,
        },
    )
    .await
    .unwrap();

    let project = db::project::insert(
        pool,
        db::project::NewProject {
            code: "P2026-001".into(),
            customer_id: customer.id,
            name: "內湖案場".into(),
            site_address: None,
            manager_id: None,
            expected_start: None,
            expected_end: None,
            note: None,
        },
    )
    .await
    .unwrap();

    (user, project.id)
}

fn lines() -> Vec<quotation::LineInput> {
    vec![
        quotation::LineInput {
            name: "冷媒銅管 3/8\"".into(),
            spec: None,
            unit: "米".into(),
            quantity: dec!(120),
            unit_price: dec!(180),
            unit_cost: Some(dec!(120)),
            note: None,
        },
        quotation::LineInput {
            name: "配管施工".into(),
            spec: None,
            unit: "式".into(),
            quantity: dec!(1),
            unit_price: dec!(45000),
            unit_cost: Some(dec!(30000)),
            note: None,
        },
    ]
}

// ==================== 建立 ====================

#[sqlx::test]
async fn create_prices_lines_numbers_them_and_totals_up(pool: PgPool) {
    let (user, project_id) = setup(&pool, &["sales"]).await;

    let detail = quotation::create(&pool, &user, project_id, &lines(), None)
        .await
        .unwrap();

    assert!(detail.quotation.quote_no.starts_with('Q'));
    assert_eq!(detail.quotation.version, 1);
    assert_eq!(detail.quotation.status, QuotationStatus::Draft);

    assert_eq!(detail.items.len(), 2);
    assert_eq!(detail.items[0].line_no, 1);
    assert_eq!(detail.items[1].line_no, 2);
    assert_eq!(detail.items[0].amount, dec!(21600));

    assert_eq!(detail.quotation.subtotal, dec!(66600));
    assert_eq!(detail.quotation.tax_amount, dec!(3330));
    assert_eq!(detail.quotation.total, dec!(69930));
}

/// 金額由 domain 計算，呼叫端無從指定——`LineInput` 根本沒有 amount 欄位。
/// 這個測試確認寫入的值確實是 `line_amount` 的輸出。
#[sqlx::test]
async fn amounts_come_from_the_domain_not_the_caller(pool: PgPool) {
    let (user, project_id) = setup(&pool, &["sales"]).await;

    let odd = vec![quotation::LineInput {
        name: "零星工料".into(),
        spec: None,
        unit: "米".into(),
        quantity: dec!(3.33),
        unit_price: dec!(85.5),
        unit_cost: None,
        note: None,
    }];

    let detail = quotation::create(&pool, &user, project_id, &odd, None)
        .await
        .unwrap();

    let expected = money::line_amount(dec!(3.33), dec!(85.5)).unwrap();
    assert_eq!(detail.items[0].amount, expected);
    assert_eq!(detail.items[0].amount.fract(), dec!(0), "列小計須為整數元");
}

#[sqlx::test]
async fn create_rejects_a_missing_project(pool: PgPool) {
    let (user, _) = setup(&pool, &["sales"]).await;

    let err = quotation::create(&pool, &user, ProjectId(9999), &lines(), None)
        .await
        .unwrap_err();

    assert!(matches!(err, ServiceError::NotFound { entity: "專案" }));
}

/// 權限不足時必須在任何寫入前擋下，尤其不可消耗單號。
#[sqlx::test]
async fn create_without_permission_consumes_nothing(pool: PgPool) {
    let (user, project_id) = setup(&pool, &["viewer"]).await;

    let err = quotation::create(&pool, &user, project_id, &lines(), None)
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::Denied(_)));

    let used = sqlx::query_scalar!(r#"SELECT count(*) AS "c!" FROM number_sequences"#)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(used, 0, "被拒的請求不該消耗單號");
}

/// 明細違反業務上限時，整筆交易回滾——不該留下沒有明細的空報價單。
#[sqlx::test]
async fn invalid_line_leaves_no_partial_quotation(pool: PgPool) {
    let (user, project_id) = setup(&pool, &["sales"]).await;

    let bad = vec![quotation::LineInput {
        name: "數量為零".into(),
        spec: None,
        unit: "式".into(),
        quantity: dec!(0),
        unit_price: dec!(100),
        unit_cost: None,
        note: None,
    }];

    let err = quotation::create(&pool, &user, project_id, &bad, None)
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::Money(_)));

    let count = sqlx::query_scalar!(r#"SELECT count(*) AS "c!" FROM quotations"#)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

// ==================== 明細重算 ====================

#[sqlx::test]
async fn replacing_items_recomputes_the_header(pool: PgPool) {
    let (user, project_id) = setup(&pool, &["sales"]).await;
    let created = quotation::create(&pool, &user, project_id, &lines(), None)
        .await
        .unwrap();

    let fewer = vec![quotation::LineInput {
        name: "只剩施工".into(),
        spec: None,
        unit: "式".into(),
        quantity: dec!(1),
        unit_price: dec!(10000),
        unit_cost: None,
        note: None,
    }];

    let updated = quotation::replace_items(&pool, &user, created.quotation.id, &fewer)
        .await
        .unwrap();

    assert_eq!(updated.items.len(), 1);
    assert_eq!(updated.quotation.subtotal, dec!(10000));
    assert_eq!(updated.quotation.tax_amount, dec!(500));
    assert_eq!(updated.quotation.total, dec!(10500));
}

#[sqlx::test]
async fn approved_quotation_items_cannot_be_replaced(pool: PgPool) {
    let (user, project_id) = setup(&pool, &["admin"]).await;
    let created = quotation::create(&pool, &user, project_id, &lines(), None)
        .await
        .unwrap();
    let id = created.quotation.id;

    quotation::submit(&pool, &user, id).await.unwrap();
    quotation::approve(&pool, &user, id).await.unwrap();

    let err = quotation::replace_items(&pool, &user, id, &lines())
        .await
        .unwrap_err();
    assert!(
        matches!(err, ServiceError::Rule(_)),
        "應由 domain 的 ensure_mutable 擋下，實際：{err}"
    );

    // 明細與金額都不該被動到
    let after = quotation::detail(&pool, &user, id).await.unwrap();
    assert_eq!(after.items.len(), 2);
    assert_eq!(after.quotation.total, created.quotation.total);
}

// ==================== 狀態流轉 ====================

#[sqlx::test]
async fn sales_can_submit_but_not_approve(pool: PgPool) {
    let (sales, project_id) = setup(&pool, &["sales"]).await;
    let created = quotation::create(&pool, &sales, project_id, &lines(), None)
        .await
        .unwrap();
    let id = created.quotation.id;

    let submitted = quotation::submit(&pool, &sales, id).await.unwrap();
    assert_eq!(submitted.status, QuotationStatus::Pending);

    let err = quotation::approve(&pool, &sales, id).await.unwrap_err();
    assert!(
        matches!(err, ServiceError::Denied(_)),
        "業務不該有核准權限，實際：{err}"
    );
}

#[sqlx::test]
async fn cannot_approve_a_draft_without_review(pool: PgPool) {
    let (user, project_id) = setup(&pool, &["admin"]).await;
    let created = quotation::create(&pool, &user, project_id, &lines(), None)
        .await
        .unwrap();

    let err = quotation::approve(&pool, &user, created.quotation.id)
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::Rule(_)));
}

#[sqlx::test]
async fn rejected_quotation_can_be_reopened_and_resubmitted(pool: PgPool) {
    let (user, project_id) = setup(&pool, &["admin"]).await;
    let id = quotation::create(&pool, &user, project_id, &lines(), None)
        .await
        .unwrap()
        .quotation
        .id;

    quotation::submit(&pool, &user, id).await.unwrap();
    assert_eq!(
        quotation::reject(&pool, &user, id).await.unwrap().status,
        QuotationStatus::Rejected
    );
    assert_eq!(
        quotation::reopen(&pool, &user, id).await.unwrap().status,
        QuotationStatus::Draft
    );
    assert_eq!(
        quotation::submit(&pool, &user, id).await.unwrap().status,
        QuotationStatus::Pending
    );
}

#[sqlx::test]
async fn approval_records_the_approver_and_audits_the_change(pool: PgPool) {
    let (user, project_id) = setup(&pool, &["admin"]).await;
    let id = quotation::create(&pool, &user, project_id, &lines(), None)
        .await
        .unwrap()
        .quotation
        .id;

    quotation::submit(&pool, &user, id).await.unwrap();
    let approved = quotation::approve(&pool, &user, id).await.unwrap();

    assert_eq!(approved.approved_by, Some(user.id));
    assert!(approved.approved_at.is_some());

    let history = db::audit::history(&pool, "quotation", id.0, 10)
        .await
        .unwrap();
    let statuses: Vec<_> = history.iter().map(|e| e.action.as_str()).collect();
    assert!(statuses.contains(&"status"));
    assert_eq!(
        history[0].changes.as_ref().unwrap()["status"]["to"],
        serde_json::json!("approved")
    );
}

// ==================== 改版 ====================

#[sqlx::test]
async fn new_version_copies_items_and_supersedes_the_old_one(pool: PgPool) {
    let (user, project_id) = setup(&pool, &["admin"]).await;
    let v1 = quotation::create(&pool, &user, project_id, &lines(), None)
        .await
        .unwrap();

    let v2 = quotation::new_version(&pool, &user, v1.quotation.id)
        .await
        .unwrap();

    assert_eq!(v2.quotation.version, 2);
    assert_eq!(v2.quotation.quote_no, v1.quotation.quote_no, "單號不變");
    assert_eq!(v2.quotation.status, QuotationStatus::Draft);
    assert_eq!(v2.items.len(), v1.items.len());
    assert_eq!(v2.quotation.total, v1.quotation.total);

    let old = db::quotation::find(&pool, v1.quotation.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(old.status, QuotationStatus::Superseded);

    // 舊版明細不該被動到——「保留舊版可查」的前提就是舊版不可變
    let old_items = db::quotation::items(&pool, v1.quotation.id).await.unwrap();
    assert_eq!(old_items.len(), 2);
}

/// 已核准的版本也可以被新版本取代。
#[sqlx::test]
async fn approved_version_can_be_superseded_by_a_new_one(pool: PgPool) {
    let (user, project_id) = setup(&pool, &["admin"]).await;
    let v1 = quotation::create(&pool, &user, project_id, &lines(), None)
        .await
        .unwrap();
    quotation::submit(&pool, &user, v1.quotation.id)
        .await
        .unwrap();
    quotation::approve(&pool, &user, v1.quotation.id)
        .await
        .unwrap();

    let v2 = quotation::new_version(&pool, &user, v1.quotation.id)
        .await
        .unwrap();

    assert_eq!(v2.quotation.version, 2);
    // 舊版讓出核准位置後，新版才可能被核准
    quotation::submit(&pool, &user, v2.quotation.id)
        .await
        .unwrap();
    assert_eq!(
        quotation::approve(&pool, &user, v2.quotation.id)
            .await
            .unwrap()
            .status,
        QuotationStatus::Approved
    );
}

// ==================== 轉合約 ====================

#[sqlx::test]
async fn converting_to_contract_materialises_work_items(pool: PgPool) {
    let (user, project_id) = setup(&pool, &["admin"]).await;
    let q = quotation::create(&pool, &user, project_id, &lines(), None)
        .await
        .unwrap();
    quotation::submit(&pool, &user, q.quotation.id)
        .await
        .unwrap();
    quotation::approve(&pool, &user, q.quotation.id)
        .await
        .unwrap();

    let c = contract::create_from_quotation(&pool, &user, q.quotation.id, None)
        .await
        .unwrap();

    assert!(c.contract_no.starts_with("CT"));
    assert_eq!(c.total, q.quotation.total, "合約金額為簽約當下的快照");

    // 工項：第二、三階段的掛載點
    let count = db::contract::count_work_items(&pool, c.id).await.unwrap();
    assert_eq!(count, 2, "報價明細應實體化為工項");

    // 專案狀態同步推進
    let project = db::project::find(&pool, project_id).await.unwrap().unwrap();
    assert_eq!(project.status, acms_domain::ProjectStatus::Contracted);
}

#[sqlx::test]
async fn only_an_approved_quotation_can_become_a_contract(pool: PgPool) {
    let (user, project_id) = setup(&pool, &["admin"]).await;
    let q = quotation::create(&pool, &user, project_id, &lines(), None)
        .await
        .unwrap();

    let err = contract::create_from_quotation(&pool, &user, q.quotation.id, None)
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::Rule(_)));

    // 失敗不該留下合約，也不該推進專案狀態
    let project = db::project::find(&pool, project_id).await.unwrap().unwrap();
    assert_eq!(project.status, acms_domain::ProjectStatus::Lead);
}

#[sqlx::test]
async fn a_quotation_cannot_be_contracted_twice(pool: PgPool) {
    let (user, project_id) = setup(&pool, &["admin"]).await;
    let q = quotation::create(&pool, &user, project_id, &lines(), None)
        .await
        .unwrap();
    quotation::submit(&pool, &user, q.quotation.id)
        .await
        .unwrap();
    quotation::approve(&pool, &user, q.quotation.id)
        .await
        .unwrap();

    contract::create_from_quotation(&pool, &user, q.quotation.id, None)
        .await
        .unwrap();

    let err = contract::create_from_quotation(&pool, &user, q.quotation.id, None)
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::AlreadyContracted));
}

/// 已轉合約的報價不可再改版，否則合約會指向一份「已被取代」的報價。
#[sqlx::test]
async fn contracted_quotation_cannot_be_superseded(pool: PgPool) {
    let (user, project_id) = setup(&pool, &["admin"]).await;
    let q = quotation::create(&pool, &user, project_id, &lines(), None)
        .await
        .unwrap();
    quotation::submit(&pool, &user, q.quotation.id)
        .await
        .unwrap();
    quotation::approve(&pool, &user, q.quotation.id)
        .await
        .unwrap();
    contract::create_from_quotation(&pool, &user, q.quotation.id, None)
        .await
        .unwrap();

    let err = quotation::new_version(&pool, &user, q.quotation.id)
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::AlreadyContracted));
}

#[sqlx::test]
async fn sales_cannot_create_contracts(pool: PgPool) {
    let (sales, project_id) = setup(&pool, &["sales"]).await;
    let q = quotation::create(&pool, &sales, project_id, &lines(), None)
        .await
        .unwrap();

    let err = contract::create_from_quotation(&pool, &sales, q.quotation.id, None)
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::Denied(_)));
}

// ==================== 讀取權限 ====================

#[sqlx::test]
async fn reading_a_quotation_requires_read_permission(pool: PgPool) {
    let (admin, project_id) = setup(&pool, &["admin"]).await;
    let q = quotation::create(&pool, &admin, project_id, &lines(), None)
        .await
        .unwrap();

    // 無任何角色的使用者
    let outsider_id = sqlx::query_scalar!(
        "INSERT INTO users (username, password_hash, display_name) VALUES ('nobody','x','外人') RETURNING id"
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let outsider = db::user::load(&pool, UserId(outsider_id))
        .await
        .unwrap()
        .unwrap();

    let err = quotation::detail(&pool, &outsider, q.quotation.id)
        .await
        .unwrap_err();
    assert!(matches!(err, ServiceError::Denied(_)));
}
