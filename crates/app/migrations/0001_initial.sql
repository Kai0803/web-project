-- 第一階段核心 schema：客戶 / 專案 / 報價 / 合約
-- 設計見 docs/g2-design.md 第 4 節。

CREATE TYPE user_kind        AS ENUM ('staff', 'customer_contact');
CREATE TYPE project_status   AS ENUM ('lead', 'quoting', 'contracted', 'closed', 'cancelled');
CREATE TYPE quotation_status AS ENUM ('draft', 'pending', 'approved', 'rejected', 'superseded');
CREATE TYPE contract_kind    AS ENUM ('original', 'variation');

-- 共用的 updated_at 維護。欄位預設值只在 INSERT 生效，UPDATE 需靠 trigger。
CREATE FUNCTION touch_updated_at() RETURNS trigger AS $$
BEGIN
  NEW.updated_at = now();
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;


-- ============================ 客戶 ============================

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


-- ============================ 使用者與權限 ============================

CREATE TABLE users (
  id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  kind          user_kind NOT NULL DEFAULT 'staff',
  -- 僅 customer_contact 有值，為第二 / 三階段的客戶入口預留
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
CREATE INDEX ON users (customer_id);
CREATE TRIGGER t_users_touch BEFORE UPDATE ON users
  FOR EACH ROW EXECUTE FUNCTION touch_updated_at();

-- 角色為資料（使用者可自行新增），權限為程式碼定義的固定集合。
-- 權限字串對應 acms_domain::authz::Permission。
CREATE TABLE roles (
  code          text PRIMARY KEY,
  name          text NOT NULL,
  description   text,
  is_system     boolean NOT NULL DEFAULT false,
  created_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE role_permissions (
  role_code     text NOT NULL REFERENCES roles(code) ON DELETE CASCADE,
  permission    text NOT NULL,
  PRIMARY KEY (role_code, permission)
);

-- 一人可兼多角色，權限取聯集
CREATE TABLE user_roles (
  user_id       bigint NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role_code     text NOT NULL REFERENCES roles(code),
  PRIMARY KEY (user_id, role_code)
);
CREATE INDEX ON user_roles (role_code);

CREATE TABLE sessions (
  id            text PRIMARY KEY,
  user_id       bigint NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  expires_at    timestamptz NOT NULL,
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX ON sessions (expires_at);
CREATE INDEX ON sessions (user_id);


-- ============================ 客戶聯絡人 ============================

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

-- 每個客戶至多一位主要聯絡人，由資料庫保證而非應用層
CREATE UNIQUE INDEX one_primary_contact_per_customer
  ON customer_contacts (customer_id) WHERE is_primary;


-- ============================ 專案 ============================

CREATE TABLE projects (
  id             bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  public_id      uuid NOT NULL UNIQUE DEFAULT gen_random_uuid(),
  code           text NOT NULL UNIQUE,
  customer_id    bigint NOT NULL REFERENCES customers(id),
  name           text NOT NULL,
  site_address   text,
  status         project_status NOT NULL DEFAULT 'lead',
  manager_id     bigint REFERENCES users(id),
  expected_start date,
  expected_end   date,
  note           text,
  created_at     timestamptz NOT NULL DEFAULT now(),
  updated_at     timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX ON projects (customer_id);
CREATE TRIGGER t_projects_touch BEFORE UPDATE ON projects
  FOR EACH ROW EXECUTE FUNCTION touch_updated_at();


-- ============================ 報價單 ============================

CREATE TABLE quotations (
  id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  public_id     uuid NOT NULL UNIQUE DEFAULT gen_random_uuid(),
  project_id    bigint NOT NULL REFERENCES projects(id),
  quote_no      text NOT NULL,
  version       integer NOT NULL CHECK (version > 0),
  status        quotation_status NOT NULL DEFAULT 'draft',
  -- 稅率隨報價儲存。稅率調整後，歷史報價重印須沿用當初的稅率。
  tax_rate      numeric(5,4) NOT NULL DEFAULT 0.0500 CHECK (tax_rate >= 0 AND tax_rate <= 1),
  valid_until   date,
  subtotal      numeric(14,4) NOT NULL DEFAULT 0,
  tax_amount    numeric(14,4) NOT NULL DEFAULT 0,
  total         numeric(14,4) NOT NULL DEFAULT 0,
  -- 樂觀鎖。與業務語意的 version 不同，故命名為 lock_version。
  lock_version  integer NOT NULL DEFAULT 0,
  created_by    bigint NOT NULL REFERENCES users(id),
  approved_by   bigint REFERENCES users(id),
  approved_at   timestamptz,
  note          text,
  created_at    timestamptz NOT NULL DEFAULT now(),
  updated_at    timestamptz NOT NULL DEFAULT now(),
  UNIQUE (quote_no, version),
  -- 供 contracts 的複合外鍵使用，保證合約的報價確實屬於該專案
  UNIQUE (project_id, id),
  -- 進位規則保證三個金額恆為整數元
  CONSTRAINT amounts_are_whole CHECK (
    subtotal = trunc(subtotal) AND tax_amount = trunc(tax_amount) AND total = trunc(total)
  ),
  CONSTRAINT approval_fields_consistent CHECK (
    (approved_by IS NULL) = (approved_at IS NULL)
  )
);
CREATE INDEX ON quotations (project_id);

-- 同一報價單至多一個核准版本。以 partial unique index 表達，
-- 應用層的檢查在併發下必然有漏洞，這裡是寫不進去。
CREATE UNIQUE INDEX one_approved_version_per_quote
  ON quotations (quote_no) WHERE status = 'approved';

CREATE TRIGGER t_quotations_touch BEFORE UPDATE ON quotations
  FOR EACH ROW EXECUTE FUNCTION touch_updated_at();

CREATE TABLE quotation_items (
  id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  quotation_id  bigint NOT NULL REFERENCES quotations(id) ON DELETE CASCADE,
  line_no       integer NOT NULL CHECK (line_no > 0),
  -- 名稱與規格為快照，不外鍵至第二階段的料件主檔：
  -- 報價送出後其內容不應隨主檔改名或改價而變動
  name          text NOT NULL,
  spec          text,
  unit          text NOT NULL,
  quantity      numeric(14,4) NOT NULL CHECK (quantity > 0),
  unit_price    numeric(14,4) NOT NULL,
  -- 成本。可為空，第二階段有料件主檔後可自動帶入。
  -- 事後無法回填歷史，故第一階段就留欄位。
  unit_cost     numeric(14,4),
  -- round_half_up(quantity * unit_price, 0)，由 acms_domain::money 計算後寫入
  amount        numeric(14,4) NOT NULL,
  note          text,
  UNIQUE (quotation_id, line_no),
  CONSTRAINT amount_is_whole CHECK (amount = trunc(amount))
);

-- 非草稿的報價單，明細不可異動。
-- 應用層已有 acms_domain::quotation::ensure_mutable，這裡再擋一層，
-- 以防手動 SQL 或未來新增的第二條寫入路徑繞過。
CREATE FUNCTION forbid_item_change_on_locked_quotation() RETURNS trigger AS $$
DECLARE
  target_id bigint := COALESCE(NEW.quotation_id, OLD.quotation_id);
  current_status quotation_status;
BEGIN
  SELECT status INTO current_status FROM quotations WHERE id = target_id;

  -- 報價單本身正在被刪除（CASCADE），放行
  IF NOT FOUND THEN
    RETURN COALESCE(NEW, OLD);
  END IF;

  IF current_status <> 'draft' THEN
    RAISE EXCEPTION '報價單狀態為 %，明細不可異動。請建立新版本。', current_status
      USING ERRCODE = 'restrict_violation';
  END IF;

  RETURN COALESCE(NEW, OLD);
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER t_quotation_items_locked
  BEFORE INSERT OR UPDATE OR DELETE ON quotation_items
  FOR EACH ROW EXECUTE FUNCTION forbid_item_change_on_locked_quotation();


-- ============================ 合約與工項 ============================

CREATE TABLE contracts (
  id                 bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  public_id          uuid NOT NULL UNIQUE DEFAULT gen_random_uuid(),
  project_id         bigint NOT NULL,
  quotation_id       bigint NOT NULL UNIQUE,
  contract_no        text NOT NULL UNIQUE,
  kind               contract_kind NOT NULL DEFAULT 'original',
  -- 追加減帳指向原約。工程業追加是常態，事後補此欄位歷史合約無法判定歸屬。
  parent_contract_id bigint REFERENCES contracts(id),
  signed_date        date,
  -- 簽約當下的快照，非衍生值。來源報價凍結後兩者恆等。
  total              numeric(14,4) NOT NULL,
  created_at         timestamptz NOT NULL DEFAULT now(),
  updated_at         timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (project_id, quotation_id) REFERENCES quotations (project_id, id),
  CONSTRAINT variation_has_parent CHECK ((kind = 'variation') = (parent_contract_id IS NOT NULL)),
  CONSTRAINT contract_total_is_whole CHECK (total = trunc(total))
);
CREATE INDEX ON contracts (project_id);
CREATE INDEX ON contracts (parent_contract_id);
CREATE TRIGGER t_contracts_touch BEFORE UPDATE ON contracts
  FOR EACH ROW EXECUTE FUNCTION touch_updated_at();

-- 工項：版本無關的明細，於「報價轉合約」時由核准版本實體化。
-- 第二階段 bom_items.work_item_id、第三階段 assignments / labor_hours /
-- progress 的 work_item_id 都掛在這裡，不掛在會被 superseded 的報價明細上。
CREATE TABLE work_items (
  id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  contract_id   bigint NOT NULL REFERENCES contracts(id) ON DELETE CASCADE,
  seq           integer NOT NULL CHECK (seq > 0),
  name          text NOT NULL,
  spec          text,
  unit          text NOT NULL,
  quantity      numeric(14,4) NOT NULL CHECK (quantity > 0),
  unit_price    numeric(14,4) NOT NULL,
  unit_cost     numeric(14,4),
  amount        numeric(14,4) NOT NULL,
  UNIQUE (contract_id, seq)
);


-- ============================ 稽核 ============================

CREATE TABLE audit_logs (
  id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  actor_id      bigint REFERENCES users(id),
  entity        text NOT NULL,
  entity_id     bigint NOT NULL,
  action        text NOT NULL,
  -- 僅記錄有變動欄位的前後值，不存整體快照：
  -- {"status": {"from": "pending", "to": "approved"}}
  -- 存整體快照會讓單列達數百 KB 並觸發 TOAST，備份體積將被此表主導。
  changes       jsonb,
  created_at    timestamptz NOT NULL DEFAULT now()
);
-- 唯一的查詢型態是「某筆資料的變更歷程，時間新到舊」，
-- 三欄複合索引可直接供應排序，不需再 sort。
CREATE INDEX ON audit_logs (entity, entity_id, created_at DESC);


-- ============================ 系統設定 ============================

-- 公司資料。單列表，由「系統設定 → 公司資料」頁面填寫。
CREATE TABLE company_profile (
  id            smallint PRIMARY KEY DEFAULT 1 CHECK (id = 1),
  name          text NOT NULL DEFAULT '',
  tax_id        text,
  phone         text,
  fax           text,
  email         text,
  address       text,
  -- SVG 原始碼而非檔案路徑：網頁與 typst 共用同一份，
  -- 換 logo 只需貼上新 SVG，不必動容器內的檔案。
  logo_svg      text,
  quote_terms   text NOT NULL DEFAULT '',
  updated_at    timestamptz NOT NULL DEFAULT now()
);
CREATE TRIGGER t_company_profile_touch BEFORE UPDATE ON company_profile
  FOR EACH ROW EXECUTE FUNCTION touch_updated_at();

-- 人類可讀單號的年度序號。
-- 不用 SELECT max(...) + 1：在 read committed 下併發會產生重複，
-- UNIQUE 雖擋得下但使用者看到的是 500。取號一律 SELECT ... FOR UPDATE。
CREATE TABLE number_sequences (
  kind          text NOT NULL,
  year          smallint NOT NULL,
  next_value    integer NOT NULL DEFAULT 1 CHECK (next_value > 0),
  PRIMARY KEY (kind, year)
);
