//! 客戶 handler。
//!
//! 單表 CRUD，依 G2 的規則允許直接呼叫 `db` 而不經 `service`；
//! 建立時要取號，故該處自行開交易。

use axum::extract::State;
use axum::Json;

use acms_domain::{CustomerId, Permission};

use crate::api::dto::{
    CreateCustomerRequest, CustomerResponse, PageQuery, PageResponse, UpdateCustomerRequest,
};
use crate::api::{ApiError, ApiJson, ApiPath, ApiQuery, AppState, CsrfGuard, CurrentUser};
use crate::db;

#[utoipa::path(
    get, path = "/api/v1/customers",
    params(("limit" = Option<i64>, Query), ("offset" = Option<i64>, Query)),
    responses((status = 200, body = PageResponse<CustomerResponse>)),
    tag = "customers",
)]
pub async fn list(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiQuery(page): ApiQuery<PageQuery>,
) -> Result<Json<PageResponse<CustomerResponse>>, ApiError> {
    user.permissions
        .require(Permission::CustomerRead)
        .map_err(|d| ApiError::forbidden(d.to_string()))?;

    let page = db::Page::new(page.limit, page.offset);
    let items = db::customer::list(&state.pool, page).await?;
    let total = db::customer::count(&state.pool).await?;

    Ok(Json(PageResponse {
        items: items.into_iter().map(Into::into).collect(),
        total,
        limit: page.limit,
        offset: page.offset,
    }))
}

#[utoipa::path(
    get, path = "/api/v1/customers/{id}",
    params(("id" = i64, Path)),
    responses((status = 200, body = CustomerResponse), (status = 404)),
    tag = "customers",
)]
pub async fn detail(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiPath(id): ApiPath<i64>,
) -> Result<Json<CustomerResponse>, ApiError> {
    user.permissions
        .require(Permission::CustomerRead)
        .map_err(|d| ApiError::forbidden(d.to_string()))?;

    db::customer::find(&state.pool, CustomerId(id))
        .await?
        .map(|c| Json(c.into()))
        .ok_or_else(|| ApiError::from(crate::service::ServiceError::NotFound { entity: "客戶" }))
}

#[utoipa::path(
    post, path = "/api/v1/customers",
    request_body = CreateCustomerRequest,
    responses((status = 200, body = CustomerResponse), (status = 403)),
    tag = "customers",
)]
pub async fn create(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiJson(body): ApiJson<CreateCustomerRequest>,
) -> Result<Json<CustomerResponse>, ApiError> {
    user.permissions
        .require(Permission::CustomerWrite)
        .map_err(|d| ApiError::forbidden(d.to_string()))?;

    if body.name.trim().is_empty() {
        return Err(ApiError::validation(
            "請填寫客戶名稱",
            [("name".to_owned(), "不可為空".to_owned())].into(),
        ));
    }

    // 取號與建立必須同交易：失敗時單號不該被消耗
    let mut tx = state.pool.begin().await?;
    let year = time::OffsetDateTime::now_utc().year() as i16;
    let code = db::numbering::next(&mut *tx, db::numbering::NumberKind::Customer, year).await?;

    let customer = db::customer::insert(
        &mut *tx,
        db::customer::NewCustomer {
            code,
            name: body.name.trim().to_owned(),
            tax_id: body.tax_id,
            phone: body.phone,
            address: body.address,
            note: body.note,
        },
    )
    .await?;

    db::audit::record(
        &mut *tx,
        Some(user.id),
        "customer",
        customer.id.0,
        "create",
        None,
    )
    .await?;

    tx.commit().await?;
    Ok(Json(customer.into()))
}

#[utoipa::path(
    patch, path = "/api/v1/customers/{id}",
    params(("id" = i64, Path)),
    request_body = UpdateCustomerRequest,
    responses((status = 200, body = CustomerResponse), (status = 404)),
    tag = "customers",
)]
pub async fn update(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    ApiPath(id): ApiPath<i64>,
    ApiJson(body): ApiJson<UpdateCustomerRequest>,
) -> Result<Json<CustomerResponse>, ApiError> {
    user.permissions
        .require(Permission::CustomerWrite)
        .map_err(|d| ApiError::forbidden(d.to_string()))?;

    // 與 create 同一套正規化：trim 後不得為空。
    // 少了這道，客戶名稱可被改成空白，之後列表與報價單抬頭都是空的。
    let name = match body.name {
        None => None,
        Some(name) if name.trim().is_empty() => {
            return Err(ApiError::validation(
                "請填寫客戶名稱",
                [("name".to_owned(), "不可為空".to_owned())].into(),
            ))
        }
        Some(name) => Some(name.trim().to_owned()),
    };

    let patch = db::customer::CustomerPatch {
        name,
        tax_id: body.tax_id,
        phone: body.phone,
        address: body.address,
        note: body.note,
        is_active: None,
    };

    // 客戶名稱與統編會印在報價單與合約上，改動要留軌跡——
    // create、專案、帳號都有稽核，這條路徑原本沒有。
    let mut tx = state.pool.begin().await?;
    let updated = db::customer::update(&mut *tx, CustomerId(id), patch)
        .await?
        .ok_or_else(|| {
            ApiError::from(crate::service::ServiceError::NotFound { entity: "客戶" })
        })?;

    db::audit::record(
        &mut *tx,
        Some(user.id),
        "customer",
        updated.id.0,
        "update",
        None,
    )
    .await?;
    tx.commit().await?;

    Ok(Json(updated.into()))
}
