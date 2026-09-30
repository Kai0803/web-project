//! 報價單 handler。
//!
//! 全部經 `service` 層——每個動作都涉及交易或跨表。

use axum::extract::{Path, Query, State};
use axum::Json;

use acms_domain::{authz, ProjectId, QuotationId};

use crate::api::dto::{
    CreateQuotationRequest, LineRequest, PageQuery, PageResponse, QuotationDetailResponse,
    QuotationItemResponse, QuotationResponse, ReplaceItemsRequest,
};
use crate::api::{ApiError, AppState, CsrfGuard, CurrentUser};
use crate::db;
use crate::service::quotation as svc;

fn to_line_inputs(lines: Vec<LineRequest>) -> Vec<svc::LineInput> {
    lines
        .into_iter()
        .map(|l| svc::LineInput {
            name: l.name,
            spec: l.spec,
            unit: l.unit,
            quantity: l.quantity,
            unit_price: l.unit_price,
            unit_cost: l.unit_cost,
            note: l.note,
        })
        .collect()
}

/// 依權限決定是否保留成本欄位。剝除發生在序列化之前，
/// 無權限者收到的 JSON 裡根本沒有這個欄位。
fn to_detail(detail: svc::QuotationDetail, user: &acms_domain::User) -> QuotationDetailResponse {
    let can_view_cost = authz::can_view_cost(&user.permissions);
    QuotationDetailResponse {
        quotation: detail.quotation.into(),
        items: detail
            .items
            .into_iter()
            .map(|i| QuotationItemResponse::from_item(i, can_view_cost))
            .collect(),
    }
}

#[utoipa::path(
    post, path = "/api/v1/projects/{id}/quotations",
    params(("id" = i64, Path, description = "專案 id")),
    request_body = CreateQuotationRequest,
    responses(
        (status = 200, body = QuotationDetailResponse),
        (status = 403, description = "權限不足"),
        (status = 404, description = "找不到專案"),
    ),
    tag = "quotations",
)]
pub async fn create(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    Path(project_id): Path<i64>,
    Json(body): Json<CreateQuotationRequest>,
) -> Result<Json<QuotationDetailResponse>, ApiError> {
    let lines = to_line_inputs(body.lines);
    let detail = svc::create(&state.pool, &user, ProjectId(project_id), &lines, body.note).await?;
    Ok(Json(to_detail(detail, &user)))
}

#[utoipa::path(
    get, path = "/api/v1/quotations/{id}",
    params(("id" = i64, Path)),
    responses((status = 200, body = QuotationDetailResponse), (status = 404)),
    tag = "quotations",
)]
pub async fn detail(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Json<QuotationDetailResponse>, ApiError> {
    let detail = svc::detail(&state.pool, &user, QuotationId(id)).await?;
    Ok(Json(to_detail(detail, &user)))
}

#[utoipa::path(
    put, path = "/api/v1/quotations/{id}/items",
    params(("id" = i64, Path)),
    request_body = ReplaceItemsRequest,
    responses(
        (status = 200, body = QuotationDetailResponse),
        (status = 422, description = "非草稿狀態不可修改明細"),
    ),
    tag = "quotations",
)]
pub async fn replace_items(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    Path(id): Path<i64>,
    Json(body): Json<ReplaceItemsRequest>,
) -> Result<Json<QuotationDetailResponse>, ApiError> {
    let lines = to_line_inputs(body.lines);
    let detail = svc::replace_items(&state.pool, &user, QuotationId(id), &lines).await?;
    Ok(Json(to_detail(detail, &user)))
}

macro_rules! transition_handler {
    ($name:ident, $svc:ident, $path:literal, $desc:literal) => {
        #[utoipa::path(
                    post, path = $path,
                    params(("id" = i64, Path)),
                    responses(
                        (status = 200, body = QuotationResponse, description = $desc),
                        (status = 403, description = "權限不足"),
                        (status = 409, description = "已有核准版本或資料已被他人修改"),
                        (status = 422, description = "狀態轉換不合法"),
                    ),
                    tag = "quotations",
                )]
        pub async fn $name(
            State(state): State<AppState>,
            CurrentUser(user): CurrentUser,
            _csrf: CsrfGuard,
            Path(id): Path<i64>,
        ) -> Result<Json<QuotationResponse>, ApiError> {
            let q = svc::$svc(&state.pool, &user, QuotationId(id)).await?;
            Ok(Json(q.into()))
        }
    };
}

transition_handler!(submit, submit, "/api/v1/quotations/{id}/submit", "已送審");
transition_handler!(
    approve,
    approve,
    "/api/v1/quotations/{id}/approve",
    "已核准"
);
transition_handler!(reject, reject, "/api/v1/quotations/{id}/reject", "已駁回");
transition_handler!(
    reopen,
    reopen,
    "/api/v1/quotations/{id}/reopen",
    "已退回草稿"
);

#[utoipa::path(
    post, path = "/api/v1/quotations/{id}/versions",
    params(("id" = i64, Path, description = "來源報價單 id")),
    responses(
        (status = 200, body = QuotationDetailResponse, description = "新版本，內容複製自來源"),
        (status = 409, description = "來源已轉合約，不可再改版"),
    ),
    tag = "quotations",
)]
pub async fn new_version(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    Path(id): Path<i64>,
) -> Result<Json<QuotationDetailResponse>, ApiError> {
    let detail = svc::new_version(&state.pool, &user, QuotationId(id)).await?;
    Ok(Json(to_detail(detail, &user)))
}

#[utoipa::path(
    get, path = "/api/v1/projects/{id}/quotations",
    params(("id" = i64, Path), ("limit" = Option<i64>, Query), ("offset" = Option<i64>, Query)),
    responses((status = 200, body = PageResponse<QuotationResponse>)),
    tag = "quotations",
)]
pub async fn list_for_project(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(project_id): Path<i64>,
    Query(page): Query<PageQuery>,
) -> Result<Json<PageResponse<QuotationResponse>>, ApiError> {
    user.permissions
        .require(acms_domain::Permission::QuotationRead)
        .map_err(|d| ApiError::forbidden(d.to_string()))?;

    let page = db::Page::new(page.limit, page.offset);
    let items = db::quotation::list_for_project(&state.pool, ProjectId(project_id), page).await?;

    Ok(Json(PageResponse {
        total: items.len() as i64,
        items: items.into_iter().map(Into::into).collect(),
        limit: page.limit,
        offset: page.offset,
    }))
}
