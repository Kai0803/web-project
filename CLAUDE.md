# CLAUDE.md

## 專案

小型空調工程公司的整合管理系統，涵蓋三塊業務：

1. **空調配管工程** — 報價、合約、施工、驗收、請款
2. **客戶與供應鏈** — 客戶、供應商、料件、採購、進料、庫存
3. **內部專案管理** — 排程、工班派工、工時、進度、成本控管

三者不是獨立系統，共用同一個核心（客戶與專案）。設計時避免切成三套各自為政的模組。

### 公司規模

10–30 人、3–5 個工班。同時上線使用者 10–15 人，年專案 150–300 個。第一階段資料量小，單一 Postgres 實例綽綽有餘；資料量壓力在第二階段的 BOM 明細與第三階段的工時、施工照片。

### 分階段

| 階段 | 範圍 | 狀態 |
| --- | --- | --- |
| 一 | 核心：客戶、專案 / 案場、報價單（版本與核准）、報價轉合約、帳號權限 | 進行中 |
| 二 | 供應鏈：料件主檔、BOM、供應商、採購單、進料、庫存 | 未開始 |
| 三 | 專案執行：排程、工班派工、工時、進度、成本控管 | 未開始 |

第一階段需求見 [docs/g1-requirements.md](docs/g1-requirements.md)（已確認）。

### 對外入口

第一階段**純內部系統**，不對外曝露。但資料模型與認證從一開始就預留對外擴充（使用者帳號區分內部同仁與客戶聯絡人），第二、三階段要加「客戶查專案進度」與「客戶線上詢價」時不需改動既有結構。

## 技術棧

| 層 | 選型 | 理由 |
| --- | --- | --- |
| 後端 | Rust | 使用者指定（含學習目的） |
| 前端 | Vue 3 + TypeScript | 使用者指定 |
| 資料庫 | PostgreSQL（容器化） | 已確認。排程衝突可用 exclusion constraint 於 DB 層強制；jsonb 供應商規格；DDL 可交易；sqlx 編譯期查詢檢查最成熟 |
| 部署 | Podman，`podman kube play` | 本機開發測試，後續移至自架 server |

G2 定案的其餘選型：

| 項目 | 選型 |
| --- | --- |
| Web 框架 | axum + tower-http |
| 資料庫存取 | sqlx（編譯期查詢檢查） |
| Crate 結構 | workspace 兩個 crate：`acms-domain`（純邏輯，不含 sqlx / axum / serde）與 `acms-app` |
| API 契約 | utoipa 由 Rust 型別產生 OpenAPI，前端據此產生 TS 型別，CI 檢查漂移 |
| 認證 | Cookie session 存 Postgres，`SameSite=Strict` + Origin 檢查 + 自訂 header |
| 報價單輸出 | typst 伺服器端產生 PDF |
| 前端樣式 | 手寫 CSS + 品牌色票，未採用 component library |

完整設計見 [docs/g2-design.md](docs/g2-design.md)。

## 開發流程

本專案依 `.claude/skills/dev-workflow` 的 G1→G5 關卡推進，每關結束需使用者確認才進下一關。四個審查 agent 唯讀，所有程式碼修改由主 agent 執行。

進 G4 審查前先自行跑過 `cargo clippy`、`cargo test`、`tsc --noEmit`。

**目前進度**：第一階段 G3（實作）完成，待進 G4 審查。產物於 `docs/`。

## `.claude/` 的同步注意事項

`.claude/` 下的 skill 與 agent 原本由外部的 `agent_tools` 專案產生，**已在本 repo 手動改寫為 Rust / TypeScript 版**（原為 Python 版）。

已改寫（需同步回 `agent_tools` 源頭，否則下次產生會被覆蓋）：
- `agents/` 下的 security-auditor、code-reviewer、architecture-reviewer、algorithm-advisor
- `skills/dev-workflow/SKILL.md` 與 `references/` 下四份 checklist

未改動（語言中立，維持原狀）：
- `skills/dev-workflow/assets/dev-review-report.md`
- `skills/docs-consistency/` 整組與 `agents/docs-consistency-analyst.md`

## 慣例

- **語言**：文件、UI、commit 訊息與註解用繁體中文；程式碼識別字、型別名、資料表與欄位名用英文
- **金額與數量**：資料庫用 `NUMERIC`，Rust 用 `rust_decimal`。一律不用浮點數。配管數量本身就是小數（米、公斤），與整數分混用容易產生進位錯誤
- **幣別與稅**：僅新台幣，單一公司單一帳本。營業稅 5%，報價單區分未稅 / 含稅
- **無舊資料移轉**：不需匯入既有 Excel 或系統資料
- **時間**：一律以 UTC 儲存，僅在顯示層轉換時區
- **錯誤**：函式庫層用 `thiserror` 定義具體型別，應用層用 `anyhow` 聚合
- **對外輸入**：在系統邊界驗證，內部不重複防禦性檢查
- **不可信輸入禁用 `.unwrap()`**：會造成單一請求層級的 DoS

## 領域詞彙

中文術語與程式碼識別字的對照，避免同一概念出現多種命名。

| 中文 | 識別字 |
| --- | --- |
| 客戶 | `customer` |
| 供應商 | `supplier` |
| 專案 / 案場 | `project` |
| 報價單 | `quotation` |
| 合約 | `contract` |
| 工項 | `work_item` |
| 料件 | `material` |
| 用料表 | `bom`（bill of materials） |
| 採購單 | `purchase_order` |
| 進料 | `goods_receipt` |
| 庫存 | `inventory` |
| 工班 | `crew` |
| 派工 | `assignment` |
| 工時 | `labor_hours` |
| 施工進度 | `progress` |
| 請款 | `billing` |

## 歷史

`836efb1` 至 `520d847` 為前一版以 Node.js + Express + EJS + SQLite 實作的冷氣清洗預約系統，功能完整且可運作（含 CSRF、SQLite session store、施工照片、完工簽收）。本次改版業務範圍不同（B2C 到府清洗 → B2B 工程專案），該版程式碼僅作參考，不沿用。
