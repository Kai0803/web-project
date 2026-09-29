# CLAUDE.md

## 專案

小型空調工程公司的整合管理系統，涵蓋三塊業務：

1. **空調配管工程** — 報價、合約、施工、驗收、請款
2. **客戶與供應鏈** — 客戶、供應商、料件、採購、進料、庫存
3. **內部專案管理** — 排程、工班派工、工時、進度、成本控管

三者不是獨立系統，共用同一個核心（客戶與專案）。設計時避免切成三套各自為政的模組。

## 技術棧

| 層 | 選型 | 理由 |
| --- | --- | --- |
| 後端 | Rust | 使用者指定（含學習目的） |
| 前端 | Vue 3 + TypeScript | 使用者指定 |
| 資料庫 | PostgreSQL（容器化） | 排程衝突可用 exclusion constraint 於 DB 層強制；jsonb 供應商規格；DDL 可交易 |
| 部署 | Podman，`podman kube play` | 本機開發測試，後續移至自架 server |

尚未定案：Rust web 框架（建議 axum + sqlx + tower-http）、Vue component library、repo 目錄結構。這些在 G2 設計關卡定案後回填此表。

## 開發流程

本專案依 `.claude/skills/dev-workflow` 的 G1→G5 關卡推進，每關結束需使用者確認才進下一關。四個審查 agent 唯讀，所有程式碼修改由主 agent 執行。

進 G4 審查前先自行跑過 `cargo clippy`、`cargo test`、`tsc --noEmit`。

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
- **金額**：以整數最小單位（分）儲存，或使用 `rust_decimal`。一律不用浮點數
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
