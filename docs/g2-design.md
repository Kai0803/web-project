# G2 設計草案 — 第一階段：核心（客戶 / 專案 / 報價）

狀態：**草案，待 architecture-reviewer 與 algorithm-advisor 審查**。

對應需求：[G1 需求釐清](./g1-requirements.md)（已確認）。

## 1. 部署拓樸

```
┌─────────────────────── Pod: acms ───────────────────────┐
│  container: app          container: postgres            │
│  ├ Rust / axum           ├ postgres:17-alpine           │
│  ├ 提供 REST API         └ PVC: acms-pgdata             │
│  └ 同時靜態提供 Vue 建置產物                             │
└─────────────────────────────────────────────────────────┘
                    hostPort 8080
```

**前端由後端同源提供**，不另開 web server。理由：

- 同源即無 CORS 問題，少一整類設定錯誤
- 認證可用 `HttpOnly` cookie session，token 不進 `localStorage`，符合資安清單 I 節
- 純內部系統，不需要 CDN 的分流效益

代價：前端建置產物要進 image，每次前端改動都要重 build image。對單人開發可接受。

## 2. 模組邊界（Rust）

單一 crate，不切 workspace。第一階段程式規模不足以撐起多 crate 的管理成本，且 module 的可見性控制已足夠表達邊界。

```
src/
  main.rs         組裝層：讀設定、建連線池、掛路由、啟動
  config.rs       環境變數讀取與啟動時驗證
  error.rs        AppError 型別與 IntoResponse 實作
  domain/         純業務邏輯，不依賴 axum 與 sqlx
    money.rs        金額型別、稅額計算、進位規則
    quotation.rs    報價金額彙總、版本與狀態轉換規則
    ids.rs          newtype 識別字（CustomerId、ProjectId…）
  db/             sqlx 查詢，具體實作，不抽 trait
    customer.rs
    project.rs
    quotation.rs
  api/            axum handler、請求 / 回應 DTO、輸入驗證
  auth/           session、密碼雜湊、權限判斷
```

### 依賴方向

```
api ──► domain ◄── db
 │                  ▲
 └──────────────────┘
main ──► 全部（僅組裝）
```

`domain` 不依賴任何框架或資料庫型別，因此金額計算與狀態轉換規則可以純函式測試，不需要資料庫。

### 為什麼 db 層不抽 trait

架構清單要求外部相依以 trait 注入，但此處刻意不做：唯一的替換情境是換資料庫，而 Postgres 已定案且不會換。抽 trait 只會多一層轉接而換不到任何實際彈性，屬 YAGNI。可測試性由 `domain` 的純函式承擔，`db` 層以整合測試搭配交易回滾驗證。

此決定提請 architecture-reviewer 評估。

## 3. 核心資料結構

### 識別字策略

所有資料表主鍵為 `bigint generated always as identity`。三張未來可能對外曝露的表（`customers`、`projects`、`quotations`）額外有 `public_id uuid not null unique default gen_random_uuid()`。

理由：內部 join 用整數鍵最快也最省，對外 URL 用 uuid 避免流水號被列舉。第一階段不對外，但 `public_id` 事後補上需要回填既有資料並改所有對外路由，成本遠高於現在就加一欄。

### 金額與數量

- 資料庫：`NUMERIC(14,4)`
- Rust：`rust_decimal::Decimal`
- 一律不用浮點數

配管的數量本身就是小數（米、公斤、才），若金額用整數分而數量用小數，每一列小計都要決定在哪個環節進位，容易出錯。全程用 decimal 可以把進位集中在一個明確的地方。

**進位規則**（需明確定義並測試）：

1. 單列小計 `= 數量 × 單價`，保留 4 位小數不進位
2. 未稅總額 `= Σ 單列小計`，四捨五入至整數元
3. 稅額 `= 未稅總額 × 0.05`，四捨五入至整數元
4. 含稅總額 `= 未稅總額 + 稅額`

先加總再進位，而非逐列進位後加總，避免累積誤差。

### 資料表

```sql
-- 建立順序需先於 users，因 users.customer_id 外鍵指向此表
CREATE TABLE customers (
  id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  public_id     uuid NOT NULL UNIQUE DEFAULT gen_random_uuid(),
  code          text NOT NULL UNIQUE,      -- 客戶編號
  name          text NOT NULL,
  tax_id        text,                       -- 統一編號
  phone         text,
  address       text,
  note          text,
  is_active     boolean NOT NULL DEFAULT true,
  created_at    timestamptz NOT NULL DEFAULT now(),
  updated_at    timestamptz NOT NULL DEFAULT now()
);

-- 使用者。kind 區分內部同仁與客戶聯絡人，為第二 / 三階段的客戶入口預留
CREATE TYPE user_kind AS ENUM ('staff', 'customer_contact');

CREATE TABLE users (
  id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  kind          user_kind NOT NULL DEFAULT 'staff',
  customer_id   bigint REFERENCES customers(id),  -- 僅 customer_contact 有值
  username      text NOT NULL UNIQUE,
  password_hash text NOT NULL,
  display_name  text NOT NULL,
  role          text NOT NULL,
  is_active     boolean NOT NULL DEFAULT true,
  created_at    timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT customer_contact_has_customer
    CHECK ((kind = 'customer_contact') = (customer_id IS NOT NULL))
);

CREATE TABLE customer_contacts (
  id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  customer_id   bigint NOT NULL REFERENCES customers(id) ON DELETE CASCADE,
  name          text NOT NULL,
  title         text,
  phone         text,
  email         text,
  is_primary    boolean NOT NULL DEFAULT false
);

-- 每個客戶至多一位主要聯絡人，由資料庫保證
CREATE UNIQUE INDEX one_primary_contact_per_customer
  ON customer_contacts (customer_id) WHERE is_primary;

CREATE TABLE projects (
  id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  public_id     uuid NOT NULL UNIQUE DEFAULT gen_random_uuid(),
  code          text NOT NULL UNIQUE,      -- 案號
  customer_id   bigint NOT NULL REFERENCES customers(id),
  name          text NOT NULL,
  site_address  text,
  status        text NOT NULL,             -- lead / quoting / contracted / closed / cancelled
  manager_id    bigint REFERENCES users(id),
  expected_start date,
  expected_end   date,
  note          text,
  created_at    timestamptz NOT NULL DEFAULT now(),
  updated_at    timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX ON projects (customer_id);
CREATE INDEX ON projects (status);

-- 報價單。一個 quote_no 有多個 version，每個版本一列
CREATE TABLE quotations (
  id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  public_id     uuid NOT NULL UNIQUE DEFAULT gen_random_uuid(),
  project_id    bigint NOT NULL REFERENCES projects(id),
  quote_no      text NOT NULL,             -- 人類可讀單號，跨版本相同
  version       integer NOT NULL,
  status        text NOT NULL,             -- draft / pending / approved / rejected / superseded
  valid_until   date,
  subtotal      numeric(14,4) NOT NULL DEFAULT 0,  -- 未稅
  tax_amount    numeric(14,4) NOT NULL DEFAULT 0,
  total         numeric(14,4) NOT NULL DEFAULT 0,  -- 含稅
  created_by    bigint NOT NULL REFERENCES users(id),
  approved_by   bigint REFERENCES users(id),
  approved_at   timestamptz,
  note          text,
  created_at    timestamptz NOT NULL DEFAULT now(),
  updated_at    timestamptz NOT NULL DEFAULT now(),
  UNIQUE (quote_no, version)
);

CREATE INDEX ON quotations (project_id);

-- 同一報價單至多一個核准版本，由資料庫保證
CREATE UNIQUE INDEX one_approved_version_per_quote
  ON quotations (quote_no) WHERE status = 'approved';

CREATE TABLE quotation_items (
  id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  quotation_id  bigint NOT NULL REFERENCES quotations(id) ON DELETE CASCADE,
  line_no       integer NOT NULL,
  name          text NOT NULL,             -- 快照，不外鍵至未來的料件主檔
  spec          text,
  unit          text NOT NULL,
  quantity      numeric(14,4) NOT NULL,
  unit_price    numeric(14,4) NOT NULL,
  amount        numeric(14,4) NOT NULL,    -- 數量 × 單價，寫入時計算
  note          text,
  UNIQUE (quotation_id, line_no)
);

CREATE INDEX ON quotation_items (quotation_id);

CREATE TABLE contracts (
  id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  project_id    bigint NOT NULL REFERENCES projects(id),
  quotation_id  bigint NOT NULL UNIQUE REFERENCES quotations(id),
  contract_no   text NOT NULL UNIQUE,
  signed_date   date,
  total         numeric(14,4) NOT NULL,
  created_at    timestamptz NOT NULL DEFAULT now()
);

-- 變更稽核。報價金額為敏感資料，需可追溯誰改了什麼
CREATE TABLE audit_logs (
  id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  actor_id      bigint REFERENCES users(id),
  entity        text NOT NULL,
  entity_id     bigint NOT NULL,
  action        text NOT NULL,
  changes       jsonb,
  created_at    timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX ON audit_logs (entity, entity_id);
```

### 報價品項為何不外鍵至料件主檔

第二階段會有料件主檔，但 `quotation_items` 仍存名稱與規格的快照，不依賴外鍵。報價一旦送出給客戶，其內容就不應隨料件主檔改名或改價而變動。第二階段會另加一個可為空的 `material_id` 作為關聯參考，但顯示與計算一律以快照為準。

### 狀態轉換

報價單：

```
draft ──► pending ──► approved ──► (轉合約)
  ▲          │
  └──────────┴──► rejected
                     │
draft/pending/approved ──► superseded（產生新版本時，舊版自動轉此狀態）
```

規則以 `domain/quotation.rs` 的純函式表達並測試，不散落於 handler。

## 4. 介面契約

### 認證

Cookie session（`HttpOnly`、`SameSite=Lax`、正式環境 `Secure`），session 存 Postgres。不用 JWT：內部系統無跨網域需求，cookie 可由伺服器即時撤銷，且 token 不進 `localStorage` 即免疫 XSS 竊取。

### API 形狀

REST + JSON，路徑 `/api/v1/...`。以 `utoipa` 從 Rust 型別產生 OpenAPI 文件，前端再由該文件產生 TypeScript 型別——**契約單一來源為 Rust 型別定義**，前後端不各自手寫。

錯誤回應統一格式，前端單一入口處理：

```json
{ "error": { "code": "VALIDATION_FAILED", "message": "…", "fields": { "quantity": "必須大於 0" } } }
```

`code` 為機器可判讀的固定字串，`message` 為給人看的繁中訊息。內部錯誤只回傳追蹤 ID，不外洩 SQL 或路徑。

### 分頁

列表一律分頁，預設每頁 50 筆。第一階段用 `LIMIT/OFFSET`——資料量下限（5 年 7,500 張報價）不足以讓深分頁成為問題。若第三階段的工時紀錄成長到需要，再改 keyset 分頁。

## 5. 擴展點

| 預期變動 | 預留方式 |
| --- | --- |
| 客戶入口（第二 / 三階段） | `users.kind` 與 `customer_id`、三張主表的 `public_id` |
| 料件主檔關聯（第二階段） | `quotation_items` 後續加可為空的 `material_id` |
| 通知管道（LINE / Email） | 尚未引入抽象層。有第二個管道需求時再抽，現在抽是 YAGNI |
| 檔案儲存 | 第一階段無附件需求。第二階段引入時再決定本機 volume 或 S3 相容 |

## 6. 已知取捨

| 決定 | 取捨 |
| --- | --- |
| 單一 crate 不切 workspace | 邊界靠 module 可見性而非編譯器強制。程式規模成長後可能需重新評估 |
| db 層不抽 trait | 換資料庫成本高，但此情境不會發生。換得較少樣板程式碼 |
| 前端同源由後端提供 | 免 CORS 與 token 儲存問題，代價是前端改動需重 build image |
| `LIMIT/OFFSET` 分頁 | 實作簡單，深分頁效能差。第一階段資料量不觸發此問題 |
| 全程 `NUMERIC` / `Decimal` | 比整數分略慢，但此規模無感；換得進位邏輯集中可測 |

## 7. 待審查確認事項

1. `db` 層不抽 trait 是否合理，或應至少對報價相關查詢抽象以利測試
2. 金額進位規則（先加總再進位）是否符合台灣報價實務
3. `audit_logs` 用 `jsonb` 存變更前後值，在 5 年約數萬筆的規模下是否需要額外索引策略
4. 單一 crate 的模組邊界是否足以防止 `domain` 被 `sqlx` 型別污染
