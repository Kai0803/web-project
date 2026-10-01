-- work_items 補上 project_id、public_id 與時間戳。
--
-- 原本 work_items 只透過 contract_id 掛在合約上，所以「某專案的工項」
-- 必須繞 contracts 走，而且工項在簽約前不可能存在。那會卡住兩件事：
--
--   1. 第二階段的 BOM。配管報價時要估料（多少米管、多少彎頭、多少公斤銲材），
--      而估料發生在**報價階段**。若 BOM 只能掛在工項上、工項只能在簽約後存在，
--      第二階段的料件主檔就只能用於簽約後的採購，不能用於每天都在做的估價。
--   2. 第三階段的簽約前排程。工程業常在口頭確認後就先排工班。
--
-- 這個 migration 不放寬 contract_id（仍為 NOT NULL），只把 project_id 加上去。
-- 理由是成本不對稱：現在加一個欄位是一行 DDL；等有資料之後再放寬，要同時
-- 改 nullable、把 UNIQUE (contract_id, seq) 換成兩個 partial index（nullable
-- 欄位在 unique index 裡兩個 NULL 不衝突，未簽約工項的 seq 會失去保護）、
-- 回填 project_id、並檢查所有既有查詢。先把欄位放好，屆時只剩前兩件事。
--
-- 觸發放寬的條件：第二階段確認 BOM 要用於報價估料，或第三階段確認
-- 要在簽約前排工班。

ALTER TABLE work_items ADD COLUMN project_id bigint;

UPDATE work_items w
   SET project_id = c.project_id
  FROM contracts c
 WHERE c.id = w.contract_id;

ALTER TABLE work_items ALTER COLUMN project_id SET NOT NULL;

-- 供 work_items 的複合外鍵使用，保證工項的專案與其合約的專案一致
ALTER TABLE contracts ADD CONSTRAINT contracts_project_id_id_key UNIQUE (project_id, id);

-- 與 contracts → quotations 同一手法：複合外鍵讓「工項掛到別的專案的合約上」
-- 寫不進去，而不是靠應用層記得檢查
ALTER TABLE work_items
  ADD CONSTRAINT work_item_contract_same_project
  FOREIGN KEY (project_id, contract_id) REFERENCES contracts (project_id, id);

CREATE INDEX ON work_items (project_id);

-- 與其他四張主表一致：對外穩定識別字（第三階段客戶查進度時，
-- 對外 URL 需要工項的識別字而非序號 id）與時間戳（工項的 quantity
-- 會因追加減帳而變動）
ALTER TABLE work_items
  ADD COLUMN public_id  uuid NOT NULL DEFAULT gen_random_uuid(),
  ADD COLUMN created_at timestamptz NOT NULL DEFAULT now(),
  ADD COLUMN updated_at timestamptz NOT NULL DEFAULT now();

ALTER TABLE work_items ADD CONSTRAINT work_items_public_id_key UNIQUE (public_id);

CREATE TRIGGER t_work_items_touch BEFORE UPDATE ON work_items
  FOR EACH ROW EXECUTE FUNCTION touch_updated_at();
