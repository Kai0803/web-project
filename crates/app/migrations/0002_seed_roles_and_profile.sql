-- 預設角色、權限組合，以及公司資料的初始列。
--
-- 角色為資料，管理員可在「系統設定 → 角色管理」新增或調整權限組合。
-- is_system = true 的角色不可刪除，但權限可調整。
-- 權限字串必須對應 acms_domain::authz::Permission，否則會被靜默忽略。

INSERT INTO roles (code, name, description, is_system) VALUES
  ('admin',      '系統管理員', '帳號、系統設定與全部資料',             true),
  ('manager',    '經營主管',   '核准報價、檢視成本與報表',             true),
  ('sales',      '業務',       '建立客戶與報價，預設不可檢視成本',     true),
  ('pm',         '專案經理',   '管理專案與合約，可檢視成本',           true),
  ('accounting', '會計',       '檢視成本、報表與請款資料',             true),
  ('viewer',     '唯讀',       '僅檢視，不含成本',                     true);

-- admin：全部權限
INSERT INTO role_permissions (role_code, permission)
SELECT 'admin', p FROM unnest(ARRAY[
  'customer.read', 'customer.write',
  'project.read', 'project.write',
  'quotation.read', 'quotation.write', 'quotation.approve', 'quotation.cost.read',
  'contract.read', 'contract.write',
  'report.read', 'user.manage', 'settings.manage'
]) AS p;

-- manager：除帳號管理外全部
INSERT INTO role_permissions (role_code, permission)
SELECT 'manager', p FROM unnest(ARRAY[
  'customer.read', 'customer.write',
  'project.read', 'project.write',
  'quotation.read', 'quotation.write', 'quotation.approve', 'quotation.cost.read',
  'contract.read', 'contract.write',
  'report.read', 'settings.manage'
]) AS p;

-- sales：可開報價，刻意不含 quotation.cost.read。
-- 若貴公司業務需要看成本，在角色管理勾選即可，不需改程式碼。
INSERT INTO role_permissions (role_code, permission)
SELECT 'sales', p FROM unnest(ARRAY[
  'customer.read', 'customer.write',
  'project.read', 'project.write',
  'quotation.read', 'quotation.write',
  'contract.read'
]) AS p;

-- pm：專案與合約，可看成本
INSERT INTO role_permissions (role_code, permission)
SELECT 'pm', p FROM unnest(ARRAY[
  'customer.read',
  'project.read', 'project.write',
  'quotation.read', 'quotation.cost.read',
  'contract.read', 'contract.write'
]) AS p;

-- accounting：各項唯讀 + 成本 + 報表
INSERT INTO role_permissions (role_code, permission)
SELECT 'accounting', p FROM unnest(ARRAY[
  'customer.read',
  'project.read',
  'quotation.read', 'quotation.cost.read',
  'contract.read',
  'report.read'
]) AS p;

-- viewer：各項唯讀，不含成本
INSERT INTO role_permissions (role_code, permission)
SELECT 'viewer', p FROM unnest(ARRAY[
  'customer.read', 'project.read', 'quotation.read', 'contract.read'
]) AS p;


-- 公司資料的初始列。抬頭與聯絡資訊留空，由使用者於系統設定填寫。
INSERT INTO company_profile (id, name, logo_svg, quote_terms) VALUES (
  1,
  '',
  $svg$<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100" width="100" height="100">
  <path d="M 20 80 H 40 A 40 40 0 0 0 80 40 V 20"
        fill="none" stroke="#1F3A4D" stroke-width="19" stroke-linecap="butt"/>
  <path d="M 23 70.5 V 89.5" stroke="#B87333" stroke-width="6" stroke-linecap="butt"/>
  <path d="M 70.5 23 H 89.5" stroke="#B87333" stroke-width="6" stroke-linecap="butt"/>
</svg>$svg$,
  $terms$一、報價有效期限
本報價單自開立日起三十日內有效，逾期請洽本公司重新報價。材料價格如遇原廠或市場大幅波動，本公司保留於有效期內調整之權利，並於調整前通知貴方。

二、付款條件
簽約時支付訂金百分之三十；主要材料進場時支付期中款百分之四十；驗收合格後七日內支付尾款百分之三十。

三、工期
自簽約並收訖訂金之次日起算，預計 ____ 個工作天完工。因天候、政府管制、他項工程延誤、或現場條件與勘查時不符等非可歸責於本公司之事由，工期順延，本公司不負遲延責任。

四、報價不含項目
下列項目未包含於本報價，如需施作請另行議價：
1. 電源配線至室外機電源端點前之所有電氣工程與開關箱
2. 泥作、油漆、天花板開孔與開孔後之修補
3. 吊車、鷹架、施工架等假設工程
4. 消防管線、灑水頭之遷移或改道
5. 建築結構補強、防水工程
6. 建物管理委員會規費、施工保證金、進場管制費用
7. 既有設備之拆除與廢棄物清運

五、追加與變更
施工期間如因現場條件變更、貴方需求調整或第三方因素而須增減項目，須經雙方書面確認數量與單價後始得施作。未經書面確認之追加項目，本公司不予施作，亦不負保固責任。

六、保固
1. 本公司施工品質保固一年，自驗收合格日起算。
2. 設備本體之保固依原廠規定辦理，本公司協助辦理報修。
3. 下列情形不在保固範圍：人為操作不當、未依原廠規定維護保養、自然災害、他人施工所致之損壞、貴方自行變更或加裝。
4. 保固期內非因本公司事由之出勤，得酌收出勤費用。

七、稅金
本報價單所列單價與小計均為未稅金額，另加百分之五營業稅。

八、驗收
完工後本公司以書面或通訊軟體通知貴方會同驗收。貴方應於通知送達後七日內完成驗收；逾期未完成驗收亦未以書面提出具體異議者，視同驗收合格。

九、數量結算
本報價單所列數量為估算值，實際結算依現場施作數量計算，單價不變。數量增減超過百分之十時，雙方得就單價另行協議。

十、其他
本報價單經貴方簽章回傳後，即與合約有同等效力。未盡事宜依中華民國相關法令及一般商業慣例辦理。$terms$
);
