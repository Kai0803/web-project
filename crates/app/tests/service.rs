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
            source_id: None,
            name: "冷媒銅管 3/8\"".into(),
            spec: None,
            unit: "米".into(),
            quantity: dec!(120),
            unit_price: dec!(180),
            unit_cost: Some(dec!(120)),
            note: None,
        },
        quotation::LineInput {
            source_id: None,
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
        source_id: None,
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
        source_id: None,
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
        source_id: None,
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

/// 建立第二位使用者，共用 [`setup`] 已建好的客戶與專案。
///
/// `setup` 會插入固定代碼的客戶與專案，呼叫兩次會撞唯一鍵。
async fn extra_user(pool: &PgPool, username: &str, roles: &[&str]) -> User {
    let id = sqlx::query_scalar!(
        "INSERT INTO users (username, password_hash, display_name) VALUES ($1,$2,$3) RETURNING id",
        username,
        "x",
        "測試者",
    )
    .fetch_one(pool)
    .await
    .unwrap();

    for role in roles {
        sqlx::query!(
            "INSERT INTO user_roles (user_id, role_code) VALUES ($1,$2)",
            id,
            role,
        )
        .execute(pool)
        .await
        .unwrap();
    }

    db::user::load(pool, UserId(id)).await.unwrap().unwrap()
}

/// 業務重寫明細不該清掉主管填的成本。
///
/// 這是個不需要惡意就會發生的資料損失：業務開啟報價單時 JSON 裡沒有
/// `unit_cost`（讀取側依權限剝除），表單因此是空的，他改一列品名按儲存，
/// 整批成本就被寫成 NULL——而 `replace_items` 是刪掉全部再重寫的語意，
/// 成本事後無法回填。
#[sqlx::test]
async fn sales_rewriting_items_does_not_wipe_costs_they_cannot_see(pool: PgPool) {
    let (manager, project_id) = setup(&pool, &["manager"]).await;
    let sales = extra_user(&pool, "業務小陳", &["sales"]).await;

    assert!(
        acms_domain::authz::can_view_cost(&manager.permissions),
        "前提：manager 看得到成本"
    );
    assert!(
        !acms_domain::authz::can_view_cost(&sales.permissions),
        "前提：sales 看不到成本"
    );

    let created = quotation::create(&pool, &manager, project_id, &lines(), None)
        .await
        .unwrap();
    assert_eq!(created.items[0].unit_cost, Some(dec!(120)));
    assert_eq!(created.items[1].unit_cost, Some(dec!(30000)));

    // 業務改了第一列的品名。成本欄位對他是空的（他根本沒收到值），
    // 但每一列都帶著自己的來源 id 送回來。
    let edited = vec![
        quotation::LineInput {
            source_id: Some(created.items[0].id),
            name: "冷媒銅管 1/2\"".into(),
            spec: None,
            unit: "米".into(),
            quantity: dec!(120),
            unit_price: dec!(180),
            unit_cost: None,
            note: None,
        },
        quotation::LineInput {
            source_id: Some(created.items[1].id),
            name: "配管施工".into(),
            spec: None,
            unit: "式".into(),
            quantity: dec!(1),
            unit_price: dec!(45000),
            unit_cost: None,
            note: None,
        },
    ];

    let updated = quotation::replace_items(&pool, &sales, created.quotation.id, &edited)
        .await
        .unwrap();

    // 業務的改動生效
    assert_eq!(updated.items[0].name, "冷媒銅管 1/2\"");
    // 但成本原封不動
    assert_eq!(
        updated.items[0].unit_cost,
        Some(dec!(120)),
        "業務看不到的成本不該被他的儲存動作清掉"
    );
    assert_eq!(updated.items[1].unit_cost, Some(dec!(30000)));
}

/// 刪掉中間一列之後，剩下的列必須還是接到自己原本的成本。
///
/// 承接的依據若是 `line_no`，這個情境會靜默接錯：`line_no` 是依送來的
/// 陣列位置重新編號的，刪掉第一列之後 B 變成 1、C 變成 2，於是 B 接到
/// A 的成本、C 接到 B 的，C 原本的成本永久消失。而且沒有任何一方會
/// 發現——做這件事的人看不到成本，有權限的人下次打開看到的是一組
/// 合法但錯誤的數字。
#[sqlx::test]
async fn deleting_a_line_does_not_shift_costs_onto_the_wrong_lines(pool: PgPool) {
    let (manager, project_id) = setup(&pool, &["manager"]).await;
    let sales = extra_user(&pool, "業務小陳", &["sales"]).await;

    let three = vec![
        quotation::LineInput {
            source_id: None,
            name: "A 銅管".into(),
            spec: None,
            unit: "米".into(),
            quantity: dec!(10),
            unit_price: dec!(100),
            unit_cost: Some(dec!(60)),
            note: None,
        },
        quotation::LineInput {
            source_id: None,
            name: "B 保溫".into(),
            spec: None,
            unit: "米".into(),
            quantity: dec!(10),
            unit_price: dec!(200),
            unit_cost: Some(dec!(130)),
            note: None,
        },
        quotation::LineInput {
            source_id: None,
            name: "C 吊架".into(),
            spec: None,
            unit: "式".into(),
            quantity: dec!(1),
            unit_price: dec!(5000),
            unit_cost: Some(dec!(3200)),
            note: None,
        },
    ];

    let created = quotation::create(&pool, &manager, project_id, &three, None)
        .await
        .unwrap();
    assert_eq!(created.items.len(), 3);

    // 業務刪掉第一列，B 與 C 原樣送回（帶著自己的來源 id）
    let remaining: Vec<quotation::LineInput> = created.items[1..]
        .iter()
        .map(|i| quotation::LineInput {
            source_id: Some(i.id),
            name: i.name.clone(),
            spec: i.spec.clone(),
            unit: i.unit.clone(),
            quantity: i.quantity,
            unit_price: i.unit_price,
            unit_cost: None, // 業務收到的 JSON 裡沒有這個欄位
            note: i.note.clone(),
        })
        .collect();

    let updated = quotation::replace_items(&pool, &sales, created.quotation.id, &remaining)
        .await
        .unwrap();

    assert_eq!(updated.items.len(), 2);
    assert_eq!(updated.items[0].name, "B 保溫");
    assert_eq!(
        updated.items[0].unit_cost,
        Some(dec!(130)),
        "B 必須保有自己的成本，不是接到 A 的 60"
    );
    assert_eq!(updated.items[1].name, "C 吊架");
    assert_eq!(
        updated.items[1].unit_cost,
        Some(dec!(3200)),
        "C 必須保有自己的成本，不是接到 B 的 130"
    );
}

/// 調換列順序時，成本要跟著各自的列走。
#[sqlx::test]
async fn reordering_lines_keeps_each_cost_with_its_own_line(pool: PgPool) {
    let (manager, project_id) = setup(&pool, &["manager"]).await;
    let sales = extra_user(&pool, "業務小陳", &["sales"]).await;

    let created = quotation::create(&pool, &manager, project_id, &lines(), None)
        .await
        .unwrap();

    // 兩列對調，各自帶著來源 id
    let swapped: Vec<quotation::LineInput> = created
        .items
        .iter()
        .rev()
        .map(|i| quotation::LineInput {
            source_id: Some(i.id),
            name: i.name.clone(),
            spec: i.spec.clone(),
            unit: i.unit.clone(),
            quantity: i.quantity,
            unit_price: i.unit_price,
            unit_cost: None,
            note: i.note.clone(),
        })
        .collect();

    let updated = quotation::replace_items(&pool, &sales, created.quotation.id, &swapped)
        .await
        .unwrap();

    assert_eq!(updated.items[0].name, "配管施工");
    assert_eq!(updated.items[0].unit_cost, Some(dec!(30000)));
    assert_eq!(updated.items[1].name, "冷媒銅管 3/8\"");
    assert_eq!(updated.items[1].unit_cost, Some(dec!(120)));
}

/// 無權限者送進來的成本一律丟棄。
///
/// 反向的攻擊面：業務在看不到成本的前提下寫入假成本，
/// 就能影響主管與會計唯一的毛利判斷依據。
#[sqlx::test]
async fn costs_submitted_without_permission_are_discarded(pool: PgPool) {
    let (manager, project_id) = setup(&pool, &["manager"]).await;
    let sales = extra_user(&pool, "業務小陳", &["sales"]).await;

    // 新建：直接丟棄，沒有既有值可承接
    let forged = vec![quotation::LineInput {
        source_id: None,
        name: "配管施工".into(),
        spec: None,
        unit: "式".into(),
        quantity: dec!(1),
        unit_price: dec!(45000),
        unit_cost: Some(dec!(1)),
        note: None,
    }];

    let created = quotation::create(&pool, &sales, project_id, &forged, None)
        .await
        .unwrap();
    assert_eq!(
        created.items[0].unit_cost, None,
        "無 quotation.cost.read 者送來的成本不該被寫入"
    );

    // 主管填上真正的成本
    let with_cost = vec![quotation::LineInput {
        source_id: Some(created.items[0].id),
        name: "配管施工".into(),
        spec: None,
        unit: "式".into(),
        quantity: dec!(1),
        unit_price: dec!(45000),
        unit_cost: Some(dec!(30000)),
        note: None,
    }];
    let priced = quotation::replace_items(&pool, &manager, created.quotation.id, &with_cost)
        .await
        .unwrap();
    assert_eq!(priced.items[0].unit_cost, Some(dec!(30000)));

    // 業務帶著來源 id 重寫並偽造成本：承接資料庫既有值，忽略送來的 1
    let forged_with_id = vec![quotation::LineInput {
        source_id: Some(priced.items[0].id),
        unit_cost: Some(dec!(1)),
        ..forged[0].clone()
    }];
    let updated = quotation::replace_items(&pool, &sales, created.quotation.id, &forged_with_id)
        .await
        .unwrap();
    assert_eq!(
        updated.items[0].unit_cost,
        Some(dec!(30000)),
        "應承接既有成本，而非業務送來的 1"
    );

    // 不帶來源 id 等於新增的列，沒有成本可承接——也不能用它當成寫入管道
    let updated = quotation::replace_items(&pool, &sales, created.quotation.id, &forged)
        .await
        .unwrap();
    assert_eq!(
        updated.items[0].unit_cost, None,
        "沒有來源 id 的列不該憑位置接到別人的成本"
    );
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

/// 從舊版改版不該把另一個「已簽約」的版本標成 superseded。
///
/// 檢查範圍與動作範圍必須一致：原本只檢查「來源版本是否已轉合約」，
/// 但 `supersede_other_versions` 作用於同單號的**所有其他版本**。
/// 於是從一個早已被取代的 V1 改版，會把已簽約的 V2 變成「已被取代」，
/// 合約因此指向一份被取代的報價——而第二、三階段的 BOM 與派工
/// 都是循 contract → quotation 回溯的。
#[sqlx::test]
async fn revising_an_old_version_cannot_supersede_a_contracted_one(pool: PgPool) {
    let (user, project_id) = setup(&pool, &["admin"]).await;

    let v1 = quotation::create(&pool, &user, project_id, &lines(), None)
        .await
        .unwrap();
    let v2 = quotation::new_version(&pool, &user, v1.quotation.id)
        .await
        .unwrap();

    quotation::submit(&pool, &user, v2.quotation.id)
        .await
        .unwrap();
    quotation::approve(&pool, &user, v2.quotation.id)
        .await
        .unwrap();
    contract::create_from_quotation(&pool, &user, v2.quotation.id, None)
        .await
        .unwrap();

    // V1 自己沒有合約，但它的同單號手足有
    let err = quotation::new_version(&pool, &user, v1.quotation.id)
        .await
        .unwrap_err();
    assert!(
        matches!(err, ServiceError::AlreadyContracted),
        "應擋下，實際：{err}"
    );

    let after = db::quotation::find(&pool, v2.quotation.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after.status,
        QuotationStatus::Approved,
        "已簽約的版本必須維持 approved，否則合約指向一份被取代的報價"
    );
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
