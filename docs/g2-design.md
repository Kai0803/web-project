# G2 設計 — 第一階段：核心（客戶 / 專案 / 報價）

狀態：**第二版，已納入 architecture-reviewer 與 algorithm-advisor 的審查結果**。

對應需求：[G1 需求釐清](./g1-requirements.md)（已確認）。
審查處置對照見第 11 節。

## 1. 部署拓樸

```
┌──────────────────── Pod: acms ─────────────────────┐
│  container: app              container: postgres   │
│  ├ Rust / axum               ├ postgres:17-alpine  │
│  ├ REST API                  └ PVC: acms-pgdata    │
│  ├ 同源提供 Vue 建置產物                            │
│  └ typst 排版產生報價單 PDF                         │
│                                                     │
│  container: backup (sidecar)                       │
│  └ 每日 pg_dump → PVC: acms-backup                 │
└─────────────────────────────────────────────────────┘
                   hostPort 8080
```

**前端由後端同源提供**，不另開 web server：同源即無 CORS，認證可用 `HttpOnly` cookie，token 不進 `localStorage`。代價是前端改動要重 build image，單人開發可接受。

**備份**：G1 要求每日備份、可接受遺失上限 24 小時。單一 PVC 損毀即全損，因此以 sidecar 每日 `pg_dump` 至獨立 PVC。還原程序需實際演練過一次才算數。

**Migration**：以 `sqlx migrate` 在 app 啟動時執行。單一實例下這是最簡單且安全的做法（無並行 migration 競爭）。app 啟動時需等待 postgres ready，以連線重試（指數退避，上限 30 秒）處理，不依賴容器啟動順序。

## 2. Crate 與模組邊界

### 兩個 crate

```
acms/
├── Cargo.toml              workspace
├── crates/
│   ├── domain/             acms-domain
│   │   └── Cargo.toml      依賴僅 rust_decimal、thiserror、time
│   └── app/                acms-app
│       └── Cargo.toml      axum、sqlx、tower-http、utoipa、argon2…
└── web/                    Vue 3 + TypeScript
```

`acms-domain` 的 `Cargo.toml` **不包含 sqlx、axum、serde**，因此 domain 內寫 `use sqlx::...` 會編譯失敗。這是 module 可見性做不到的——`pub(crate)` 管的是「誰能看到我」，不管「我能用什麼」，同一個 crate 內任何檔案都能 `use` 該 crate 的任何依賴。

單一 crate 還會**鼓勵**污染：`db` 層想直接用 `CustomerId` 當 `query_as!` 參數就需要 `impl sqlx::Type for CustomerId`，而孤兒規則規定該 impl 只能寫在定義型別的 crate 裡。在單一 crate 下，最省事的寫法永遠是回頭污染 domain。

附帶好處：domain 的測試不需編譯 sqlx 與 axum，增量編譯明顯變快。

`rust_decimal::Decimal` 可以進 domain：它是值型別而非框架型別，sqlx 與 serde 對它的支援是由**那兩邊的 feature** 提供（`sqlx = { features = ["rust_decimal"] }`），domain 不需要知道任何事。

### acms-app 的模組

```
src/
  main.rs         組裝：讀設定、建連線池、跑 migration、掛路由、啟動
  config.rs       環境變數讀取與啟動時驗證
  error.rs        AppError（僅 api 層使用）與 IntoResponse
  service/        use case：交易邊界、編排、授權決策
    quotation.rs    改版、核准、明細重算
    contract.rs     報價轉合約
  db/             sqlx 查詢，回傳 domain 型別
  api/            axum handler、DTO、輸入驗證
    v1/
      customers.rs / projects.rs / quotations.rs / contracts.rs
  auth/           session、argon2 密碼雜湊（機制，不含政策）
  pdf/            typst 模板與報價單產生
```

### 依賴方向

```
        api (handler / DTO / 驗證)
         │
         ▼
      service (use case：交易邊界、編排)
       ╱      ╲
      ▼        ▼
  domain     db (sqlx，接受 executor)
      ▲        │
      └────────┘   db 回傳 domain 型別

auth (機制) ◄── api, service
main ──► 全部（僅組裝）
```

### 為什麼有 service 層

第一階段有四個「規則判斷 + 多次 I/O + 單一交易」的流程：報價改版（舊版轉 superseded + 插入新版 + 複製明細 + 寫稽核）、報價核准、報價轉合約、明細編輯後重算表頭。`domain` 不能做（不碰 I/O），`db` 不該做（等於把業務規則搬進 SQL 層），沒有 service 就只剩 handler。

**規則**：開啟交易或跨表即進 service；單純的單表 CRUD 允許 `api → db` 直連。

### db 層的簽章契約

所有 `db` 函式接受 `impl sqlx::PgExecutor<'_>` 或 `&mut PgConnection`，**絕不接受 `PgPool`**。交易一律由 service 層開啟與提交。收 pool 的話任兩個 db 呼叫永遠無法在同一交易內，而上述四個流程全都需要。

### 為什麼 db 層不抽 trait

抽 repository trait 的動機從來不是「換資料庫」，而是讓上層能在無 DB 的情況下測試。此處仍不抽，理由是：

1. sqlx 的 `query!` 巨集提供編譯期查詢檢查，這是選 sqlx 的主要價值。包在 trait 後面要多一層轉接型別與 `async_trait` 的 box 配置
2. SQL 密集的程式碼用 mock 測到的是「我有沒有照我以為的方式呼叫自己寫的介面」，不是「這句 SQL 對不對」。以真實 Postgres + `#[sqlx::test]` 的交易回滾測試才測得到後者
3. 純邏輯（金額、狀態轉換、授權）都在 `acms-domain`，那裡本來就沒有 I/O

**代價**：測試需要跑得起來的 Postgres（本機與 CI 皆是），整合測試比純單元測試慢一個數量級。此規模可接受，記錄為有意識的選擇。

### domain 純淨性的具體規則

1. `domain/ids.rs` 的 newtype 不加任何外部 derive
2. `db` 的 `#[derive(sqlx::FromRow)]` row struct 一律 `pub(super)`，只活在 `db` 模組內，對外一律回傳 domain 型別
3. `api` 的 DTO 自帶 serde derive，不重用 domain 型別做序列化

## 3. 金額與進位規則

### 型別

- 資料庫：`NUMERIC(14,4)`
- Rust：`rust_decimal::Decimal`
- 一律不用浮點數

範圍上限約 100 億，報價金額數千萬距上限有近三個數量級餘裕。

### 進位策略

**`rust_decimal` 的預設 `round()` 是銀行家進位（MidpointNearestEven），不是四捨五入。** 實測：

```
500.5  -> round() = 500    MidpointAwayFromZero = 501
-500.5 -> round() = -500   MidpointAwayFromZero = -501
```

掃描 1–2000 元的未稅金額，5.0% 落在 .5 邊界，其中 2.5% 兩策略結果不同。開立發票時稅額差 1 元會被客戶退件。

Postgres 的 `round()` 則是 half-away-from-zero，與 `rust_decimal` 預設不一致。因此：

```rust
// acms-domain/src/money.rs — 全專案唯一的進位入口
use rust_decimal::{Decimal, RoundingStrategy};

/// 台灣會計慣例的四捨五入（half away from zero）。
/// 全專案禁止直接呼叫 Decimal::round() 或 round_dp()。
pub fn round_half_up(v: Decimal, dp: u32) -> Decimal {
    v.round_dp_with_strategy(dp, RoundingStrategy::MidpointAwayFromZero)
}
```

### 計算規則

報價單列印時**每列小計顯示整數元**（使用者確認），因此規則必須讓客戶逐列相加能對上合計：

```
1. 列小計 amount = round_half_up(quantity × unit_price, 0)   -- 進位至整數元，存為權威值
2. 未稅總額 subtotal = Σ amount                               -- 各列已是整數，直接相加不需再進位
3. 稅額 tax_amount = round_half_up(subtotal × tax_rate, 0)    -- 稅基為已進位的 subtotal
4. 含稅總額 total = subtotal + tax_amount
```

三個關鍵點：

- **逐列進位而非先加總再進位**。先加總再進位配上整數元顯示，客戶逐列相加會與印出的合計差到列數的一半元，這是唯一不可行的組合
- **稅基為已進位的 subtotal**，客戶拿報價單上印的未稅金額自行乘 5% 可驗算得到相同稅額
- **`amount` 由 Rust 計算後寫入**，不用 generated column：計算邏輯必須與 `domain/money.rs` 是同一份，不能在 SQL 裡再寫一份

`total = subtotal + tax_amount` 與 `tax_amount = round_half_up(subtotal × tax_rate, 0)` 可寫成 property test。

### 溢位

`Decimal` 的 `*` 溢位時 **panic**（實測 `Decimal::MAX * 2` 確實 panic）。CLAUDE.md 已規定不可信輸入禁用 `.unwrap()`，會 panic 的算術是同一類問題。

- `domain/money.rs` 內部一律用 `checked_mul` / `checked_add`，`None` 轉為具名錯誤
- API 邊界對輸入設業務上限：`quantity` ≤ 1,000,000、`unit_price` ≤ 100,000,000，超出回傳 `VALIDATION_FAILED`

### 跨 JSON 邊界

**`Decimal` 一律序列化為字串**，不是 JSON number。`rust_decimal` 預設序列化為 number，前端 `JSON.parse` 會得到 IEEE754 double，第 3 節建立的精度保證會在系統邊界上蒸發，而報價單正是前端會做即時試算的畫面。

- Rust 側啟用 `rust_decimal` 的 `serde-with-str`
- utoipa schema 標註為 `type: string, format: decimal`，使產生的 TS 型別是 `string` 而非 `number`
- **前端不做金額試算**，數量或單價變動時呼叫後端重算。此規模的延遲完全可接受，且保證前後端數字永遠一致

## 4. 核心資料結構

### 識別字

主鍵為 `bigint generated always as identity`。可能對外曝露的表（`customers`、`projects`、`quotations`、`contracts`）額外有 `public_id uuid unique`。內部 join 用整數鍵最快，對外 URL 用 uuid 避免流水號被列舉。

### Schema

```sql
CREATE TYPE user_kind     AS ENUM ('staff', 'customer_contact');
CREATE TYPE project_status AS ENUM ('lead', 'quoting', 'contracted', 'closed', 'cancelled');
CREATE TYPE quotation_status AS ENUM ('draft', 'pending', 'approved', 'rejected', 'superseded');
CREATE TYPE contract_kind AS ENUM ('original', 'variation');

-- 共用的 updated_at 維護
CREATE FUNCTION touch_updated_at() RETURNS trigger AS $$
BEGIN NEW.updated_at = now(); RETURN NEW; END;
$$ LANGUAGE plpgsql;

CREATE TABLE customers (
  id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  public_id     uuid NOT NULL UNIQUE DEFAULT gen_random_uuid(),
  code          text NOT NULL UNIQUE,
  name          text NOT NULL,
  tax_id        text,
  phone         text,
  address       text,
  note          text,
  is_active     boolean NOT NULL DEFAULT true,
  created_at    timestamptz NOT NULL DEFAULT now(),
  updated_at    timestamptz NOT NULL DEFAULT now()
);
CREATE TRIGGER t_customers_touch BEFORE UPDATE ON customers
  FOR EACH ROW EXECUTE FUNCTION touch_updated_at();

CREATE TABLE users (
  id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  kind          user_kind NOT NULL DEFAULT 'staff',
  customer_id   bigint REFERENCES customers(id),
  username      text NOT NULL UNIQUE,
  password_hash text NOT NULL,
  display_name  text NOT NULL,
  is_active     boolean NOT NULL DEFAULT true,
  created_at    timestamptz NOT NULL DEFAULT now(),
  updated_at    timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT customer_contact_has_customer
    CHECK ((kind = 'customer_contact') = (customer_id IS NOT NULL))
);
CREATE TRIGGER t_users_touch BEFORE UPDATE ON users
  FOR EACH ROW EXECUTE FUNCTION touch_updated_at();

-- 角色為資料（使用者可自行新增），權限為程式碼定義的固定集合。
-- 見第 4.5 節「角色與權限」。
CREATE TABLE roles (
  code          text PRIMARY KEY,
  name          text NOT NULL,
  description   text,
  is_system     boolean NOT NULL DEFAULT false,   -- 系統預設角色不可刪除
  created_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE role_permissions (
  role_code     text NOT NULL REFERENCES roles(code) ON DELETE CASCADE,
  permission    text NOT NULL,
  PRIMARY KEY (role_code, permission)
);

-- 一人可兼多角色（使用者確認）
CREATE TABLE user_roles (
  user_id       bigint NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role_code     text NOT NULL REFERENCES roles(code),
  PRIMARY KEY (user_id, role_code)
);

-- 公司資料。單列表，內容由使用者於「系統設定 → 公司資料」頁面填寫。
CREATE TABLE company_profile (
  id            smallint PRIMARY KEY DEFAULT 1 CHECK (id = 1),
  name          text NOT NULL,                    -- 公司抬頭
  tax_id        text,                             -- 統一編號
  phone         text,
  fax           text,
  email         text,
  address       text,
  logo_svg      text,                             -- SVG 原始碼，供網頁與 typst 共用
  quote_terms   text NOT NULL DEFAULT '',         -- 報價單條款，見 docs/quotation-terms.md
  updated_at    timestamptz NOT NULL DEFAULT now()
);
CREATE TRIGGER t_company_profile_touch BEFORE UPDATE ON company_profile
  FOR EACH ROW EXECUTE FUNCTION touch_updated_at();

CREATE TABLE customer_contacts (
  id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  customer_id   bigint NOT NULL REFERENCES customers(id) ON DELETE CASCADE,
  name          text NOT NULL,
  title         text,
  phone         text,
  email         text,
  is_primary    boolean NOT NULL DEFAULT false
);
CREATE INDEX ON customer_contacts (customer_id);
CREATE UNIQUE INDEX one_primary_contact_per_customer
  ON customer_contacts (customer_id) WHERE is_primary;

CREATE TABLE projects (
  id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  public_id     uuid NOT NULL UNIQUE DEFAULT gen_random_uuid(),
  code          text NOT NULL UNIQUE,
  customer_id   bigint NOT NULL REFERENCES customers(id),
  name          text NOT NULL,
  site_address  text,
  status        project_status NOT NULL DEFAULT 'lead',
  manager_id    bigint REFERENCES users(id),
  expected_start date,
  expected_end   date,
  note          text,
  created_at    timestamptz NOT NULL DEFAULT now(),
  updated_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX ON projects (customer_id);
CREATE TRIGGER t_projects_touch BEFORE UPDATE ON projects
  FOR EACH ROW EXECUTE FUNCTION touch_updated_at();

CREATE TABLE quotations (
  id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  public_id     uuid NOT NULL UNIQUE DEFAULT gen_random_uuid(),
  project_id    bigint NOT NULL REFERENCES projects(id),
  quote_no      text NOT NULL,
  version       integer NOT NULL,
  status        quotation_status NOT NULL DEFAULT 'draft',
  tax_rate      numeric(5,4) NOT NULL DEFAULT 0.0500,  -- 隨報價儲存，稅率調整不影響歷史
  valid_until   date,
  subtotal      numeric(14,4) NOT NULL DEFAULT 0,
  tax_amount    numeric(14,4) NOT NULL DEFAULT 0,
  total         numeric(14,4) NOT NULL DEFAULT 0,
  lock_version  integer NOT NULL DEFAULT 0,            -- 樂觀鎖，與業務 version 區分
  created_by    bigint NOT NULL REFERENCES users(id),
  approved_by   bigint REFERENCES users(id),
  approved_at   timestamptz,
  note          text,
  created_at    timestamptz NOT NULL DEFAULT now(),
  updated_at    timestamptz NOT NULL DEFAULT now(),
  UNIQUE (quote_no, version),
  UNIQUE (project_id, id),                             -- 供 contracts 複合外鍵使用
  -- 進位規則保證三個金額恆為整數元
  CONSTRAINT amounts_are_whole CHECK (
    subtotal = trunc(subtotal) AND tax_amount = trunc(tax_amount) AND total = trunc(total)
  )
);
CREATE INDEX ON quotations (project_id);
CREATE UNIQUE INDEX one_approved_version_per_quote
  ON quotations (quote_no) WHERE status = 'approved';
CREATE TRIGGER t_quotations_touch BEFORE UPDATE ON quotations
  FOR EACH ROW EXECUTE FUNCTION touch_updated_at();

CREATE TABLE quotation_items (
  id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  quotation_id  bigint NOT NULL REFERENCES quotations(id) ON DELETE CASCADE,
  line_no       integer NOT NULL,
  name          text NOT NULL,              -- 快照，不外鍵至第二階段的料件主檔
  spec          text,
  unit          text NOT NULL,
  quantity      numeric(14,4) NOT NULL,
  unit_price    numeric(14,4) NOT NULL,
  unit_cost     numeric(14,4),              -- 成本，可為空；事後無法回填故現在就留
  amount        numeric(14,4) NOT NULL,     -- round_half_up(quantity × unit_price, 0)
  note          text,
  UNIQUE (quotation_id, line_no),
  CONSTRAINT amount_is_whole CHECK (amount = trunc(amount))
);

CREATE TABLE contracts (
  id                bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  public_id         uuid NOT NULL UNIQUE DEFAULT gen_random_uuid(),
  project_id        bigint NOT NULL,
  quotation_id      bigint NOT NULL UNIQUE,
  contract_no       text NOT NULL UNIQUE,
  kind              contract_kind NOT NULL DEFAULT 'original',
  parent_contract_id bigint REFERENCES contracts(id),   -- 追加減帳指向原約
  signed_date       date,
  total             numeric(14,4) NOT NULL,             -- 簽約當下快照，非衍生值
  created_at        timestamptz NOT NULL DEFAULT now(),
  updated_at        timestamptz NOT NULL DEFAULT now(),
  -- 保證 quotation 確實屬於該 project
  FOREIGN KEY (project_id, quotation_id) REFERENCES quotations (project_id, id),
  CONSTRAINT variation_has_parent CHECK ((kind = 'variation') = (parent_contract_id IS NOT NULL))
);
CREATE INDEX ON contracts (project_id);
CREATE TRIGGER t_contracts_touch BEFORE UPDATE ON contracts
  FOR EACH ROW EXECUTE FUNCTION touch_updated_at();

-- 工項：版本無關的明細，於「報價轉合約」時由核准版本實體化。
-- 第二階段 bom_items.work_item_id、第三階段 assignments / labor_hours /
-- progress 的 work_item_id 都掛在這裡，不掛在會被 superseded 的報價明細上。
CREATE TABLE work_items (
  id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  contract_id   bigint NOT NULL REFERENCES contracts(id) ON DELETE CASCADE,
  seq           integer NOT NULL,
  name          text NOT NULL,
  spec          text,
  unit          text NOT NULL,
  quantity      numeric(14,4) NOT NULL,
  unit_price    numeric(14,4) NOT NULL,
  unit_cost     numeric(14,4),
  amount        numeric(14,4) NOT NULL,
  UNIQUE (contract_id, seq)
);

CREATE TABLE audit_logs (
  id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  actor_id      bigint REFERENCES users(id),
  entity        text NOT NULL,
  entity_id     bigint NOT NULL,
  action        text NOT NULL,
  changes       jsonb,              -- 僅記錄有變動欄位的前後值，不存整體快照
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX ON audit_logs (entity, entity_id, created_at DESC);

CREATE TABLE sessions (
  id            text PRIMARY KEY,
  user_id       bigint NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  expires_at    timestamptz NOT NULL,
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX ON sessions (expires_at);
```

### 稽核紀錄的形狀

`changes` 固定為「僅有變動欄位的前後值」，不存整體快照：

```json
{ "status": {"from": "pending", "to": "approved"},
  "total":  {"from": "1250000", "to": "1280000"} }
```

明細變更以「新增／刪除／修改的 `line_no` 清單加該列欄位 diff」表達，不複製未變動的列。存整體快照會讓單列 jsonb 達 50–200 KB、數萬列即數 GB 並觸發 TOAST 外存，備份體積會被稽核表主導。

稽核寫入必須與被稽核的變更在**同一交易**內。

**不需要 GIN 索引、不需要分區。** 數萬列規模下 `(entity, entity_id, created_at DESC)` 已完整覆蓋唯一的查詢型態並直接供應排序。

### 狀態轉換

```
draft ──► pending ──► approved ──► (轉合約)
  ▲          │
  │          └──► rejected ──► draft      （退回修改再送審）
  │
draft/pending/approved ──► superseded     （產生新版本時舊版轉此）
```

規則以 `acms-domain` 的純函式表達並測試。

**核准後凍結**：`domain` 定義 `is_mutable(status) -> bool`，僅 `draft` 為 true。所有明細寫入與金額重算的 use case 一律先過此判斷。另加 DB trigger 阻擋對非 `draft` 報價的 `quotation_items` 寫入——此規模 trigger 成本可忽略，且能擋掉手動 SQL 誤操作。G1 成功條件「修改報價產生新版本，保留舊版可查」的前提就是舊版不可變。

### 4.5 角色與權限

**權限（permission）是程式碼定義的固定集合，角色（role）是資料。** 使用者可在「系統設定 → 角色管理」自由新增角色並勾選權限組合，但不能發明新權限——權限必須有對應的程式碼檢查點，否則新增的權限不會有任何效果。

#### 權限清單（程式碼定義）

| 權限 | 說明 |
| --- | --- |
| `customer.read` / `customer.write` | 客戶資料檢視 / 編輯 |
| `project.read` / `project.write` | 專案檢視 / 編輯 |
| `quotation.read` / `quotation.write` | 報價單檢視 / 編輯 |
| `quotation.approve` | 核准報價單 |
| `quotation.cost.read` | **檢視成本與毛利**（G1 要求的敏感資料控管） |
| `contract.read` / `contract.write` | 合約檢視 / 編輯 |
| `report.read` | 報表 |
| `user.manage` | 帳號管理 |
| `settings.manage` | 公司資料、角色、單號規則等系統設定 |

第二階段新增 `material.*`、`purchase_order.*`、`inventory.*`；第三階段新增 `assignment.*`、`labor.*`。新增權限需同時新增程式碼檢查點，屬於開發工作而非設定。

#### 預設角色（`is_system = true`，不可刪除，但權限可調整）

| 角色 | code | 權限 |
| --- | --- | --- |
| 系統管理員 | `admin` | 全部 |
| 經營主管 | `manager` | 全部（含 `quotation.approve` 與 `quotation.cost.read`），不含 `user.manage` |
| 業務 | `sales` | `customer.*`、`project.*`、`quotation.read/write`、`contract.read`。**不含 `quotation.cost.read`** |
| 專案經理 | `pm` | `project.*`、`quotation.read`、`quotation.cost.read`、`contract.*` |
| 會計 | `accounting` | 各項 `read` + `quotation.cost.read` + `report.read` |
| 唯讀 | `viewer` | 各項 `read`，不含成本 |

業務預設看不到成本是刻意的：報價由業務開立，成本由主管與會計掌握，這是多數工程公司的實務分工。若貴公司業務需要看成本，在角色管理勾選即可，不需改程式碼。

#### 成本欄位的可見性

`quotation.cost.read` 的檢查發生在 **API 的 DTO 序列化階段**，無此權限者回應中根本不含 `unit_cost` 欄位，不是前端隱藏。前端隱藏等於把資料送到瀏覽器再假裝看不到，開發者工具一開就破功。

#### 授權判斷的位置

授權為業務規則，放在 `acms-domain/src/authz.rs`（純函式：輸入角色權限集合與資源屬性，輸出可否）。`auth/` 模組只保留 session 與密碼雜湊等機制。

### 單號產生

`customers.code`、`projects.code`、`quotations.quote_no`、`contracts.contract_no` 皆為人類可讀單號，格式含年度（如 `P2026-001`）。

以年度序號表加 `SELECT ... FOR UPDATE` 產生，於交易內完成。**不用 `SELECT max(...) + 1`**：10–15 人並行下在 read committed 隔離級別會產生重複，UNIQUE 會擋下但使用者看到的是 500。跨年重置由序號表的 `(kind, year)` 主鍵表達。

## 5. 介面契約

### 認證與 CSRF

Cookie session（`HttpOnly`、`SameSite=Strict`、正式環境 `Secure`），session 存 Postgres。不用 JWT：內部系統無跨網域需求，cookie 可即時撤銷，且 token 不進 `localStorage` 即免疫 XSS 竊取。

Cookie 認證的代價是 CSRF。防護組合：

1. `SameSite=Strict`（同源 SPA 無跨站導向需求，可用最嚴格值）
2. 所有非安全方法檢查 `Origin` header 與預期來源相符
3. 自訂 header 檢查（`X-Requested-With`），跨站表單無法設定自訂 header

前一版 Node 系統有 CSRF token，這次以上述組合取代。細節交由 security-auditor 於 G4 驗證。

### API 形狀

REST + JSON，路徑 `/api/v1/...`。以 `utoipa` 從 Rust 型別產生 OpenAPI，前端據此產生 TypeScript 型別，**契約單一來源為 Rust 型別定義**。

**漂移檢查**：utoipa 的標註由人工維護，可能與 handler 實際行為不同步。CI 檢查「重新產生的 `openapi.json` 與 committed 版本無 diff」且「重新產生的 TS 型別無 diff」，否則單一來源會在三個月內退化成兩份手寫。

### 錯誤格式

```json
{ "error": { "code": "VALIDATION_FAILED", "message": "…", "fields": { "quantity": "必須大於 0" } } }
```

`code` 為列舉型別（非自由字串），一併進 OpenAPI schema 成為 TS union type，前端才知道全集。內部錯誤只回傳追蹤 ID，不外洩 SQL 或路徑。

**錯誤型別分層**（CLAUDE.md 慣例）：`acms-domain` 各模組定義自己的 `thiserror` 型別（如 `InvalidTransition { from, to }`），`db` 定義自己的，`AppError` 只存在於 `api` 層，以 `From` 收斂並決定 HTTP 狀態碼與 `code`。單一扁平的 `AppError` 會讓 domain 反過來認識 axum。

### 列表查詢契約

**列表端點一律以單一 JOIN 查詢完成，不在 handler 迴圈內查詢資料庫。**

| 端點 | 回傳關聯欄位 | 查詢方式 |
| --- | --- | --- |
| 專案列表 | 客戶名稱 | `JOIN customers` |
| 報價列表 | 專案名、客戶名、建立者、明細列數 | `JOIN` + `COUNT(*) OVER` 或 lateral |
| 專案列表（含最新報價金額） | 最新版報價 total | `DISTINCT ON (quote_no) ... ORDER BY quote_no, version DESC`，吃 `UNIQUE (quote_no, version)` |

未定義查詢形狀的話，報價列表逐列查四次關聯資料會變成 201 次 round-trip（約 200 ms），雖仍在預算內但吃掉大半且完全可避免。

### 分頁

`LIMIT/OFFSET`，預設每頁 50 筆。5 年 7,500 張報價的最深一頁約 5–15 ms，佔 500 ms 預算的 3%，keyset 分頁在此規模只增加複雜度而換不到可感知差異。第三階段工時紀錄成長時再評估。

**`ORDER BY` 必須全序**：一律附加 `id` 作為決勝鍵（如 `ORDER BY created_at DESC, id DESC`）。否則 `created_at` 相同的兩列在跨頁時順序不保證一致，使用者翻頁會看到某筆重複、另一筆消失，而這個 bug 極難從回報中診斷。零成本，且日後改 keyset 分頁可沿用同一排序鍵。

### 批次寫入

報價明細以**單次多列 INSERT** 完成（`QueryBuilder::push_values` 或 `UNNEST` 陣列參數），不逐列 execute。100 列逐列寫入約 50–100 ms，是第一階段最貴的單一使用者操作，且成因純粹是 round-trip 次數。改批次後降至 5 ms 以下。

表頭金額重算與明細寫入必須在**同一交易**內，且一律由 `Σ amount` 重算而非增量更新。重算只呼叫 `domain/money.rs` 的同一個函式，SQL 裡不得有第二份 `SUM` 實作。

## 6. 報價單輸出

**typst** 伺服器端排版產生 PDF（使用者確認）。

- 純 Rust 排版引擎，映像只增加數 MB，不需 headless 瀏覽器的 process 管理
- 中文字型（思源黑體或類似開源字型）需打包進映像，字型授權需確認可散布
- 排版為 CPU 密集工作，於 `spawn_blocking` 執行，不阻塞 async runtime
- 第一階段 PDF 即時產生並回傳，不落地。第二階段若要 Email 寄送或存檔，再決定儲存位置（屆時一併處理施工照片的儲存，見第 9 節）

版面需求：每列小計顯示整數元（與第 3 節進位規則一致）、顯示未稅／稅額／含稅三個數字、含公司抬頭與報價有效期限。

### 公司抬頭、Logo 與條款的填寫位置

三者皆存於 `company_profile` 單列表，由**「系統設定 → 公司資料」**頁面填寫，改動後下一次產生的 PDF 立即生效，不需重新建置映像。

| 欄位 | 內容 | 初始值 |
| --- | --- | --- |
| `name` | 公司抬頭 | 空白，待使用者填寫 |
| `tax_id` / `phone` / `fax` / `email` / `address` | 表頭聯絡資訊 | 空白 |
| `logo_svg` | Logo 的 SVG 原始碼 | migration 寫入 `assets/brand/` 選定的方案 |
| `quote_terms` | 報價單條款 | migration 寫入 `docs/quotation-terms.md` 的範本 |

Logo 存 SVG 原始碼而非檔案路徑：網頁與 typst 可共用同一份，且使用者日後換 logo 只需貼上新的 SVG，不必動容器內的檔案。

## 7. 維運與可觀測性

| 項目 | 做法 |
| --- | --- |
| Migration | `sqlx migrate` 於 app 啟動時執行，連線重試上限 30 秒 |
| 結構化日誌 | `tracing` + `tower-http` 的 `TraceLayer`，每個請求產生 request id 並貫穿日誌 |
| 追蹤 ID | 錯誤回應中的追蹤 ID 即 request id，可直接於日誌檢索 |
| 健康檢查 | `/healthz` 執行 `SELECT 1` 實際驗證資料庫連線，供 kube probe 使用 |
| 備份 | sidecar 每日 `pg_dump` 至獨立 PVC，還原程序需演練 |
| 失敗模式 | Postgres 不可用時連線池取得逾時，回 503 而非掛住；不在啟動時 panic |
| Graceful shutdown | 收到 SIGTERM 後停止接受新請求，等待進行中請求完成（上限 30 秒）再關閉 |

## 8. 前端架構

```
web/src/
  api/          由 OpenAPI 產生的 client 與型別（產物，不手改）
  features/
    customers/  元件 + composable + 路由
    projects/
    quotations/ 最複雜：動態明細列、與後端往返試算
  stores/       Pinia，僅放跨頁共用狀態（目前只有目前登入者與權限）
  components/   跨 feature 共用的表現層元件
```

- 元件狀態優先留在元件內，不上提至全域 store
- 金額一律以字串接收與顯示，**前端不做試算**（見第 3 節）
- 報價明細編輯畫面切成「明細列表」「單列編輯」「合計摘要」三個元件，避免長成單一大元件

## 9. 擴展點

| 預期變動 | 預留方式 |
| --- | --- |
| 客戶入口（第二 / 三階段） | `users.kind` 與 `customer_id`、四張主表的 `public_id` |
| BOM（第二階段） | `bom_items.work_item_id` 掛在 `work_items` |
| 派工 / 工時 / 進度（第三階段） | 同上，皆掛 `work_items` |
| 料件主檔關聯（第二階段） | `quotation_items` 後續加可為空的 `material_id`，顯示與計算仍以快照為準 |
| 合約追加減帳 | `contracts.kind` 與 `parent_contract_id` |
| 通知管道（LINE / Email） | 尚未抽象。引入第二個管道時才定義 trait，不在呼叫點加 if |
| 施工照片（第二階段） | **不進資料庫、不與 pgdata 共用 PVC**。5 年可能達數百 GB，備份策略與資料庫完全不同 |

**明確不做**：`suppliers` 與 `customers` 統一為通用 `parties` 表（兩者欄位與生命週期會分歧，統一換來每次查詢多一個 kind 條件）；`projects` 的階層 / 分區（對此規模是憑空複雜度）。

## 10. 已知取捨

| 決定 | 取捨 |
| --- | --- |
| 兩個 crate 的 workspace | 每個查詢多幾行 domain ↔ db 型別映射，換得編譯器強制的 domain 純淨性與更快的增量編譯 |
| db 層不抽 trait | 測試需要真實 Postgres，比純單元測試慢一個數量級 |
| 前端同源由後端提供 | 免 CORS 與 token 儲存問題，代價是前端改動需重 build image |
| 前端不做金額試算 | 每次改數量要往返後端，換得前後端數字保證一致、且計算邏輯只有一份 |
| `LIMIT/OFFSET` 分頁 | 深分頁效能差，但此規模不觸發 |
| 全程 `NUMERIC` / `Decimal` | 比整數分略慢（實測每次報價彙總約 10 µs，佔預算 0.002%），換得進位邏輯集中可測 |
| 逐列進位至整數元 | 有列數一半元的理論誤差，但誤差由文件自身定義且客戶可驗算；先加總再進位配整數元顯示則無法自洽 |
| 工項於轉合約時才實體化 | 未簽約的專案無法排程。第一階段無此需求，第三階段若需要再放寬 |

## 11. 審查 findings 處置

### architecture-reviewer

| ID | 內容 | 處置 |
| --- | --- | --- |
| B1 | 缺 service 層承接 use case | 已加，見第 2 節 |
| B2 | 報價單輸出無設計 | 已加，typst，見第 6 節 |
| H1 | db 函式簽章需收 executor | 已明訂，見第 2 節 |
| H2 | 單一 crate 對 domain 純淨性零保證 | 改為兩個 crate，見第 2 節 |
| H3 | 缺版本無關工項 | 已加 `work_items`，見第 4 節 |
| H4 | 缺成本欄位 | 已加 `unit_cost` |
| H5 | Decimal 跨 JSON 變 float | 序列化為字串，見第 3 節 |
| H6 | 核准後報價未凍結 | `is_mutable` + DB trigger，見第 4 節 |
| H7 | 可觀測性與維運缺席 | 已加第 7 節 |
| H8 | 合約無追加減帳 | 已加 `kind` 與 `parent_contract_id` |
| H9 | 顯示精度未定義 | 使用者確認整數元，進位規則已重寫 |
| M1 | status 用 text 可繞過 partial index | 全改 ENUM |
| M2 | 單一 AppError 與慣例衝突 | 錯誤分層，見第 5 節 |
| M3 | auth 混合機制與政策 | 政策移至 `acms-domain`，auth 只留 session 與雜湊 |
| M4 | 單號產生策略未定義 | 已加，見第 4 節 |
| M5 | `updated_at` 無更新機制 | 共用 trigger |
| M6 | 前端架構空白 | 已加第 8 節 |
| M7 | OpenAPI → TS 缺漂移檢查 | CI 檢查，見第 5 節 |
| M8 | CSRF 責任未指派 | 已加防護組合，見第 5 節 |
| M9 | `amount` 誰計算未定義 | Rust 計算後寫入，不用 generated column |
| M10 | 表頭彙總重算時機未定義 | 同交易內重算，見第 5 節 |
| M11 | 無樂觀鎖 | 已加 `lock_version` |
| L1 | 冗餘索引 | 已移除 |
| L2 | contracts 未保證 quotation 屬於該 project | 複合外鍵 |
| L3 | 缺 `rejected → draft` | 已加入狀態圖 |
| L4 | audit_logs 索引 | 改為三欄含 `created_at DESC` |
| L5 | contracts 缺 `public_id` 與 `updated_at` | 已補 |
| N1 | uuid v4 vs v7 | 此規模無影響，維持 v4 |

### algorithm-advisor

| ID | 內容 | 處置 |
| --- | --- | --- |
| CALC-1 | 「不進位」與 `NUMERIC(14,4)` 矛盾 | 進位規則重寫，改逐列進位至元 |
| CALC-2 | `rust_decimal` 預設是銀行家進位 | 已實測確認。明訂 `MidpointAwayFromZero`，全專案唯一進位入口。**實測分歧率為 2.5% 而非報告所稱 5%**（整數部分已為奇數時兩策略結果相同） |
| CALC-3 | 稅基未指明 | 明訂為已進位的 subtotal |
| CALC-4 | 表頭與明細無一致性機制 | 同交易重算 + 核准後凍結 + 整合測試斷言 |
| CALC-5 | 稅率未隨報價儲存 | 已加 `tax_rate` 欄位 |
| CALC-6 | `Decimal` 乘法溢位 panic | 已實測確認。內部用 `checked_*`，邊界設業務上限 |
| DB-1 | 列表查詢形狀未定義 | 已加列表查詢契約 |
| DB-2 | status 裸 text | 全改 ENUM |
| DB-3 | 逐列 INSERT | 改單次批次 INSERT |
| DB-4 | 冗餘索引 | 已移除 |
| DB-5 | audit_logs 索引可免費消掉排序 | 已改三欄 |
| DB-6 | `customer_contacts.customer_id` 缺索引 | 已補 |
| DB-7 | `projects (status)` 選擇性低 | **已移除**。列表查詢形狀定義後，若需要再建 `(customer_id, status)` 複合索引 |
| DB-8 | 金額欄位 scale 可轉為不變量 | 已加 CHECK 約束 |
| AUDIT-1 | `changes` 形狀未定義 | 已明訂為欄位 diff |
| PAGE-1 | `ORDER BY` 需全序 | 已明訂加 `id` 決勝鍵 |
| Nit | `updated_at`、單號前綴搜尋、併發建版、`contracts.total` | trigger 已加；單號搜尋不建索引（seq scan 足夠）；併發建版由 `UNIQUE` 擋下，handler 將 `23505` 轉為可讀訊息；`contracts.total` 已註明為快照 |

### 兩位審查者一致確認不需優化

`rust_decimal` 選型、`LIMIT/OFFSET` 分頁、`NUMERIC(14,4)` 範圍、`audit_logs` 不需 GIN 或分區、水平擴展不需要、記憶體內容器選型。此規模對 Postgres 而言極小，所有查詢即使全表掃描都在預算內。

## 12. 待確認

1. typst 中文字型的授權是否允許散布於容器映像（思源黑體為 SIL OFL，可）
2. 公司抬頭與聯絡資訊由使用者自行於系統設定填寫（設計已完成，見第 6 節）
3. Logo 採用方案 A（彎頭）或方案 C（管網），見 `assets/brand/`
4. 報價單條款範本是否需請會計師或律師檢視（見 `docs/quotation-terms.md`）
