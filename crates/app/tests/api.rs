//! HTTP 層的端對端測試。
//!
//! 直接對 axum 的 `Router` 發請求，不經網路。驗的是這一層獨有的東西：
//! 認證、CSRF、狀態碼映射、以及 DTO 的序列化格式——尤其是金額必須為字串、
//! 成本欄位必須依權限剝除。

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;

use acms_app::api;
use acms_app::auth;

/// 建立測試用的 router 與一位指定角色的使用者（密碼一律為 `pw-2026-test`）。
const PASSWORD: &str = "pw-2026-test";

async fn setup(pool: &PgPool, roles: &[&str]) -> axum::Router {
    let hash = auth::hash_password(PASSWORD).unwrap();
    let user_id = sqlx::query_scalar!(
        "INSERT INTO users (username, password_hash, display_name) VALUES ('tester',$1,'測試者') RETURNING id",
        hash,
    )
    .fetch_one(pool)
    .await
    .unwrap();

    for role in roles {
        sqlx::query!(
            "INSERT INTO user_roles (user_id, role_code) VALUES ($1,$2)",
            user_id,
            role
        )
        .execute(pool)
        .await
        .unwrap();
    }

    api::routes(api::AppState {
        pool: pool.clone(),
        cookie_secure: false,
        // 測試直連，不經代理：退回與 Host 比對的路徑
        public_origin: None,
    })
}

fn request(method: &str, uri: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "localhost")
        // 所有會改變資料的請求都需要這個標頭（CSRF 防護）
        .header("x-requested-with", "XMLHttpRequest")
}

/// 送出請求並解析 JSON 回應。
///
/// 非 JSON 的回應一律當成失敗。原本的寫法是 `unwrap_or(Value::Null)`，
/// 於是 axum 內建 rejection 回的 `text/plain` 被靜默轉成 `Null`，
/// 測試照樣全綠——那是「錯誤回應不在信封裡」這個問題能長期存在的原因。
async fn send(router: &axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    let response = router.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();

    if bytes.is_empty() {
        return (status, Value::Null);
    }
    let body = serde_json::from_slice(&bytes).unwrap_or_else(|err| {
        panic!(
            "回應不是 JSON（status {status}, content-type {content_type}）：{err}\nbody: {}",
            String::from_utf8_lossy(&bytes)
        )
    });
    (status, body)
}

/// 登入並取回 session cookie。
async fn login(router: &axum::Router) -> String {
    let req = request("POST", "/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({ "username": "tester", "password": PASSWORD }).to_string(),
        ))
        .unwrap();

    let response = router.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    response
        .headers()
        .get(header::SET_COOKIE)
        .expect("應設定 session cookie")
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

fn authed(method: &str, uri: &str, cookie: &str) -> axum::http::request::Builder {
    request(method, uri).header(header::COOKIE, cookie)
}

fn json_body(builder: axum::http::request::Builder, value: Value) -> Request<Body> {
    builder
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(value.to_string()))
        .unwrap()
}

// ==================== 認證 ====================

#[sqlx::test]
async fn login_sets_an_httponly_strict_cookie(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;

    let req = json_body(
        request("POST", "/api/v1/auth/login"),
        json!({ "username": "tester", "password": PASSWORD }),
    );
    let response = router.clone().oneshot(req).await.unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();

    assert!(cookie.contains("HttpOnly"), "JavaScript 不該讀得到 session");
    assert!(cookie.contains("SameSite=Strict"), "CSRF 的第一道防線");
    assert!(
        !cookie.contains("Secure"),
        "本機以 HTTP 開發時不應強制 Secure"
    );
}

/// 資料庫裡存的不能是 cookie 裡那個 token。
///
/// 存原始 token 的話，`sessions.id` 那一欄就等於 bearer 憑證：
/// 唯讀資料庫帳號、每日 pg_dump 留在 volume 上的備份、或拿到備份檔的人
/// 都能把值貼進 cookie 冒充任何使用者，而稽核會記成受害者做的。
#[sqlx::test]
async fn session_tokens_are_not_stored_in_the_clear(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let cookie = login(&router).await;

    let token = cookie
        .split(';')
        .next()
        .and_then(|c| c.split_once('='))
        .map(|(_, v)| v.to_owned())
        .expect("cookie 應為 name=value 形式");
    assert_eq!(token.len(), 64, "token 應為 32 位元組的 hex");

    let stored: Vec<String> = sqlx::query_scalar!("SELECT id FROM sessions")
        .fetch_all(&pool)
        .await
        .unwrap();

    assert_eq!(stored.len(), 1);
    assert_ne!(stored[0], token, "資料庫不該存 token 本身");
    assert_eq!(stored[0].len(), 64, "應為 SHA-256 的 hex 摘要");

    // 摘要仍然能解析回同一個 session——改存摘要不該讓登入失效
    let (status, _) = send(
        &router,
        authed("GET", "/api/v1/auth/me", &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[sqlx::test]
async fn wrong_password_is_rejected_without_revealing_which_part_failed(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;

    let (status, body) = send(
        &router,
        json_body(
            request("POST", "/api/v1/auth/login"),
            json!({ "username": "tester", "password": "wrong" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "UNAUTHENTICATED");

    // 不存在的帳號回相同訊息
    let (status2, body2) = send(
        &router,
        json_body(
            request("POST", "/api/v1/auth/login"),
            json!({ "username": "nobody", "password": "wrong" }),
        ),
    )
    .await;
    assert_eq!(status2, status);
    assert_eq!(body2["error"]["message"], body["error"]["message"]);
}

#[sqlx::test]
async fn protected_routes_reject_anonymous_requests(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;

    let (status, body) = send(
        &router,
        request("GET", "/api/v1/customers")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "UNAUTHENTICATED");
}

#[sqlx::test]
async fn logout_invalidates_the_session(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let cookie = login(&router).await;

    let (status, _) = send(
        &router,
        authed("GET", "/api/v1/auth/me", &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = send(
        &router,
        authed("POST", "/api/v1/auth/logout", &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = send(
        &router,
        authed("GET", "/api/v1/auth/me", &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "登出後 session 應失效");
}

#[sqlx::test]
async fn me_reports_the_users_permissions(pool: PgPool) {
    let router = setup(&pool, &["sales"]).await;
    let cookie = login(&router).await;

    let (status, body) = send(
        &router,
        authed("GET", "/api/v1/auth/me", &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let perms: Vec<&str> = body["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(perms.contains(&"quotation.write"));
    assert!(
        !perms.contains(&"quotation.cost.read"),
        "業務預設看不到成本"
    );
}

// ==================== CSRF ====================

/// 缺少自訂標頭的寫入請求應被拒絕——跨站的表單送出無法設定自訂標頭。
#[sqlx::test]
async fn write_without_the_custom_header_is_refused(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let cookie = login(&router).await;

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/customers")
        .header(header::HOST, "localhost")
        .header(header::COOKIE, &cookie)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "name": "偽造請求" }).to_string()))
        .unwrap();

    let (status, body) = send(&router, req).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "FORBIDDEN");
}

/// 跨站的 Origin 應被拒絕。
#[sqlx::test]
async fn write_from_a_foreign_origin_is_refused(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let cookie = login(&router).await;

    let req = json_body(
        authed("POST", "/api/v1/customers", &cookie)
            .header(header::ORIGIN, "https://evil.example.com"),
        json!({ "name": "偽造請求" }),
    );

    let (status, _) = send(&router, req).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[sqlx::test]
async fn write_from_the_same_origin_is_allowed(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let cookie = login(&router).await;

    let req = json_body(
        authed("POST", "/api/v1/customers", &cookie).header(header::ORIGIN, "http://localhost"),
        json!({ "name": "正常客戶" }),
    );

    let (status, _) = send(&router, req).await;
    assert_eq!(status, StatusCode::OK);
}

/// 讀取不需要自訂標頭——CSRF 只針對會改變資料的請求。
#[sqlx::test]
async fn reads_do_not_require_the_csrf_header(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let cookie = login(&router).await;

    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/customers")
        .header(header::HOST, "localhost")
        .header(header::COOKIE, &cookie)
        .body(Body::empty())
        .unwrap();

    let (status, _) = send(&router, req).await;
    assert_eq!(status, StatusCode::OK);
}

// ==================== 金額序列化 ====================

/// 建立一個客戶與專案，回傳專案 id。
async fn create_project(router: &axum::Router, cookie: &str) -> i64 {
    let (_, customer) = send(
        router,
        json_body(
            authed("POST", "/api/v1/customers", cookie),
            json!({ "name": "測試客戶" }),
        ),
    )
    .await;

    let (_, project) = send(
        router,
        json_body(
            authed("POST", "/api/v1/projects", cookie),
            json!({ "customer_id": customer["id"], "name": "內湖案場" }),
        ),
    )
    .await;
    project["id"].as_i64().expect("專案應有 id")
}

async fn make_quotation(router: &axum::Router, cookie: &str) -> Value {
    let (_, customer) = send(
        router,
        json_body(
            authed("POST", "/api/v1/customers", cookie),
            json!({ "name": "測試客戶" }),
        ),
    )
    .await;

    let (_, project) = send(
        router,
        json_body(
            authed("POST", "/api/v1/projects", cookie),
            json!({ "customer_id": customer["id"], "name": "內湖案場" }),
        ),
    )
    .await;

    let (status, quotation) = send(
        router,
        json_body(
            authed(
                "POST",
                &format!("/api/v1/projects/{}/quotations", project["id"]),
                cookie,
            ),
            json!({ "lines": [
                { "name": "冷媒銅管", "unit": "米", "quantity": "120", "unit_price": "180", "unit_cost": "120" },
                { "name": "配管施工", "unit": "式", "quantity": "1", "unit_price": "45000" }
            ]}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "建立報價單失敗：{quotation}");
    quotation
}

/// 金額必須是 JSON 字串而非數字。
///
/// 若序列化成 number，前端 `JSON.parse` 會得到 double，
/// 整套 `NUMERIC` / `Decimal` 建立的精度保證會在邊界上蒸發。
#[sqlx::test]
async fn money_is_serialised_as_strings_not_numbers(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let cookie = login(&router).await;
    let q = make_quotation(&router, &cookie).await;

    for field in ["subtotal", "tax_amount", "total", "tax_rate"] {
        assert!(
            q[field].is_string(),
            "{field} 應為字串，實際為 {:?}",
            q[field]
        );
    }
    for field in ["quantity", "unit_price", "amount"] {
        assert!(q["items"][0][field].is_string(), "明細的 {field} 應為字串");
    }

    assert_eq!(q["subtotal"], "66600");
    assert_eq!(q["tax_amount"], "3330");
    assert_eq!(q["total"], "69930");
}

/// 無成本權限者收到的 JSON **根本沒有** `unit_cost` 這個 key，
/// 不是 null 也不是由前端隱藏。
#[sqlx::test]
async fn cost_field_is_absent_without_permission(pool: PgPool) {
    let router = setup(&pool, &["sales"]).await;
    let cookie = login(&router).await;
    let q = make_quotation(&router, &cookie).await;

    let item = &q["items"][0];
    assert!(
        item.get("unit_cost").is_none(),
        "業務不該看到成本欄位，實際回傳：{item}"
    );
    assert!(item.get("unit_price").is_some(), "售價仍應可見");
}

#[sqlx::test]
async fn cost_field_is_present_with_permission(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let cookie = login(&router).await;
    let q = make_quotation(&router, &cookie).await;

    assert_eq!(q["items"][0]["unit_cost"], "120");
}

// ==================== 狀態碼映射 ====================

#[sqlx::test]
async fn approving_a_draft_returns_422_not_500(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let cookie = login(&router).await;
    let q = make_quotation(&router, &cookie).await;

    let (status, body) = send(
        &router,
        authed(
            "POST",
            &format!("/api/v1/quotations/{}/approve", q["id"]),
            &cookie,
        )
        .body(Body::empty())
        .unwrap(),
    )
    .await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["code"], "RULE_VIOLATION");
}

#[sqlx::test]
async fn sales_approving_returns_403(pool: PgPool) {
    let router = setup(&pool, &["sales"]).await;
    let cookie = login(&router).await;
    let q = make_quotation(&router, &cookie).await;

    send(
        &router,
        authed(
            "POST",
            &format!("/api/v1/quotations/{}/submit", q["id"]),
            &cookie,
        )
        .body(Body::empty())
        .unwrap(),
    )
    .await;

    let (status, body) = send(
        &router,
        authed(
            "POST",
            &format!("/api/v1/quotations/{}/approve", q["id"]),
            &cookie,
        )
        .body(Body::empty())
        .unwrap(),
    )
    .await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "FORBIDDEN");
}

#[sqlx::test]
async fn missing_resources_return_404(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let cookie = login(&router).await;

    let (status, body) = send(
        &router,
        authed("GET", "/api/v1/quotations/99999", &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "NOT_FOUND");
}

#[sqlx::test]
async fn invalid_input_returns_field_level_errors(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let cookie = login(&router).await;

    let (status, body) = send(
        &router,
        json_body(
            authed("POST", "/api/v1/customers", &cookie),
            json!({ "name": "   " }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "VALIDATION_FAILED");
    assert_eq!(body["error"]["fields"]["name"], "不可為空");
}

/// 數量為零由 domain 擋下，應回 400 而非 500。
#[sqlx::test]
async fn invalid_quantity_returns_400(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let cookie = login(&router).await;

    let (_, customer) = send(
        &router,
        json_body(
            authed("POST", "/api/v1/customers", &cookie),
            json!({ "name": "客戶" }),
        ),
    )
    .await;
    let (_, project) = send(
        &router,
        json_body(
            authed("POST", "/api/v1/projects", &cookie),
            json!({ "customer_id": customer["id"], "name": "案場" }),
        ),
    )
    .await;

    let (status, body) = send(
        &router,
        json_body(
            authed(
                "POST",
                &format!("/api/v1/projects/{}/quotations", project["id"]),
                &cookie,
            ),
            json!({ "lines": [{ "name": "零", "unit": "式", "quantity": "0", "unit_price": "100" }]}),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "VALIDATION_FAILED");
}

// ==================== 完整流程 ====================

#[sqlx::test]
async fn full_flow_from_quotation_to_contract(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let cookie = login(&router).await;
    let q = make_quotation(&router, &cookie).await;
    let id = q["id"].as_i64().unwrap();

    let (status, _) = send(
        &router,
        authed("POST", &format!("/api/v1/quotations/{id}/submit"), &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, approved) = send(
        &router,
        authed("POST", &format!("/api/v1/quotations/{id}/approve"), &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(approved["status"], "approved");

    let (status, contract) = send(
        &router,
        json_body(
            authed(
                "POST",
                &format!("/api/v1/quotations/{id}/contract"),
                &cookie,
            ),
            json!({ "signed_date": "2026-10-01" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "轉合約失敗：{contract}");
    assert!(contract["contract_no"].as_str().unwrap().starts_with("CT"));
    assert_eq!(contract["total"], approved["total"]);

    // 同一報價不得重複轉約
    let (status, body) = send(
        &router,
        json_body(
            authed(
                "POST",
                &format!("/api/v1/quotations/{id}/contract"),
                &cookie,
            ),
            json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "RULE_VIOLATION");
}

#[sqlx::test]
async fn replacing_items_updates_the_totals_over_http(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let cookie = login(&router).await;
    let q = make_quotation(&router, &cookie).await;

    let (status, updated) = send(
        &router,
        json_body(
            authed(
                "PUT",
                &format!("/api/v1/quotations/{}/items", q["id"]),
                &cookie,
            ),
            json!({ "lines": [
                { "name": "只剩施工", "unit": "式", "quantity": "1", "unit_price": "10000" }
            ]}),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["items"].as_array().unwrap().len(), 1);
    assert_eq!(updated["subtotal"], "10000");
    assert_eq!(updated["tax_amount"], "500");
    assert_eq!(updated["total"], "10500");
}

// ==================== OpenAPI ====================

/// 契約的單一來源是 Rust 型別。這個測試確認規格產得出來，
/// 且金額欄位在規格上是 string——否則產生的 TS 型別會誤導前端寫出浮點運算。
#[test]
fn openapi_declares_money_as_strings() {
    use utoipa::OpenApi;

    let spec = serde_json::to_value(api::ApiDoc::openapi()).unwrap();
    let schemas = &spec["components"]["schemas"];

    for field in ["subtotal", "tax_amount", "total", "tax_rate"] {
        assert_eq!(
            schemas["QuotationResponse"]["properties"][field]["type"], "string",
            "{field} 在 OpenAPI 上應為 string"
        );
    }

    assert!(
        spec["paths"]["/api/v1/quotations/{id}/approve"]["post"].is_object(),
        "核准端點應出現在規格中"
    );
}

// ==================== 帳號管理 ====================

#[sqlx::test]
async fn admin_can_create_a_user_who_can_then_log_in(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let cookie = login(&router).await;

    let (status, created) = send(
        &router,
        json_body(
            authed("POST", "/api/v1/users", &cookie),
            json!({
                "username": "wang",
                "password": "wang-pw-2026",
                "display_name": "王師傅",
                "roles": ["sales"]
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "建立帳號失敗：{created}");
    assert_eq!(created["roles"][0], "sales");

    // 新帳號可實際登入
    let (status, _) = send(
        &router,
        json_body(
            request("POST", "/api/v1/auth/login"),
            json!({ "username": "wang", "password": "wang-pw-2026" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "新建的帳號應可登入");
}

#[sqlx::test]
async fn short_passwords_are_refused(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let cookie = login(&router).await;

    let (status, body) = send(
        &router,
        json_body(
            authed("POST", "/api/v1/users", &cookie),
            json!({ "username": "x", "password": "short", "display_name": "x", "roles": [] }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"]["fields"]["password"].is_string());
}

#[sqlx::test]
async fn unknown_roles_are_refused_with_a_readable_error(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let cookie = login(&router).await;

    let (status, body) = send(
        &router,
        json_body(
            authed("POST", "/api/v1/users", &cookie),
            json!({
                "username": "y", "password": "long-enough-pw",
                "display_name": "y", "roles": ["wizard"]
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "應為驗證錯誤而非 500");
    assert_eq!(body["error"]["fields"]["roles"], "含未知的角色代碼");
}

#[sqlx::test]
async fn duplicate_usernames_return_409(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let cookie = login(&router).await;

    let body = json!({
        "username": "dup", "password": "long-enough-pw",
        "display_name": "重複", "roles": []
    });
    send(
        &router,
        json_body(authed("POST", "/api/v1/users", &cookie), body.clone()),
    )
    .await;
    let (status, _) = send(
        &router,
        json_body(authed("POST", "/api/v1/users", &cookie), body),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[sqlx::test]
async fn sales_cannot_manage_users(pool: PgPool) {
    let router = setup(&pool, &["sales"]).await;
    let cookie = login(&router).await;

    let (status, _) = send(
        &router,
        authed("GET", "/api/v1/users", &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// 停用帳號或改密碼時必須撤銷既有 session，否則對方仍能繼續操作。
#[sqlx::test]
async fn deactivating_a_user_revokes_their_session(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let admin_cookie = login(&router).await;

    let (_, created) = send(
        &router,
        json_body(
            authed("POST", "/api/v1/users", &admin_cookie),
            json!({
                "username": "temp", "password": "temp-pw-2026",
                "display_name": "臨時", "roles": ["viewer"]
            }),
        ),
    )
    .await;

    // 該使用者登入
    let response = router
        .clone()
        .oneshot(json_body(
            request("POST", "/api/v1/auth/login"),
            json!({ "username": "temp", "password": "temp-pw-2026" }),
        ))
        .await
        .unwrap();
    let their_cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();

    let (status, _) = send(
        &router,
        authed("GET", "/api/v1/auth/me", &their_cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 管理員停用該帳號
    let (status, _) = send(
        &router,
        json_body(
            authed(
                "PATCH",
                &format!("/api/v1/users/{}", created["id"]),
                &admin_cookie,
            ),
            json!({ "is_active": false }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = send(
        &router,
        authed("GET", "/api/v1/auth/me", &their_cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "停用後既有 session 應失效"
    );
}

/// 停用自己會把自己鎖在系統外，而且可能是最後一個管理員。
#[sqlx::test]
async fn admin_cannot_deactivate_themselves(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let cookie = login(&router).await;

    let (_, me) = send(
        &router,
        authed("GET", "/api/v1/auth/me", &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;

    let (status, body) = send(
        &router,
        json_body(
            authed("PATCH", &format!("/api/v1/users/{}", me["id"]), &cookie),
            json!({ "is_active": false }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["fields"]["is_active"], "不可停用自己");
}

// ==================== 公司資料 ====================

#[sqlx::test]
async fn company_profile_is_readable_by_anyone_but_writable_only_by_settings_manager(pool: PgPool) {
    let router = setup(&pool, &["sales"]).await;
    let cookie = login(&router).await;

    // 列印報價單需要抬頭，因此任何登入者皆可讀
    let (status, profile) = send(
        &router,
        authed("GET", "/api/v1/company", &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(profile["quote_terms"]
        .as_str()
        .unwrap()
        .contains("報價有效期限"));

    // 但業務不能改
    let (status, _) = send(
        &router,
        json_body(
            authed("PUT", "/api/v1/company", &cookie),
            json!({ "name": "亂改", "quote_terms": "" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[sqlx::test]
async fn admin_can_fill_in_the_company_header(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let cookie = login(&router).await;

    let (status, _) = send(
        &router,
        json_body(
            authed("PUT", "/api/v1/company", &cookie),
            json!({
                "name": "〇〇空調工程有限公司",
                "tax_id": "12345678",
                "phone": "02-1234-5678",
                "quote_terms": "一、報價有效期限三十日。"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_, profile) = send(
        &router,
        authed("GET", "/api/v1/company", &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(profile["name"], "〇〇空調工程有限公司");
    assert_eq!(profile["tax_id"], "12345678");
}

/// 帶腳本的 Logo 必須在寫入資料庫前被擋下。
///
/// 公司資料任何登入者皆可讀取，所以一旦惡意 SVG 進得了資料庫，
/// 就會在每個開啟「系統設定」的人（含 admin）的瀏覽器裡執行，
/// 而同源腳本能繞過全部三道 CSRF 防護。畫面已改用 `<img>` 渲染，
/// 這裡是第二道：不讓它進得了資料庫。
#[sqlx::test]
async fn a_logo_carrying_script_is_rejected(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let cookie = login(&router).await;

    for payload in [
        r#"<img src=x onerror="fetch('/api/v1/users',{method:'POST'})">"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg" onload="alert(1)"></svg>"#,
        r#"<svg><use xlink:href="/etc/passwd"/></svg>"#,
    ] {
        let (status, body) = send(
            &router,
            json_body(
                authed("PUT", "/api/v1/company", &cookie),
                json!({ "name": "測試", "logo_svg": payload, "quote_terms": "" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "應被拒絕：{payload}");
        assert!(
            body["error"]["fields"]["logo_svg"].is_string(),
            "應指出是哪個欄位的問題，實際：{body}"
        );
    }

    // 專案自己的 logo 必須通得過，否則驗證把合法用途一起擋掉了
    let real = include_str!("../../../assets/brand/logo-elbow.svg");
    let (status, _) = send(
        &router,
        json_body(
            authed("PUT", "/api/v1/company", &cookie),
            json!({ "name": "測試", "logo_svg": real, "quote_terms": "" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[sqlx::test]
async fn permissions_endpoint_lists_every_defined_permission(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let cookie = login(&router).await;

    let (status, body) = send(
        &router,
        authed("GET", "/api/v1/permissions", &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let perms: Vec<&str> = body
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(perms.contains(&"quotation.cost.read"));
    assert!(perms.contains(&"user.manage"));
}

#[sqlx::test]
async fn roles_endpoint_reports_seeded_roles_with_their_permissions(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let cookie = login(&router).await;

    let (status, body) = send(
        &router,
        authed("GET", "/api/v1/roles", &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let roles = body.as_array().unwrap();
    assert_eq!(roles.len(), 6);

    let sales = roles.iter().find(|r| r["code"] == "sales").unwrap();
    assert_eq!(sales["is_system"], true);
    let perms: Vec<&str> = sales["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        !perms.contains(&"quotation.cost.read"),
        "業務預設不含成本權限"
    );
}

// ==================== 錯誤格式契約 ====================

/// 所有錯誤回應都必須在 `{"error":{…}}` 信封裡。
///
/// 原本有五類 extractor rejection 繞過信封並回 `text/plain`，
/// 前端的 `JSON.parse` 因此直接拋 SyntaxError，使用者看到的是
/// 瀏覽器產的英文訊息。最常觸發的不是欄位拼錯，而是**數量欄位
/// 多打一個空格**——金額以字串解析成 Decimal，失敗走的就是這條路。
#[sqlx::test]
async fn every_rejection_uses_the_error_envelope(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let cookie = login(&router).await;
    let project_id = create_project(&router, &cookie).await;

    let quotation_path = format!("/api/v1/projects/{project_id}/quotations");

    // 1. 數量欄位有空白 —— 日常路徑
    let (status, body) = send(
        &router,
        json_body(
            authed("POST", &quotation_path, &cookie),
            json!({ "lines": [{
                "name": "配管", "unit": "式",
                "quantity": " 1 ", "unit_price": "1000"
            }]}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "實際回應：{body}");
    assert_eq!(body["error"]["code"], "VALIDATION_FAILED");
    assert!(body["error"]["fields"]["body"].is_string());

    // 2. 未知欄位
    let (status, body) = send(
        &router,
        json_body(
            authed("POST", &quotation_path, &cookie),
            json!({ "lines": [], "unknownField": 1 }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "VALIDATION_FAILED");

    // 3. 路徑參數不是數字
    let (status, body) = send(
        &router,
        authed("GET", "/api/v1/quotations/abc", &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "VALIDATION_FAILED");

    // 4. 查詢參數不是數字
    let (status, body) = send(
        &router,
        authed("GET", "/api/v1/customers?limit=abc", &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "VALIDATION_FAILED");

    // 5. 打錯的 API 路徑要回 404 信封，不該落到 SPA 的靜態檔 fallback
    let (status, body) = send(
        &router,
        authed("GET", "/api/v1/customer/1", &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "實際回應：{body}");
    assert_eq!(body["error"]["code"], "NOT_FOUND");
}

/// 數量與單價各自合法、乘積卻寫不進 `numeric(14,4)` 時要回 400 而非 500。
#[sqlx::test]
async fn an_amount_too_large_for_the_column_is_a_validation_error(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let cookie = login(&router).await;
    let project_id = create_project(&router, &cookie).await;

    // 100 × 1e8 = 1e10，欄位上限是 9,999,999,999
    let (status, body) = send(
        &router,
        json_body(
            authed(
                "POST",
                &format!("/api/v1/projects/{project_id}/quotations"),
                &cookie,
            ),
            json!({ "lines": [{
                "name": "配管", "unit": "式",
                "quantity": "100", "unit_price": "100000000"
            }]}),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "使用者輸入過大不該是 500，實際：{body}"
    );
    assert_eq!(body["error"]["code"], "VALIDATION_FAILED");
}

/// `PATCH /customers/{id}` 只送 name 不該把其他欄位清成 NULL。
#[sqlx::test]
async fn patching_a_customer_leaves_untouched_fields_alone(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let cookie = login(&router).await;

    let (_, created) = send(
        &router,
        json_body(
            authed("POST", "/api/v1/customers", &cookie),
            json!({
                "name": "原始客戶", "tax_id": "12345678",
                "phone": "02-1111-2222", "address": "台北市內湖區"
            }),
        ),
    )
    .await;
    let id = created["id"].as_i64().unwrap();

    let (status, patched) = send(
        &router,
        json_body(
            authed("PATCH", &format!("/api/v1/customers/{id}"), &cookie),
            json!({ "name": "改名後" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "實際：{patched}");
    assert_eq!(patched["name"], "改名後");
    assert_eq!(patched["tax_id"], "12345678", "統編不該被清掉");
    assert_eq!(patched["phone"], "02-1111-2222");
    assert_eq!(patched["address"], "台北市內湖區");

    // 明確給 null 才是清空
    let (_, cleared) = send(
        &router,
        json_body(
            authed("PATCH", &format!("/api/v1/customers/{id}"), &cookie),
            json!({ "phone": null }),
        ),
    )
    .await;
    assert!(cleared["phone"].is_null(), "明確給 null 應清空");
    assert_eq!(cleared["tax_id"], "12345678", "其他欄位仍不受影響");

    // 名稱不可被改成空白
    let (status, _) = send(
        &router,
        json_body(
            authed("PATCH", &format!("/api/v1/customers/{id}"), &cookie),
            json!({ "name": "   " }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// 不合法的狀態轉換要回 422 加 `RULE_VIOLATION`，不是 500。
///
/// 註：真正的樂觀鎖衝突（`lock_version` 失配 → 409 `CONFLICT`）需要兩個
/// 並行交易才測得到，單執行緒的 router 測試做不出來——`service` 的
/// `transition` 在同一個交易內重讀，永遠拿到最新值。那條路徑目前只有
/// DB 層的 `status_update_respects_the_optimistic_lock` 覆蓋，
/// 而前端的衝突提示依賴 `CONFLICT` 這個 code，是個已知的測試缺口。
#[sqlx::test]
async fn an_illegal_transition_returns_422_not_500(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let cookie = login(&router).await;
    let project_id = create_project(&router, &cookie).await;

    let (_, created) = send(
        &router,
        json_body(
            authed(
                "POST",
                &format!("/api/v1/projects/{project_id}/quotations"),
                &cookie,
            ),
            json!({ "lines": [{
                "name": "配管", "unit": "式",
                "quantity": "1", "unit_price": "1000"
            }]}),
        ),
    )
    .await;
    let id = created["id"].as_i64().unwrap();

    let (status, _) = send(
        &router,
        authed("POST", &format!("/api/v1/quotations/{id}/submit"), &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 第二次送審：狀態已是 pending，轉換不合法 → 422 RuleViolation
    let (status, body) = send(
        &router,
        authed("POST", &format!("/api/v1/quotations/{id}/submit"), &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "實際：{body}");
    assert_eq!(body["error"]["code"], "RULE_VIOLATION");
}

/// 空白報價單不可送審。
#[sqlx::test]
async fn an_empty_quotation_cannot_be_submitted(pool: PgPool) {
    let router = setup(&pool, &["admin"]).await;
    let cookie = login(&router).await;
    let project_id = create_project(&router, &cookie).await;

    let (_, created) = send(
        &router,
        json_body(
            authed(
                "POST",
                &format!("/api/v1/projects/{project_id}/quotations"),
                &cookie,
            ),
            json!({ "lines": [] }),
        ),
    )
    .await;
    let id = created["id"].as_i64().unwrap();

    let (status, body) = send(
        &router,
        authed("POST", &format!("/api/v1/quotations/{id}/submit"), &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "實際：{body}");
    assert_eq!(body["error"]["code"], "RULE_VIOLATION");
}
