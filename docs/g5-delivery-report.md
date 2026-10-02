# Dev Review Report — 第一階段

產出日期：2026-10-03　｜　分支：`claude/ac-service-booking-system-ad236j`　｜　HEAD：`14075cb`

## 1. 範圍

- **專案 / 模組**：空調配管工程管理系統，第一階段核心
- **本次變更目標**：客戶、專案 / 案場、報價單（版本與核准）、報價轉合約、帳號權限、報價單 PDF
- **涵蓋檔案**：Rust 10,084 行（`crates/domain` 6 檔、`crates/app` 24 檔、`tests/` 4 檔）、前端 3,065 行（`web/src` 18 檔）、5 支 migration、`deploy/acms.yaml`、`Dockerfile`
- **未涵蓋範圍與理由**：
  - 第二階段（料件、BOM、採購、進料、庫存）與第三階段（排程、派工、工時、進度）未實作，依 G1 分階段
  - **客戶對外入口未實作**。資料模型已預留（`users.kind`、`users.customer_id` 與 CHECK 約束），但所有端點目前只檢查權限、不檢查「這筆資料是不是你的」。`CurrentUser` 擋掉所有非 staff 帳號作為臨時閘門（見風險 R3）
  - 無刪除端點。重複建立的客戶只能停用，而 `CustomerPatch.is_active` 目前沒有任何端點能設定它
  - `audit_logs` 只寫不讀，沒有查詢介面（見風險 R6）

## 2. 需求摘要（G1）

- **目標**：讓一間 10–30 人、3–5 個工班的配管工程公司，把一個真實案子從客戶建檔走到合約與可交付的報價單 PDF
- **非功能需求**：同時上線 10–15 人；年專案 150–300 個；報價單 10–50 列明細；資料庫每日備份、可接受遺失上限 24 小時；金額與成本屬敏感資料需依角色控管；所有變更需可追溯
- **成功條件**：能跑完一個真實案子——客戶 → 專案 → 報價單 → 改版 → 核准 → 合約 → PDF 檔案

## 3. 設計決策（G2）

- **模組邊界**：`api`（解析組裝）→ `service`（交易邊界與編排）→ `db`（sqlx 查詢）；業務規則在獨立的 `acms-domain` crate。邊界由編譯器強制：`acms-domain/Cargo.toml` 不含 sqlx / axum / serde，寫 `use sqlx` 會 E0432（已實測）
- **核心資料結構與選型理由**：
  - 金額一律 `NUMERIC(14,4)` + `rust_decimal`，**全程以字串跨越 JSON 邊界**。`rust_decimal` 預設序列化為 JSON number，前端 `JSON.parse` 會得到 IEEE754 double，整套精度保證會在邊界上蒸發
  - 進位只有一份實作（`acms_domain::money::round_half_up`，`MidpointAwayFromZero`）。`rust_decimal` 的預設 `round()` 是銀行家進位，與 Postgres 的 `round()` 不一致
  - **逐列進位**而非先加總再進位：客戶拿計算機逐列相加必須對得上印出的合計
  - `Permission` 是封閉 enum（13 個變體），角色是資料（`role_permissions` 表）。權限必須有檢查點，憑空新增的權限不會有效果，所以不該是資料
  - 不變量下推到資料庫：partial unique index（唯一核准版本）、複合外鍵（合約的報價必屬該專案、工項的專案必與合約一致）、trigger（核准後明細凍結）、CHECK（金額為整數元）
- **目標複雜度**：全部 O(1) 或 O(n)，n 為單張報價單的明細列數（硬上限 500）。無排序、無巢狀資料結構走訪
- **已知取捨**：
  - 單表 CRUD 允許 api 直接呼叫 db，不為形式一致塞進 service（但見風險 R1）
  - `LIMIT/OFFSET` 分頁而非 keyset。實測 1,500 筆專案最深一頁 0.64 ms，keyset 省不到 0.15 ms
  - 取號用 UPSERT 取列鎖，鎖持有到 COMMIT。換得無跳號單號，代價是取號序列化——實測有 5 個數量級餘裕
  - 不統一 `customers` 與 `suppliers` 為 `parties`：生命週期、付款條件、料件關聯會分歧，統一的代價是每個查詢多一個 kind 條件
  - 施工照片不進資料庫、不與 pgdata 共用 volume

## 4. 審查結果（G4）

四個 agent **循序**執行，每關的 Blocker / High 修完才進下一關。

### 4.1 資安 — Security Auditor

整體評級：中。零 `unsafe`、SQL 全參數綁定、handler 內無任何可由外部輸入觸發的 panic。

| ID | 嚴重度 | 位置 | 問題 | 影響 | 狀態 |
| --- | --- | --- | --- | --- | --- |
| SEC-H1 | High | `SettingsView.vue`、`admin.rs` | `logo_svg` 是自由文字，前端以 `v-html` 渲染 | 儲存型 XSS。公司資料任何登入者皆可讀，所以持有 `settings.manage` 的 manager 可在 admin 的瀏覽器裡建立新 admin（同源腳本繞過全部三道 CSRF 防護） | **已修**：改 `<img src=data:>` + 寫入側驗證 |
| SEC-H2 | High | `service/quotation.rs` | 成本權限只做在讀取側 | 業務收到的 JSON 無 `unit_cost`，改一列品名按儲存就把整批成本寫成 NULL，不可回復。反向可寫入假成本 | **已修**（修法在 CR-H1 被推翻後重修） |
| SEC-H3 | High | `auth.rs`、migration | session token 明文存於 `sessions.id` | 那一欄等於 bearer 憑證：唯讀 DB 帳號或備份 volume 上的 14 份 dump 都能換成可用登入，稽核記成受害者 | **已修**：改存 SHA-256 摘要 |
| SEC-M4 | Medium | `deploy/acms.yaml` | cookie 走明文 HTTP、服務綁 0.0.0.0 | 同網段被動側錄即取得 session | **已修**：`hostIP: 127.0.0.1`、啟動時 warn |
| SEC-M5 | Medium | `extract.rs` | Origin 與請求自帶的 Host 比對 | 自我比對；且代理改寫 Host 時全部寫入 403 | **已修**：`PUBLIC_ORIGIN`（與 ARCH-A6 同一處） |
| SEC-M6 | Medium | `auth.rs`、`main.rs` | argon2 在 async worker 上跑；登入無速率限制 | 併發打 `/auth/login` 可阻塞所有 worker | **部分修**：已移到 `spawn_blocking` + 30 秒 timeout；**速率限制與帳號鎖定未做**（風險 R4） |
| SEC-M7 | Medium | `money.rs` | 業務上限超出 `numeric(14,4)` 容量；`unit_cost` 無驗證 | 合法輸入變 500 | **已修**：`MAX_AMOUNT`（CR-H3）+ `validate_unit_cost` |
| SEC-M8 | Medium | `customers.rs`、`admin.rs` | 客戶更新與公司資料更新無稽核 | `quote_terms` 是印在報價單上的法律文字，改了查不到是誰 | **已修**：兩處都補上欄位 diff |
| SEC-M9 | Medium | `entities.rs`、`extract.rs` | `users.kind` 從未被讀取 | 客戶入口上線當天所有端點一次對客戶開放 | **已修**：`kind` 進 domain，非 staff 擋在 extractor |
| SEC-M10 | Medium | `deploy/acms.yaml` | 無 securityContext，三容器以 root 執行 | 容器逃逸門檻降低 | **未修**（風險 R5） |
| SEC-M11 | Medium | `deploy/acms.yaml` | 備份先 sleep 24 小時；dump 未加密 | 每日重啟的 pod 可能永遠沒備份過 | **部分修**：已改先備份再睡 + `.last-success` sentinel；**異地與加密未做**（風險 R2） |
| SEC-M12 | Medium | `main.rs` | `create-admin` 密碼經 argv 傳入 | 進 `ps`、shell history、容器 event log | **未修**（風險 R7） |
| SEC-M13 | Medium | `main.rs` | 無 CSP 等安全標頭 | SEC-H1 缺少縱深防護 | **已修**：CSP、nosniff、Referrer-Policy |
| SEC-L1 | Low | `dto.rs` | 無 `deny_unknown_fields` | 欄位拼錯靜默生效為「沒帶」——與 SEC-H2 同一失效模式 | **已修** |
| SEC-L2 | Low | `customers.rs` | PATCH 把未送欄位寫成 NULL | — | **已修**（CR-H4） |
| SEC-L3 | Low | `api/mod.rs` | 登入是唯一無 CSRF 的寫入端點 | 實務上被無 CORS 擋住 | **已修** |
| SEC-L4 | Low | `quotations.rs` | 列表 `total` 回本頁筆數 | — | **已修**（CR-M3） |
| SEC-L5 | Low | `pdf/mod.rs` | 負稅率產生 `-005%` | DB CHECK 使其不可達 | **已修**（CR-M7 收到 domain） |
| SEC-L6 | Low | `.sqlx` | 離線快取缺測試查詢 | `clippy --all-targets` 與 `cargo test` 無法離線編譯 | **已修** |
| SEC-L7 | Low | `main.rs` | 無顯式 body limit、無列數上限 | 單一請求成本遠高於預期 | **已修**：512 KiB + `MAX_LINES` |
| SEC-L8 | Low | `auth.rs` | session 固定 8 小時，`purge_expired` 只在啟動時跑 | 過期列累積（15 列/天） | **不修**：有意識取捨 |
| SEC-L9 | Low | `admin.rs` | 可清空最後一個 admin 的角色 | 無人能管理帳號，救援路徑是容器指令 | **已修** |
| SEC-N1 | Nit | `auth.rs` | session 比對非常數時間 | token 是 256 bit `OsRng`，無實務可利用性 | **不修** |
| SEC-N2 | Nit | `SettingsView.vue` | `editable` 非 reactive | — | **已修** |
| SEC-N5 | Nit | 4 處 | `year() as i16` 截斷轉型 | 西元 32768 年前無影響 | **不修** |

**依賴掃描**（我自行執行，審查者環境未裝 cargo-audit）：3 個漏洞 + 6 個未維護警告。`quick-xml` 兩個 High 走 `typst → hayagriva → citationberg`，是 typst 的文獻引用功能，報價單模板用不到，且我們的 typst `World` 的 `file()` 無條件回 `NotFound`，不讀任何外部檔案；`rsa` 的 Marvin 時序攻擊，`cargo tree -i` 找不到使用者，代表未被啟用的 feature 拉進建置。`npm audit` 0 漏洞。

### 4.2 程式品質 — Code Reviewer

判定：有條件合併，無 Blocker。

| ID | 嚴重度 | 位置 | 問題 | 影響 | 狀態 |
| --- | --- | --- | --- | --- | --- |
| CR-H1 | High | `service/quotation.rs` | **成本依 `line_no` 承接是錯的**。`line_no` 依送來的陣列位置重新編號，刪除 / 插入 / 排序後不再對應原本那一列 | 刪掉三列中的第一列，B 接到 A 的成本、C 接到 B 的、C 原本的值永久消失。**沒有任何一方會發現**：操作者看不到成本，有權限者看到的是一組合法但錯誤的數字 | **已修**：改依來源明細 id 承接 |
| CR-H2 | High | `service/quotation.rs` | 改版只檢查來源版本是否已轉合約，但 supersede 作用於同單號全部其他版本 | 從舊版改版會把已簽約的版本標成 superseded，合約指向一份被取代的報價 | **已修**：檢查範圍擴到整個 quote_no |
| CR-H3 | High | `money.rs` | domain 上限 1e14 vs `numeric(14,4)` 的 9,999,999,999 | 數量 100 × 單價 1e8 就越界 → 500 | **已修**：`MAX_AMOUNT` + `MAX_LINES`，`totals` 也加上限 |
| CR-H4 | High | `customers.rs` | PATCH 收 `CreateCustomerRequest`，全欄位填 `Some(...)` | `PATCH {"name":"x"}` 把統編、電話、地址、備註全寫成 NULL | **已修**：`Option<Option<T>>` 三態 DTO |
| CR-H5 | High | `extract.rs`、`client.ts` | 五類 extractor rejection 回 `text/plain`，繞過錯誤信封 | 前端 `JSON.parse` 直接拋英文 SyntaxError。**最常見的觸發是數量欄位多打一個空格** | **已修**：`ApiJson`/`ApiPath`/`ApiQuery` + 前端不假設回應是 JSON + 送出前 trim |
| CR-M1 | Medium | `service/quotation.rs` | 可從非最新版本改版 | 操作者取代掉自己沒看到的那一版 | **已修**：`NotLatestVersion` |
| CR-M2 | Medium | `service/quotation.rs` | 併發改版撞 UNIQUE → 500 | 應為 409 | **已修** |
| CR-M3 | Medium | `quotations.rs` | 列表 `total` 回本頁筆數 | 前端算不出頁數 | **已修**：`count_for_project` |
| CR-M4 | Medium | `service/quotation.rs` | 空白報價單可送審、核准、轉合約 | 得到 total = 0、零工項的合約，並把專案推進到已簽約 | **已修**：`EmptyQuotation`（且檢查在交易內，見 ARCH-A3） |
| CR-M5 | Medium | `admin.rs` | 重複帳號借用樂觀鎖的訊息 | 使用者看到「請重新載入後再試」 | **已修**：`ApiError::conflict` |
| CR-M6 | Medium | `main.rs` | 打錯的 API 路徑落到 SPA fallback → 200 + HTML | 與 CR-H5 同一個傷口 | **已修**：`/api/{*path}` catch-all 404 |
| CR-M7 | Medium | `QuotationView.vue`、`pdf/mod.rs` | 稅率轉百分比有兩份實作，**前端那份把 `0.1` 顯示成 `1%`** | 稅率是隨報價儲存、未來會調整的，屆時會印錯在報價單上 | **已修**：收到 domain，回應帶 `tax_percent` |
| CR-M8 | Medium | `customers.rs` | update 無稽核 | 同 SEC-M8 | **已修** |
| CR-M9 | Medium | `admin.rs` | 可清空自己的角色 | 同 SEC-L9 | **已修** |
| CR-L1 | Low | `dto.rs` | `PageQuery` 的 `deny_unknown_fields` 讓多餘 query 參數 400 | 分享連結帶追蹤參數會壞 | **未修**（風險 R8） |
| CR-L2 | Low | `config.rs` | 過時的 `allow(dead_code)` 與註解 | — | **已修** |
| CR-L3 | Low | `stores/auth.ts` | `can()` 收裸字串，typo 讓按鈕永久消失 | 15 個呼叫點已逐一比對正確 | **未修**（風險 R9） |
| CR-L4 | Low | `service/quotation.rs` | `unwrap_or(i32::MAX)` 靜默截斷 | 會產生重複 `line_no` | **已修**：改 `expect` |
| CR-L5 | Low | `admin.rs` | `display_name` 寫入與回應不一致，且無空字串檢查 | — | **已修** |
| CR-L6 | Low | `LoginView.vue` | `?next=//evil.com` 拋未捕捉的 SecurityError | 非 open redirect | **已修** |
| CR-L7 | Low | `main.rs` | 無顯式 body limit | 同 SEC-L7 | **已修** |
| CR-L8 | Low | `QuotationView.vue` | `v-for :key="index"` 配 `v-model` | 刪中間列時 IME 組字跳到別列 | **已修**：穩定 uid |
| CR-L9 | Low | `tests/api.rs` | 測試輔助把非 JSON 回應轉成 `Null` | **這是 CR-H5 能在全綠測試下存活的原因之一** | **已修**：改為大聲失敗 |
| CR-N1 | Nit | `service/quotation.rs` | 四處一字不差的收尾 | — | **不修** |
| CR-N2 | Nit | `db/` | 回傳型別三種風格並存 | 見 ARCH-M1 | **未修**（風險 R10） |

### 4.3 架構與擴展性 — Architecture Reviewer

判定：無 Blocker。domain 純淨性、`db` 收 executor 的契約、不變量下推這三項是第二三階段最難事後補上的東西，而它們已經在了。

| ID | 嚴重度 | 位置 | 問題 | 建議調整 | 狀態 |
| --- | --- | --- | --- | --- | --- |
| ARCH-A1 | High | 3 個 handler | 5 個 `pool.begin()` 在 handler 裡，違反自訂規則（跨表即進 service） | 搬進 service | **未修**（風險 R1） |
| ARCH-A2 | High | `service/*` | 函式收 `&PgPool`，兩個 use case 無法落在同一交易 | 拆成 `foo` / `foo_tx` 兩層 | **未修**（風險 R1） |
| ARCH-A3 | High | `service/quotation.rs` | 送審的前置檢查跨兩個交易 | 檢查移進交易內 | **已修**：`Precondition` |
| ARCH-A4 | High | 7 個讀取端點 | 資源層級授權完全無接點，`can_access_customer` 零呼叫點 | 補 service 讀取函式 + 單一 scope 入口 | **未修**（風險 R3） |
| ARCH-A5 | High | `client.ts`、列表畫面 | 硬寫 `limit=200`，且下拉選單用同一個分頁列表 | 分頁 + 專屬 options 端點 | **已修** |
| ARCH-A6 | High | `extract.rs`、`config.rs` | CSRF 比對請求自帶的 Host | `PUBLIC_ORIGIN` | **已修** |
| ARCH-A7 | High | `deploy/acms.yaml` | 備份沒有離開那台機器；sentinel 無人讀；還原未演練 | 異地副本 + 失聯告警 + 演練 | **未修**（風險 R2） |
| ARCH-A8 | High | migration | `work_items` 只掛在合約上，簽約前不可能存在 | 加 `project_id` | **已修**：migration 0004 + 複合外鍵 |
| ARCH-M1 | Medium | `db/` | 回傳型別三種風格，`project.rs` 的映射有 3 份 14 行 | 統一為 `TryFrom<Row>` | **未修**（風險 R10） |
| ARCH-M2 | Medium | `admin.rs` | 7 處 SQL 寫在 handler 裡 | 抽到 `db/user.rs`、`db/role.rs` | **未修**（風險 R1） |
| ARCH-M3 | Medium | `api/error.rs` | `From<sqlx::Error>` 一律 500 | 第二階段使用者會指定唯一鍵 | **未修**（風險 R11） |
| ARCH-M4 | Medium | `audit_logs` | 只寫不讀，沒有查詢介面 | 加 history 端點 | **未修**（風險 R6） |
| ARCH-M5 | Medium | `entities.rs` | `ProjectStatus` 無轉換規則，`Quoting` / `Closed` / `Cancelled` 不可達 | 加 `validate_transition` | **未修**（風險 R12） |
| ARCH-M6 | Medium | `db/project.rs` | `$1 IS NULL OR` 過濾模式無法吃索引 | 現在不改，第二階段別複製 | **已記錄** |
| ARCH-M8 | Medium | `web/src/views/*` | 無 async 抽象、無快取、6 份重複樣板 | `useResource` + 參考資料 store | **未修**（風險 R13） |
| ARCH-M9 | Medium | 契約鏈 | 無 CI；`client.ts` 手寫列表契約；錯誤信封未綁到任何 response | — | **部分修**：列表契約已改用產生型別；**CI 與 `body = ErrorResponse` 未做**（風險 R14） |
| ARCH-L1–L6 | Low | 各處 | 權限三處維護、`ServiceError::NotFound` 當通用 404、`Page` 住在 db 層、`purge_expired` 只在啟動跑、文件與程式碼分歧、POST 無冪等 | — | **部分修**：L2 的 `allow(dead_code)` 已清 |

### 4.4 資料結構與演算法 — Algorithm Advisor

判定：**無 Blocker、無 High**。所有效能討論落在「微秒級記憶體內成本」對「毫秒級 I/O」的 3 個數量級落差上。

| ID | 嚴重度 | 位置 | 目前複雜度 | 建議做法 | 改善後 | 狀態 |
| --- | --- | --- | --- | --- | --- | --- |
| DSA-M1 | Medium | `main.rs`、`pdf/world.rs` | 字型快取 lazy 初始化 | 啟動時預熱 | 成本不變，移到開機 | **已修**（實測 0.62 s，設錯 FONT_DIR 會在開機報錯） |
| DSA-M2 | Medium | `pdf/world.rs` | 持有全部字型檔位元組 | 只留 CJK | 審查者稱 777→60 MiB | **已修，但其論證不成立**（見下） |
| DSA-L1 | Low | migration | 2 個冗餘單欄索引 | DROP | 無查詢變慢（實測） | **已修**：migration 0005 |
| DSA-L2 | Low | `db/project.rs` | `p.status::text = $1` | 改比 enum | 絕對差 ≈ 0 | **已修**（理由是移除陷阱，非效能） |
| DSA-L3 | Low | `db/quotation.rs` | `update_status` 回 bool 後再 find | `RETURNING` | −0.25 ms | **已修**（理由是刪掉永不成立的錯誤處理） |
| DSA-N2 | Nit | `pdf/mod.rs` | `group()` 混用 byte 長度與 char 索引 | 走 bytes | 目前正確但前提只在呼叫端 | **已修** |
| DSA-N3 | Nit | `pdf/world.rs` | `is_dir()` 多一次 stat | `file_type()` | −0.4 ms/行程 | **已修** |
| DSA-N4 | Nit | `domain/quotation.rs` | `current + 1` 裸算術 | `checked_add` | release 下的 wrap 會變難查的 500 | **已修** |
| DSA-N1 | Nit | `db/` | UNNEST 的 `String` clone | `as_deref()` | 500 列 −60 µs（0.9%） | **不修**：審查者說跳過沒有損失 |
| DSA-N5 | Nit | migration | `audit_logs.actor_id` 無索引 | — | 不要加：稽核寫入是每個寫入都發生 | **不修**（刻意） |

**我推翻了 DSA-M2 的論證。** 它說過濾字型會讓開發機從 777 MiB 降到約 60 MiB。實測：**過濾前後都是約 770 MiB**——macOS 上那 25 個中文家族本身就是大檔，被留下的正是大的那些。容器裡更徹底：`/usr/share/fonts` 只有 89 MiB 的 4 個 Noto CJK，沒有東西可以過濾（RSS 143 MiB，上限 512 MiB）。改動保留，但理由改成實際成立的那個：**輸出的確定性**——typst 的 fallback 會從主機剛好裝著的字型撈字，所以備註欄塞 emoji 的報價單在開發機印得出來、在容器裡是豆腐方塊。

**實測的資料庫數字**（審查者在 `acms_dev` 上以 `EXPLAIN ANALYZE` 取得，5 年後規模）：

| 查詢 | 時間 |
| --- | --- |
| `project::list` 第一頁（1,500 筆） | 0.49 ms |
| `project::list` 最深一頁（OFFSET 1450） | 0.64 ms |
| `project::list` 帶 status 過濾 | 0.25 ms |
| `quotation::items`（500 列） | 0.106 ms |
| `latest_version`（Index Only Scan Backward） | 0.016 ms |
| `replace_items`（500 列，伺服器端） | 9.4 ms，其中 7.5 ms 是每列 trigger |

## 5. 統計

| 嚴重度 | 資安 | 品質 | 架構 | DSA | 合計 |
| --- | --- | --- | --- | --- | --- |
| Blocker | 0 | 0 | 0 | 0 | **0** |
| High | 3 | 5 | 8 | 0 | **16** |
| Medium | 10 | 9 | 8 | 2 | **29** |
| Low | 9 | 9 | 6 | 3 | **27** |
| Nit | 5 | 2 | 2 | 5 | **14** |
| 合計 | 27 | 25 | 24 | 10 | **86** |

**處置**：High 16 項全部已修或已轉為有明確觸發條件的記錄風險（ARCH-A1/A2/A4/A7 四項未修，見第 7 節）。Medium 已修 20 / 部分修 4 / 未修 5。

## 6. 測試狀態

**143 個測試**（domain 單元 45、app 單元 13、db 整合 21、service 整合 24、api 端對端 38、pdf 2）。`#[sqlx::test]` 對真實 Postgres 執行，每個測試一個獨立資料庫。

- **正常路徑覆蓋**：完整，含 `full_flow_from_quotation_to_contract` 端對端
- **邊界值覆蓋**：金額進位的 midpoint（`n ≡ 10 (mod 20)` 性質生成）、`MAX_QUANTITY + 1`、`MAX_AMOUNT` 上下、`MAX_LINES` 上下、空明細、net negative 報價、分頁上下界
- **錯誤路徑覆蓋**：權限 × 動作幾乎全組合、狀態機八條合法轉換與反例、輸入格式錯誤的信封契約（5 類 rejection）、容量邊界回 400 而非 500
- **未覆蓋區域與理由**：
  - **真正的樂觀鎖衝突**（`lock_version` 失配 → 409）需要兩個並行交易，單執行緒的 router 測試做不到。目前只有 DB 層的 `status_update_respects_the_optimistic_lock` 覆蓋，而**前端的衝突提示依賴 `CONFLICT` 這個 code**。這個缺口已寫在測試的註解裡
  - **前端零測試**。刻意不做試算，所以沒有業務邏輯需要測；但剩下的純函式承擔全部顯示正確性，而其中一個（`taxPercent`）確實有 bug 並已修。建議補 vitest + 兩三個純函式測試（`formatMoney`、`client.ts` 的錯誤解析）
  - `NoCjkFont` 路徑：`FONTS` 是 `OnceLock`，同一行程內無法同時測有字型與無字型。已由啟動預熱在實務上覆蓋
  - ARCH-A3 的競態窗口：需要併發測試，未實證（邏輯上窗口存在，已修掉）

## 7. 未解風險

| ID | 風險 | 觸發條件 | 目前緩解 | 為何暫不處理 |
| --- | --- | --- | --- | --- |
| **R1** | 5 個交易開在 handler 而非 service；service 函式收 `&PgPool` 所以 use case 無法互相包覆；`admin.rs` 有 7 處 SQL | **第二階段開始寫採購或進料流程之前**。進料要同時更新採購單已收量與庫存結存，那必須原子化 | 現有流程正確，有 service 層的回滾測試 | 無現存缺陷，而在 G4 裡做會讓變更面過大。延後成本隨第二階段每個 handler 線性成長 |
| **R2** | 備份沒有離開那台機器；`.last-success` 無人讀；還原從未演練 | **移到 server 時**，不可再延 | 每日 dump 至獨立 volume、保留 14 份、先備份再睡 | 需要你的 server 環境才能決定異地目標 |
| **R3** | 資源層級授權無接點。7 個讀取端點只檢查權限、不檢查擁有者 | **寫第一個客戶入口端點之前**，不是之後 | `CurrentUser` 擋掉所有非 staff 帳號；`can_access_customer` 已寫好待接 | 第一階段無 `customer_contact` 帳號，不可利用 |
| **R4** | 登入無速率限制與帳號鎖定 | 服務對外曝露時 | argon2 在 blocking pool、30 秒 timeout、密碼至少 10 字元、`hostIP: 127.0.0.1` | 10–15 人內部系統，離職者慢速字典攻擊是主要情境，優先序低於 R2 |
| **R5** | 容器無 securityContext，三容器以 root 執行 | 移到 server 時，與 TLS 同批 | 純內部、只發布到 loopback | 本機開發不可利用 |
| **R6** | `audit_logs` 只寫不讀，無查詢介面 | **第一次有人問「這個數字是誰改的」**——預期交付後第一個月內 | 資料已完整寫入且 diff 格式正確，可下 SQL 查 | 需要一個端點 + 一個畫面，約 2–3 小時 |
| **R7** | `create-admin` 密碼經 argv 傳入，進 `ps` 與 shell history | 移到 server 時 | 只在有容器執行權時可用（等於已有主機權） | 改成 stdin 需同步改 README 的指令 |
| **R8** | `PageQuery` 的 `deny_unknown_fields` 讓多餘 query 參數回 400 | 有人分享帶追蹤參數的連結時 | 列表現在會帶正確的 limit/offset | 一行改動，但要判斷 query string 是否該與 body 用同一標準 |
| **R9** | `auth.can()` 收裸字串，typo 讓按鈕永久消失且無人察覺 | 第二階段新增 15–20 個權限時 | 現有 15 個呼叫點已逐一比對正確 | 正解是讓 utoipa 把 `Permission::ALL` 產生成 enum schema |
| **R10** | `db` 回傳型別三種風格並存，`project.rs` 有 3 份重複映射 | **第二階段開始寫 `db/material.rs` 之前**。6 張新表 × 4 查詢 = 24 份手寫映射 | 欄位錯誤是編譯錯誤（但對應錯誤不是） | 純機械改動，143 測試是安全網 |
| **R11** | `From<sqlx::Error>` 一律 500 | **第一個讓使用者指定唯一鍵的實體**（料件代碼、供應商代碼） | 第一階段所有 code 由 numbering 產生，使用者無法指定 | `ServiceError::from_db` 已有正確做法，只需擴充並讓所有寫入路徑經過它 |
| **R12** | `ProjectStatus` 無轉換規則，`Quoting`/`Closed`/`Cancelled` 不可達 | 第三階段排程與進度要寫專案狀態時 | 唯一寫入者是轉合約 | 不可達的狀態值比缺少的狀態值更糟，會讓人以為有這個流程 |
| **R13** | 前端無 async 抽象、無快取、6 份重複樣板 | **第二階段的 BOM 編輯畫面**（一個畫面同時操作工項、料件主檔、BOM 明細、單位換算） | 目前每畫面 2 個請求，無 N+1 | `useResource` 約 30 行 + 參考資料 store，約 2 小時 |
| **R14** | 契約漂移無 CI 檢查；錯誤信封的 schema 未綁到任何 response | 下一次改 DTO 卻忘記重新產生型別時 | 目前同步（已驗證無 diff）；列表契約已改用產生型別 | 沒有 CI runner；最小解是 `make check-contract` + 提交前跑 |
| **R15** | `quotation_items` 的 `FOR EACH ROW` trigger 是每列 5.9 µs，佔 500 列重寫的 80% | **第二階段寫 `bom_items` 的鎖定保護時**。預估 30–50 萬列 | 第一階段 50 列約 0.75 ms，佔一次存檔 15% | 要用 `FOR EACH STATEMENT` 配 transition table，不能照抄這個 trigger |
| **R16** | **27 個 commit 無法推送**。這台 Mac 沒有 GitHub 憑證 | 現在 | 全部已 commit 在本機 `claude/ac-service-booking-system-ad236j` | 需要你在 GitHub 加 SSH 公鑰或建一個 PAT |

## 8. 假設

1. **第二、三階段的需求細節未確認。** 關於 BOM 用於報價估料、進料需同時更新採購單與庫存、派工跨四張表的推論來自配管工程業的一般流程與 G2 的擴展點表，不是確認過的需求。R1、R3、R10 的觸發條件綁在這些推論上
2. **資料量估算**依「30 人、5 工班、300 專案/年、5 年」推算。即使全部放大 10 倍，「Postgres 無壓力」的結論不變
3. **typst 的排版時間未量測**（引用 `main.rs` 註解的「數百毫秒」）。它是全系統第一熱點，而 repo 裡沒有留下 benchmark。若 PDF 延遲成為抱怨來源，該做的第一件事是量測 `typst::compile` 與 `typst_pdf::pdf` 的分帳
4. **公司抬頭、統編、電話、地址尚未填寫。** PDF 目前印「（尚未設定公司抬頭）」。填寫位置：登入後 **系統設定 → 公司資料**
5. **還原演練未進行。** G2 自己要求「還原程序需實際演練過一次才算數」。我驗證過 `pg_dump` + `pg_restore` 到 scratch 資料庫（16 表、6 角色），但那不是正式環境的演練
6. **所有微秒級的數字是分析推算**，非 criterion 量測。資料庫的毫秒數字是 `EXPLAIN ANALYZE` 實測
7. 審查 agent 在 `acms_dev` 灌過測試資料並以 `ROLLBACK` 清掉。唯一殘留是 IDENTITY 計數器往前跳（id 不需連續，`public_id` 才是對外識別字）

## 9. 交付判定

**決定：有條件交付。**

**擋交付項目**：無。零 Blocker，16 個 High 全部已修或已轉為有觸發條件的記錄風險。系統可完成 G1 的成功條件——客戶 → 專案 → 報價單 → 改版 → 核准 → 合約 → PDF，我已實際跑過。

**交付的條件**（這三項在「移到 server」之前必須完成，不是建議）：

1. **R2 — 備份異地化與還原演練。** 這套系統裝的是報價單、合約與請款資料；對一家 10–30 人的工程公司，丟掉它等於丟掉三年的工程紀錄與帳。目前它靠「同一顆磁碟上有第二份副本」擋著
2. **R5 + TLS 反向代理 + `COOKIE_SECURE=true` + `PUBLIC_ORIGIN`。** 前兩者已知，後兩者現在會在設定不一致時拒絕啟動
3. **公司抬頭填寫**（系統設定 → 公司資料），否則報價單印不出抬頭

**後續建議（依優先序）**：

1. **R16**：解決推送問題，否則這 27 個 commit 只存在這台 Mac 上
2. **R2**：備份異地 + 還原演練（移到 server 時）
3. **R3**：資源層級授權——**在寫第一個客戶入口端點之前**
4. **R1 + R10**：service 層的交易組合性與 `db` 回傳型別統一——**在第二階段寫第一個新模組之前**。這兩項的延後成本隨第二階段的程式碼線性成長
5. **R6**：稽核查詢介面（約 2–3 小時，預期交付後第一個月就會需要）
6. **R14**：契約檢查腳本 + 提交前跑
7. **R13**：前端的 `useResource`（第二階段 BOM 畫面之前）
8. **R4 / R7 / R12 / R15**：各自的觸發條件見第 7 節

---

### 本次 G4 的一個觀察

四關之中最嚴重的一條（CR-H1）**是我在前一關的修正本身引入的 bug**，而我為那個修正寫的回歸測試剛好落在正確的子集合裡（只改品名，列數與順序不變），所以它通過了並給了我不該有的信心。這是循序審查而非平行審查的具體價值：code-reviewer 看到的是 security-auditor 的修正結果，而不是原始程式碼。

另一個方向也成立：我推翻了 DSA-M2 的記憶體論證，因為我能跑 build 而審查 agent 不能。審查者的數字值得查核，不值得照抄。
