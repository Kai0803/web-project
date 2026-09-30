//! `db` 層的整合測試。
//!
//! 這些測試跑在真實的 Postgres 上。`#[sqlx::test]` 為每個測試建立一個
//! 獨立資料庫、跑完 migration、結束後丟棄，因此測試之間互不干擾。
//!
//! 用 mock 取代資料庫在這一層測不到東西：這裡要驗的正是 SQL 本身是否正確、
//! 約束是否生效、以及交易語意是否如預期。

use acms_domain::{money, NewQuotationItem, Permission, ProjectStatus, QuotationStatus, UserId};
use rust_decimal_macros::dec;
use sqlx::PgPool;

use acms_app::db::numbering::NumberKind;
use acms_app::db::{self, customer, numbering, project, quotation, user, Page};

/// 建立測試所需的最小資料：一個使用者、一個客戶、一個專案。
async fn fixture(pool: &PgPool) -> (UserId, acms_domain::Customer, acms_domain::Project) {
    let user_id = sqlx::query_scalar!(
        "INSERT INTO users (username, password_hash, display_name) VALUES ($1,$2,$3) RETURNING id",
        "tester",
        "x",
        "測試者",
    )
    .fetch_one(pool)
    .await
    .unwrap();

    let c = customer::insert(
        pool,
        customer::NewCustomer {
            code: "C2026-001".into(),
            name: "測試客戶".into(),
            tax_id: Some("12345678".into()),
            phone: None,
            address: None,
            note: None,
        },
    )
    .await
    .unwrap();

    let p = project::insert(
        pool,
        project::NewProject {
            code: "P2026-001".into(),
            customer_id: c.id,
            name: "內湖案場".into(),
            site_address: Some("台北市內湖區".into()),
            manager_id: None,
            expected_start: None,
            expected_end: None,
            note: None,
        },
    )
    .await
    .unwrap();

    (UserId(user_id), c, p)
}

// ==================== 客戶 ====================

#[sqlx::test]
async fn customer_round_trips(pool: PgPool) {
    let created = customer::insert(
        &pool,
        customer::NewCustomer {
            code: "C2026-001".into(),
            name: "甲公司".into(),
            tax_id: None,
            phone: Some("02-1234-5678".into()),
            address: None,
            note: None,
        },
    )
    .await
    .unwrap();

    let found = customer::find(&pool, created.id).await.unwrap().unwrap();
    assert_eq!(found, created);

    let by_code = customer::find_by_code(&pool, "C2026-001").await.unwrap();
    assert_eq!(by_code.as_ref().map(|c| c.id), Some(created.id));

    assert_eq!(customer::count(&pool).await.unwrap(), 1);
    assert!(customer::find_by_code(&pool, "nope")
        .await
        .unwrap()
        .is_none());
}

/// 部分更新：外層 `None` 不動該欄位，`Some(None)` 才是清空。
#[sqlx::test]
async fn customer_patch_distinguishes_absent_from_cleared(pool: PgPool) {
    let c = customer::insert(
        &pool,
        customer::NewCustomer {
            code: "C1".into(),
            name: "原名".into(),
            tax_id: Some("11111111".into()),
            phone: Some("02-1".into()),
            address: None,
            note: None,
        },
    )
    .await
    .unwrap();

    let updated = customer::update(
        &pool,
        c.id,
        customer::CustomerPatch {
            name: Some("新名".into()),
            phone: Some(None), // 明確清空
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(updated.name, "新名");
    assert_eq!(updated.phone, None, "Some(None) 應清空");
    assert_eq!(
        updated.tax_id,
        Some("11111111".into()),
        "未提及的欄位應維持原值"
    );
}

#[sqlx::test]
async fn customer_update_of_missing_row_returns_none(pool: PgPool) {
    let result = customer::update(
        &pool,
        acms_domain::CustomerId(9999),
        customer::CustomerPatch::default(),
    )
    .await
    .unwrap();
    assert!(result.is_none());
}

// ==================== 單號 ====================

#[sqlx::test]
async fn numbers_increment_and_reset_each_year(pool: PgPool) {
    assert_eq!(
        numbering::next(&pool, NumberKind::Project, 2026)
            .await
            .unwrap(),
        "P2026-001"
    );
    assert_eq!(
        numbering::next(&pool, NumberKind::Project, 2026)
            .await
            .unwrap(),
        "P2026-002"
    );
    // 不同種類各有自己的序號
    assert_eq!(
        numbering::next(&pool, NumberKind::Quotation, 2026)
            .await
            .unwrap(),
        "Q2026-001"
    );
    // 跨年重置
    assert_eq!(
        numbering::next(&pool, NumberKind::Project, 2027)
            .await
            .unwrap(),
        "P2027-001"
    );
}

/// 併發取號不得重複。
///
/// 這正是不用 `SELECT max(...) + 1` 的原因：那種寫法在 read committed 下
/// 會讓兩個交易讀到同一個值。此處以 UPSERT 取得列鎖，後到者會等待。
#[sqlx::test]
async fn concurrent_numbering_never_collides(pool: PgPool) {
    let tasks: Vec<_> = (0..8)
        .map(|_| {
            let pool = pool.clone();
            tokio::spawn(async move {
                let mut tx = pool.begin().await.unwrap();
                let n = numbering::next(&mut *tx, NumberKind::Project, 2026)
                    .await
                    .unwrap();
                tx.commit().await.unwrap();
                n
            })
        })
        .collect();

    let mut numbers = Vec::new();
    for t in tasks {
        numbers.push(t.await.unwrap());
    }
    numbers.sort();
    numbers.dedup();

    assert_eq!(numbers.len(), 8, "八次併發取號應得到八個相異單號");
}

// ==================== 使用者權限 ====================

#[sqlx::test]
async fn permissions_are_the_union_of_all_roles(pool: PgPool) {
    let (user_id, _, _) = fixture(&pool).await;

    sqlx::query!(
        "INSERT INTO user_roles (user_id, role_code) VALUES ($1,'sales'), ($1,'accounting')",
        user_id.0
    )
    .execute(&pool)
    .await
    .unwrap();

    let u = user::load(&pool, user_id).await.unwrap().unwrap();

    // sales 有寫報價權限，accounting 沒有
    assert!(u.permissions.has(Permission::QuotationWrite));
    // accounting 有成本權限，sales 沒有——聯集後應具備
    assert!(u.permissions.has(Permission::QuotationCostRead));
    // 兩者皆無的權限不該出現
    assert!(!u.permissions.has(Permission::UserManage));
}

#[sqlx::test]
async fn user_without_roles_has_no_permissions(pool: PgPool) {
    let (user_id, _, _) = fixture(&pool).await;
    let u = user::load(&pool, user_id).await.unwrap().unwrap();
    assert!(u.permissions.is_empty(), "未指派角色者不該有任何權限");
}

#[sqlx::test]
async fn sales_role_alone_cannot_see_cost(pool: PgPool) {
    let (user_id, _, _) = fixture(&pool).await;
    sqlx::query!(
        "INSERT INTO user_roles (user_id, role_code) VALUES ($1,'sales')",
        user_id.0
    )
    .execute(&pool)
    .await
    .unwrap();

    let u = user::load(&pool, user_id).await.unwrap().unwrap();
    assert!(!acms_domain::authz::can_view_cost(&u.permissions));
}

// ==================== 報價單 ====================

async fn draft_quotation(
    pool: &PgPool,
    user_id: UserId,
    project_id: acms_domain::ProjectId,
) -> acms_domain::Quotation {
    quotation::insert(
        pool,
        quotation::NewQuotation {
            project_id,
            quote_no: "Q2026-001".into(),
            version: 1,
            tax_rate: money::default_tax_rate(),
            valid_until: None,
            created_by: user_id,
            note: None,
        },
    )
    .await
    .unwrap()
}

#[sqlx::test]
async fn quotation_items_insert_in_one_round_trip_and_totals_match_domain(pool: PgPool) {
    let (user_id, _, p) = fixture(&pool).await;
    let q = draft_quotation(&pool, user_id, p.id).await;

    let lines = [
        (dec!(120), dec!(180)), // 21,600
        (dec!(120), dec!(70)),  // 8,400
        (dec!(1), dec!(45000)), // 45,000
    ];
    let items: Vec<NewQuotationItem> = lines
        .iter()
        .enumerate()
        .map(|(i, (qty, price))| NewQuotationItem {
            line_no: i as i32 + 1,
            name: format!("項目{}", i + 1),
            spec: None,
            unit: "式".into(),
            quantity: *qty,
            unit_price: *price,
            unit_cost: None,
            amount: money::line_amount(*qty, *price).unwrap(),
            note: None,
        })
        .collect();

    let written = quotation::insert_items(&pool, q.id, &items).await.unwrap();
    assert_eq!(written, 3, "三列應以單次 INSERT 寫入");

    let stored = quotation::items(&pool, q.id).await.unwrap();
    assert_eq!(stored.len(), 3);
    assert_eq!(stored[0].amount, dec!(21600));

    // 金額一律由 domain 重算後回寫，SQL 裡沒有第二份 SUM 實作
    let amounts: Vec<_> = stored.iter().map(|i| i.amount).collect();
    let totals = money::totals(&amounts, q.tax_rate).unwrap();
    assert!(quotation::update_totals(&pool, q.id, totals).await.unwrap());

    let reloaded = quotation::find(&pool, q.id).await.unwrap().unwrap();
    assert_eq!(reloaded.subtotal, dec!(75000));
    assert_eq!(reloaded.tax_amount, dec!(3750));
    assert_eq!(reloaded.total, dec!(78750));
}

#[sqlx::test]
async fn inserting_no_items_is_a_no_op(pool: PgPool) {
    let (user_id, _, p) = fixture(&pool).await;
    let q = draft_quotation(&pool, user_id, p.id).await;
    assert_eq!(quotation::insert_items(&pool, q.id, &[]).await.unwrap(), 0);
}

/// 樂觀鎖：`lock_version` 不符時更新應失敗，代表他人已先行修改。
#[sqlx::test]
async fn status_update_respects_the_optimistic_lock(pool: PgPool) {
    let (user_id, _, p) = fixture(&pool).await;
    let q = draft_quotation(&pool, user_id, p.id).await;

    assert!(
        quotation::update_status(&pool, q.id, QuotationStatus::Pending, None, q.lock_version)
            .await
            .unwrap(),
        "以正確的 lock_version 更新應成功"
    );

    assert!(
        !quotation::update_status(&pool, q.id, QuotationStatus::Approved, None, q.lock_version)
            .await
            .unwrap(),
        "以過期的 lock_version 更新應失敗"
    );
}

/// 資料庫層的保證：同一報價單只能有一個核准版本。
#[sqlx::test]
async fn only_one_version_can_be_approved(pool: PgPool) {
    let (user_id, _, p) = fixture(&pool).await;

    let v1 = draft_quotation(&pool, user_id, p.id).await;
    let v2 = quotation::insert(
        &pool,
        quotation::NewQuotation {
            project_id: p.id,
            quote_no: "Q2026-001".into(),
            version: 2,
            tax_rate: money::default_tax_rate(),
            valid_until: None,
            created_by: user_id,
            note: None,
        },
    )
    .await
    .unwrap();

    // v1 送審後核准
    quotation::update_status(
        &pool,
        v1.id,
        QuotationStatus::Pending,
        None,
        v1.lock_version,
    )
    .await
    .unwrap();
    quotation::update_status(
        &pool,
        v1.id,
        QuotationStatus::Approved,
        Some(user_id),
        v1.lock_version + 1,
    )
    .await
    .unwrap();

    // v2 也想核准 —— 應被 partial unique index 擋下
    quotation::update_status(
        &pool,
        v2.id,
        QuotationStatus::Pending,
        None,
        v2.lock_version,
    )
    .await
    .unwrap();
    let err = quotation::update_status(
        &pool,
        v2.id,
        QuotationStatus::Approved,
        Some(user_id),
        v2.lock_version + 1,
    )
    .await
    .unwrap_err();

    assert!(
        err.to_string().contains("one_approved_version_per_quote"),
        "應由 partial unique index 擋下，實際錯誤：{err}"
    );
}

/// 核准後明細凍結，由資料庫 trigger 保證，不倚賴應用層記得檢查。
#[sqlx::test]
async fn approved_quotation_items_are_frozen(pool: PgPool) {
    let (user_id, _, p) = fixture(&pool).await;
    let q = draft_quotation(&pool, user_id, p.id).await;

    let item = NewQuotationItem {
        line_no: 1,
        name: "銅管".into(),
        spec: None,
        unit: "米".into(),
        quantity: dec!(100),
        unit_price: dec!(180),
        unit_cost: None,
        amount: dec!(18000),
        note: None,
    };
    quotation::insert_items(&pool, q.id, std::slice::from_ref(&item))
        .await
        .unwrap();

    quotation::update_status(&pool, q.id, QuotationStatus::Pending, None, q.lock_version)
        .await
        .unwrap();
    quotation::update_status(
        &pool,
        q.id,
        QuotationStatus::Approved,
        Some(user_id),
        q.lock_version + 1,
    )
    .await
    .unwrap();

    let err = quotation::delete_items(&pool, q.id).await.unwrap_err();
    assert!(
        err.to_string().contains("明細不可異動"),
        "應由 trigger 擋下，實際錯誤：{err}"
    );
}

#[sqlx::test]
async fn superseding_leaves_the_kept_version_alone(pool: PgPool) {
    let (user_id, _, p) = fixture(&pool).await;
    let v1 = draft_quotation(&pool, user_id, p.id).await;
    let v2 = quotation::insert(
        &pool,
        quotation::NewQuotation {
            project_id: p.id,
            quote_no: "Q2026-001".into(),
            version: 2,
            tax_rate: money::default_tax_rate(),
            valid_until: None,
            created_by: user_id,
            note: None,
        },
    )
    .await
    .unwrap();

    let count = quotation::supersede_other_versions(&pool, "Q2026-001", v2.id)
        .await
        .unwrap();
    assert_eq!(count, 1);

    let old = quotation::find(&pool, v1.id).await.unwrap().unwrap();
    let kept = quotation::find(&pool, v2.id).await.unwrap().unwrap();
    assert_eq!(old.status, QuotationStatus::Superseded);
    assert_eq!(kept.status, QuotationStatus::Draft, "保留的版本不該被更動");

    // 再呼叫一次不應重複更新已是 superseded 的列
    assert_eq!(
        quotation::supersede_other_versions(&pool, "Q2026-001", v2.id)
            .await
            .unwrap(),
        0
    );
}

#[sqlx::test]
async fn latest_version_reports_the_highest_number(pool: PgPool) {
    let (user_id, _, p) = fixture(&pool).await;
    assert_eq!(
        quotation::latest_version(&pool, "Q2026-001").await.unwrap(),
        None
    );

    draft_quotation(&pool, user_id, p.id).await;
    assert_eq!(
        quotation::latest_version(&pool, "Q2026-001").await.unwrap(),
        Some(1)
    );
}

// ==================== 交易語意 ====================

/// `db` 函式收 executor 而非 pool，因此多個呼叫可以落在同一交易裡並一起回滾。
/// 這是整個簽章契約存在的理由。
#[sqlx::test]
async fn multiple_db_calls_share_one_transaction_and_roll_back_together(pool: PgPool) {
    let mut tx = pool.begin().await.unwrap();

    let c = customer::insert(
        &mut *tx,
        customer::NewCustomer {
            code: "C-TX".into(),
            name: "交易測試".into(),
            tax_id: None,
            phone: None,
            address: None,
            note: None,
        },
    )
    .await
    .unwrap();

    project::insert(
        &mut *tx,
        project::NewProject {
            code: "P-TX".into(),
            customer_id: c.id,
            name: "交易測試案".into(),
            site_address: None,
            manager_id: None,
            expected_start: None,
            expected_end: None,
            note: None,
        },
    )
    .await
    .unwrap();

    tx.rollback().await.unwrap();

    assert_eq!(
        customer::count(&pool).await.unwrap(),
        0,
        "回滾後客戶不應存在"
    );
    assert_eq!(project::count(&pool, None).await.unwrap(), 0);
}

// ==================== 專案列表 ====================

#[sqlx::test]
async fn project_list_joins_the_customer_name_in_one_query(pool: PgPool) {
    let (_, c, _) = fixture(&pool).await;

    let rows = project::list(&pool, None, Page::default()).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].customer_name, c.name);
}

#[sqlx::test]
async fn project_list_filters_by_status(pool: PgPool) {
    let (_, _, p) = fixture(&pool).await;

    assert_eq!(
        project::list(&pool, Some(ProjectStatus::Lead), Page::default())
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        project::list(&pool, Some(ProjectStatus::Closed), Page::default())
            .await
            .unwrap()
            .is_empty()
    );

    assert!(project::set_status(&pool, p.id, ProjectStatus::Closed)
        .await
        .unwrap());
    assert_eq!(
        project::count(&pool, Some(ProjectStatus::Closed))
            .await
            .unwrap(),
        1
    );
}

/// 分頁排序必須全序。`created_at` 在測試中可能同秒，
/// 決勝鍵 `id` 保證兩次查詢的順序一致，否則翻頁會重複或漏資料。
#[sqlx::test]
async fn pagination_order_is_stable_across_pages(pool: PgPool) {
    let (_, c, _) = fixture(&pool).await;

    for i in 2..=10 {
        project::insert(
            &pool,
            project::NewProject {
                code: format!("P2026-{i:03}"),
                customer_id: c.id,
                name: format!("案場{i}"),
                site_address: None,
                manager_id: None,
                expected_start: None,
                expected_end: None,
                note: None,
            },
        )
        .await
        .unwrap();
    }

    let page1 = project::list(&pool, None, Page::new(Some(5), Some(0)))
        .await
        .unwrap();
    let page2 = project::list(&pool, None, Page::new(Some(5), Some(5)))
        .await
        .unwrap();

    let mut codes: Vec<_> = page1
        .iter()
        .chain(page2.iter())
        .map(|r| r.project.code.clone())
        .collect();
    let total = codes.len();
    codes.sort();
    codes.dedup();

    assert_eq!(total, 10);
    assert_eq!(codes.len(), 10, "跨頁不應出現重複或遺漏");
}

// ==================== 公司資料 ====================

#[sqlx::test]
async fn company_profile_is_seeded_and_updatable(pool: PgPool) {
    let profile = db::company::get(&pool).await.unwrap();
    assert!(profile.name.is_empty(), "抬頭留空，由使用者自行填寫");
    assert!(profile.quote_terms.contains("報價有效期限"), "條款應已寫入");
    assert!(profile.logo_svg.is_some(), "logo 應已寫入");

    let updated = acms_domain::CompanyProfile {
        name: "〇〇空調工程有限公司".into(),
        ..profile
    };
    db::company::update(&pool, &updated).await.unwrap();

    assert_eq!(
        db::company::get(&pool).await.unwrap().name,
        "〇〇空調工程有限公司"
    );
}

// ==================== 稽核 ====================

#[sqlx::test]
async fn audit_entries_are_returned_newest_first(pool: PgPool) {
    let (user_id, c, _) = fixture(&pool).await;

    db::audit::record(&pool, Some(user_id), "customer", c.id.0, "create", None)
        .await
        .unwrap();
    db::audit::record(
        &pool,
        Some(user_id),
        "customer",
        c.id.0,
        "update",
        Some(serde_json::json!({ "name": { "from": "舊", "to": "新" } })),
    )
    .await
    .unwrap();

    let history = db::audit::history(&pool, "customer", c.id.0, 10)
        .await
        .unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].action, "update", "最新的在前");
    assert_eq!(
        history[0].changes.as_ref().unwrap()["name"]["to"],
        serde_json::json!("新")
    );
}
