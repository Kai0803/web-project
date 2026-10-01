//! 報價單 PDF 下載。

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

use acms_domain::{Permission, QuotationId};

use crate::api::{ApiError, AppState, CurrentUser};
use crate::service::ServiceError;
use crate::{db, pdf};

#[utoipa::path(
    get, path = "/api/v1/quotations/{id}/pdf",
    params(("id" = i64, Path)),
    responses(
        (status = 200, description = "報價單 PDF", content_type = "application/pdf"),
        (status = 403, description = "權限不足"),
        (status = 404),
    ),
    tag = "quotations",
)]
pub async fn quotation_pdf(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
    user.permissions
        .require(Permission::QuotationRead)
        .map_err(|d| ApiError::forbidden(d.to_string()))?;

    let id = QuotationId(id);
    let quotation = db::quotation::find(&state.pool, id)
        .await?
        .ok_or(ServiceError::NotFound {
            entity: "報價單"
        })?;
    let items = db::quotation::items(&state.pool, id).await?;
    let project = db::project::find(&state.pool, quotation.project_id)
        .await?
        .ok_or(ServiceError::NotFound { entity: "專案" })?;
    let customer = db::customer::find(&state.pool, project.customer_id)
        .await?
        .ok_or(ServiceError::NotFound { entity: "客戶" })?;
    let company = db::company::get(&state.pool).await?;

    let filename = format!("{}-v{}.pdf", quotation.quote_no, quotation.version);

    // 排版是 CPU 密集工作，丟到 blocking pool，不佔住 async worker
    let bytes = tokio::task::spawn_blocking(move || {
        pdf::render(&company, &customer, &project, &quotation, &items)
    })
    .await
    .map_err(|err| ApiError::internal("pdf join", err))?
    .map_err(|err| match err {
        // 缺字型是部署環境的問題，訊息要能指出該修哪裡
        pdf::PdfError::NoCjkFont => ApiError::internal("pdf font", err),
        other => ApiError::internal("pdf", other),
    })?;

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/pdf".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("inline; filename=\"{filename}\""),
            ),
        ],
        Body::from(bytes),
    )
        .into_response())
}
