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

async fn send(router: &axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    let response = router.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
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
