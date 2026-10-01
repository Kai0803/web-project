//! OpenAPI 規格。
//!
//! 契約的單一來源是 Rust 型別定義：前端由此產生 TypeScript 型別，
//! 前後端不各自手寫一份。
//!
//! 但 utoipa 的標註是人工維護的，可能與 handler 的實際行為不同步。
//! CI 需檢查「重新產生的 openapi.json 與版控中的版本無 diff」，
//! 否則「單一來源」會在三個月內退化成兩份手寫。

use utoipa::OpenApi;

use super::dto;
use super::error;
use super::handlers;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "空調配管工程管理系統 API",
        version = "0.1.0",
        description = "第一階段：客戶、專案、報價單、合約。\n\n\
                       所有金額與數量皆以**字串**傳遞，避免 JSON number 的浮點精度問題。\n\n\
                       認證使用 HttpOnly cookie session；\
                       所有會改變資料的請求需帶 `X-Requested-With` 標頭（CSRF 防護）。"
    ),
    paths(
        handlers::auth::login,
        handlers::auth::logout,
        handlers::auth::me,
        handlers::customers::list,
        handlers::customers::detail,
        handlers::customers::create,
        handlers::customers::update,
        handlers::projects::list,
        handlers::projects::detail,
        handlers::projects::create,
        handlers::quotations::create,
        handlers::quotations::detail,
        handlers::quotations::list_for_project,
        handlers::quotations::replace_items,
        handlers::quotations::submit,
        handlers::quotations::approve,
        handlers::quotations::reject,
        handlers::quotations::reopen,
        handlers::quotations::new_version,
        handlers::pdf::quotation_pdf,
        handlers::contracts::create_from_quotation,
        handlers::contracts::detail,
        handlers::admin::list_users,
        handlers::admin::create_user,
        handlers::admin::update_user,
        handlers::admin::list_roles,
        handlers::admin::list_permissions,
        handlers::admin::get_company,
        handlers::admin::update_company,
    ),
    components(schemas(
        dto::LoginRequest,
        dto::MeResponse,
        dto::CustomerResponse,
        dto::CreateCustomerRequest,
        dto::ProjectResponse,
        dto::ProjectListItem,
        dto::CreateProjectRequest,
        dto::QuotationResponse,
        dto::QuotationItemResponse,
        dto::QuotationDetailResponse,
        dto::LineRequest,
        dto::CreateQuotationRequest,
        dto::ReplaceItemsRequest,
        dto::ContractResponse,
        dto::CreateContractRequest,
        handlers::admin::UserSummary,
        handlers::admin::CreateUserRequest,
        handlers::admin::UpdateUserRequest,
        handlers::admin::RoleSummary,
        handlers::admin::CompanyProfileBody,
        error::ErrorCode,
        error::ErrorBody,
        error::ErrorResponse,
    )),
    tags(
        (name = "auth", description = "登入與登出"),
        (name = "customers", description = "客戶"),
        (name = "projects", description = "專案 / 案場"),
        (name = "quotations", description = "報價單"),
        (name = "contracts", description = "合約"),
        (name = "admin", description = "帳號、角色與系統設定"),
    ),
)]
pub struct ApiDoc;
