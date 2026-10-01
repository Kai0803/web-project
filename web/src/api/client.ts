/**
 * API client。
 *
 * 型別全部來自 `schema.d.ts`，而那是由後端的 Rust 型別產生的——
 * 前後端不各自手寫一份契約。改了後端的 DTO 卻沒重新產生型別時，
 * `npm run typecheck` 會失敗。
 *
 * 金額在型別上是 `string`，因此前端不可能不小心對它做浮點運算。
 * 所有試算一律打後端，保證與列印出來的數字一致。
 */
import type { components } from './schema'

export type Schemas = components['schemas']

export type Me = Schemas['MeResponse']
export type Customer = Schemas['CustomerResponse']
export type Project = Schemas['ProjectResponse']
export type ProjectListItem = Schemas['ProjectListItem']
export type Quotation = Schemas['QuotationResponse']
export type QuotationDetail = Schemas['QuotationDetailResponse']
export type QuotationItem = Schemas['QuotationItemResponse']
export type LineRequest = Schemas['LineRequest']
export type Contract = Schemas['ContractResponse']
export type UserSummary = Schemas['UserSummary']
export type RoleSummary = Schemas['RoleSummary']
export type CompanyProfile = Schemas['CompanyProfileBody']
export type ErrorCode = Schemas['ErrorCode']

/** 後端回傳的錯誤。`code` 是列舉，前端可據此分流而不必比對訊息字串。 */
export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: ErrorCode | 'NETWORK',
    message: string,
    readonly fields?: Record<string, string>,
  ) {
    super(message)
    this.name = 'ApiError'
  }

  /** 表單可直接取用的欄位錯誤。 */
  fieldError(name: string): string | undefined {
    return this.fields?.[name]
  }
}

type Method = 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE'

async function request<T>(method: Method, path: string, body?: unknown): Promise<T> {
  let response: Response
  try {
    response = await fetch(path, {
      method,
      credentials: 'same-origin',
      headers: {
        // CSRF 防護：跨站的表單送出設不了自訂標頭，後端據此拒絕
        'X-Requested-With': 'XMLHttpRequest',
        ...(body === undefined ? {} : { 'Content-Type': 'application/json' }),
      },
      body: body === undefined ? undefined : JSON.stringify(body),
    })
  } catch {
    throw new ApiError(0, 'NETWORK', '無法連線至伺服器，請確認網路狀態')
  }

  if (response.status === 204) return undefined as T

  const text = await response.text()

  // 不假設回應一定是 JSON。SPA 的靜態檔 fallback 會回 HTML、反向代理的
  // 錯誤頁是 HTML、任何繞過 ApiError 的 middleware 可能回 text/plain。
  // 在這裡讓 JSON.parse 直接拋，使用者看到的會是瀏覽器產的英文
  // SyntaxError（`Unexpected token 'F'…`），而不是我們寫的訊息。
  let payload: unknown = null
  if (text) {
    try {
      payload = JSON.parse(text)
    } catch {
      payload = null
    }
  }

  if (!response.ok) {
    const err = (payload as { error?: { code: ErrorCode; message: string; fields?: Record<string, string> } })?.error
    throw new ApiError(
      response.status,
      err?.code ?? 'INTERNAL',
      err?.message ?? `請求失敗（${response.status}）`,
      err?.fields,
    )
  }

  // 成功卻解不出 JSON，代表拿到的不是 API 的回應（最常見的是打錯路徑
  // 而被 SPA fallback 接走）。讓它成為一個可讀的錯誤，而不是後續
  // 存取 undefined 欄位時才在別處炸開。
  if (payload === null && text) {
    throw new ApiError(response.status, 'INTERNAL', '伺服器回應格式不正確')
  }

  return payload as T
}

export const api = {
  // 認證
  login: (username: string, password: string) =>
    request<void>('POST', '/api/v1/auth/login', { username, password }),
  logout: () => request<void>('POST', '/api/v1/auth/logout'),
  me: () => request<Me>('GET', '/api/v1/auth/me'),

  // 客戶
  listCustomers: () =>
    request<{ items: Customer[]; total: number }>('GET', '/api/v1/customers?limit=200'),
  createCustomer: (body: Schemas['CreateCustomerRequest']) =>
    request<Customer>('POST', '/api/v1/customers', body),

  // 專案
  listProjects: () =>
    request<{ items: ProjectListItem[]; total: number }>('GET', '/api/v1/projects?limit=200'),
  getProject: (id: number) => request<Project>('GET', `/api/v1/projects/${id}`),
  createProject: (body: Schemas['CreateProjectRequest']) =>
    request<Project>('POST', '/api/v1/projects', body),

  // 報價單
  listQuotations: (projectId: number) =>
    request<{ items: Quotation[] }>('GET', `/api/v1/projects/${projectId}/quotations`),
  getQuotation: (id: number) => request<QuotationDetail>('GET', `/api/v1/quotations/${id}`),
  createQuotation: (projectId: number, lines: LineRequest[], note?: string) =>
    request<QuotationDetail>('POST', `/api/v1/projects/${projectId}/quotations`, { lines, note }),
  replaceItems: (id: number, lines: LineRequest[]) =>
    request<QuotationDetail>('PUT', `/api/v1/quotations/${id}/items`, { lines }),
  submit: (id: number) => request<Quotation>('POST', `/api/v1/quotations/${id}/submit`),
  approve: (id: number) => request<Quotation>('POST', `/api/v1/quotations/${id}/approve`),
  reject: (id: number) => request<Quotation>('POST', `/api/v1/quotations/${id}/reject`),
  reopen: (id: number) => request<Quotation>('POST', `/api/v1/quotations/${id}/reopen`),
  newVersion: (id: number) => request<QuotationDetail>('POST', `/api/v1/quotations/${id}/versions`),
  toContract: (id: number, signedDate?: string) =>
    request<Contract>('POST', `/api/v1/quotations/${id}/contract`, { signed_date: signedDate }),

  // 合約
  getContract: (id: number) => request<Contract>('GET', `/api/v1/contracts/${id}`),

  // 帳號與設定
  listUsers: () => request<UserSummary[]>('GET', '/api/v1/users'),
  createUser: (body: Schemas['CreateUserRequest']) =>
    request<UserSummary>('POST', '/api/v1/users', body),
  updateUser: (id: number, body: Schemas['UpdateUserRequest']) =>
    request<void>('PATCH', `/api/v1/users/${id}`, body),
  listRoles: () => request<RoleSummary[]>('GET', '/api/v1/roles'),
  getCompany: () => request<CompanyProfile>('GET', '/api/v1/company'),
  updateCompany: (body: CompanyProfile) => request<void>('PUT', '/api/v1/company', body),
}

/**
 * 金額顯示。輸入是後端給的字串，只做加分位符號，不做任何運算。
 *
 * 若在此處 `Number(value)` 再格式化，就等於把精度保證丟掉——
 * 這正是整條鏈刻意用字串傳遞金額的原因。
 */
export function formatMoney(value: string): string {
  const [whole = '0', fraction] = value.split('.')
  const sign = whole.startsWith('-') ? '-' : ''
  const digits = sign ? whole.slice(1) : whole
  const grouped = digits.replace(/\B(?=(\d{3})+(?!\d))/g, ',')
  return fraction ? `${sign}${grouped}.${fraction}` : `${sign}${grouped}`
}
