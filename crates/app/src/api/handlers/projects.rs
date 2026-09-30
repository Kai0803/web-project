//! 專案 handler。

use axum::extract::{Path, Query, State};
use axum::Json;

use acms_domain::{CustomerId, Permission, ProjectId};

use crate::api::dto::{
    CreateProjectRequest, PageQuery, PageResponse, ProjectListItem, ProjectResponse,
};
use crate::api::{ApiError, AppState, CsrfGuard, CurrentUser};
use crate::db;
use crate::service::ServiceError;

#[utoipa::path(
    get, path = "/api/v1/projects",
    params(("limit" = Option<i64>, Query), ("offset" = Option<i64>, Query)),
    responses((status = 200, body = PageResponse<ProjectListItem>)),
    tag = "projects",
)]
pub async fn list(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(page): Query<PageQuery>,
) -> Result<Json<PageResponse<ProjectListItem>>, ApiError> {
    user.permissions
        .require(Permission::ProjectRead)
        .map_err(|d| ApiError::forbidden(d.to_string()))?;

    let page = db::Page::new(page.limit, page.offset);
    let items = db::project::list(&state.pool, None, page).await?;
    let total = db::project::count(&state.pool, None).await?;

    Ok(Json(PageResponse {
        items: items.into_iter().map(Into::into).collect(),
        total,
        limit: page.limit,
        offset: page.offset,
    }))
}

#[utoipa::path(
    get, path = "/api/v1/projects/{id}",
    params(("id" = i64, Path)),
    responses((status = 200, body = ProjectResponse), (status = 404)),
    tag = "projects",
)]
pub async fn detail(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Json<ProjectResponse>, ApiError> {
    user.permissions
        .require(Permission::ProjectRead)
        .map_err(|d| ApiError::forbidden(d.to_string()))?;

    db::project::find(&state.pool, ProjectId(id))
        .await?
        .map(|p| Json(p.into()))
        .ok_or_else(|| ApiError::from(ServiceError::NotFound { entity: "專案" }))
}

#[utoipa::path(
    post, path = "/api/v1/projects",
    request_body = CreateProjectRequest,
    responses((status = 200, body = ProjectResponse), (status = 404, description = "找不到客戶")),
    tag = "projects",
)]
pub async fn create(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    Json(body): Json<CreateProjectRequest>,
) -> Result<Json<ProjectResponse>, ApiError> {
    user.permissions
        .require(Permission::ProjectWrite)
        .map_err(|d| ApiError::forbidden(d.to_string()))?;

    if body.name.trim().is_empty() {
        return Err(ApiError::validation(
            "請填寫案場名稱",
            [("name".to_owned(), "不可為空".to_owned())].into(),
        ));
    }

    let mut tx = state.pool.begin().await?;

    if db::customer::find(&mut *tx, CustomerId(body.customer_id))
        .await?
        .is_none()
    {
        return Err(ApiError::from(ServiceError::NotFound { entity: "客戶" }));
    }

    let year = time::OffsetDateTime::now_utc().year() as i16;
    let code = db::numbering::next(&mut *tx, db::numbering::NumberKind::Project, year).await?;

    let project = db::project::insert(
        &mut *tx,
        db::project::NewProject {
            code,
            customer_id: CustomerId(body.customer_id),
            name: body.name.trim().to_owned(),
            site_address: body.site_address,
            manager_id: None,
            expected_start: None,
            expected_end: None,
            note: body.note,
        },
    )
    .await?;

    db::audit::record(
        &mut *tx,
        Some(user.id),
        "project",
        project.id.0,
        "create",
        None,
    )
    .await?;

    tx.commit().await?;
    Ok(Json(project.into()))
}
