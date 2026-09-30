//! 合約 handler。

use axum::extract::{Path, State};
use axum::Json;

use acms_domain::{ContractId, QuotationId};

use crate::api::dto::{ContractResponse, CreateContractRequest};
use crate::api::{ApiError, AppState, CsrfGuard, CurrentUser};
use crate::service::contract as svc;

#[utoipa::path(
    post, path = "/api/v1/quotations/{id}/contract",
    params(("id" = i64, Path, description = "已核准的報價單 id")),
    request_body = CreateContractRequest,
    responses(
        (status = 200, body = ContractResponse, description = "合約已建立，報價明細已實體化為工項"),
        (status = 409, description = "此報價已轉過合約"),
        (status = 422, description = "報價尚未核准"),
    ),
    tag = "contracts",
)]
pub async fn create_from_quotation(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    _csrf: CsrfGuard,
    Path(id): Path<i64>,
    Json(body): Json<CreateContractRequest>,
) -> Result<Json<ContractResponse>, ApiError> {
    let signed_date = match body.signed_date.as_deref() {
        None => None,
        Some(raw) => Some(
            time::Date::parse(raw, &time::format_description::well_known::Iso8601::DATE).map_err(
                |_| {
                    ApiError::validation(
                        "簽約日期格式不正確",
                        [("signed_date".to_owned(), "請使用 YYYY-MM-DD".to_owned())].into(),
                    )
                },
            )?,
        ),
    };

    let contract =
        svc::create_from_quotation(&state.pool, &user, QuotationId(id), signed_date).await?;
    Ok(Json(contract.into()))
}

#[utoipa::path(
    get, path = "/api/v1/contracts/{id}",
    params(("id" = i64, Path)),
    responses((status = 200, body = ContractResponse), (status = 404)),
    tag = "contracts",
)]
pub async fn detail(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Json<ContractResponse>, ApiError> {
    let contract = svc::find(&state.pool, &user, ContractId(id)).await?;
    Ok(Json(contract.into()))
}
