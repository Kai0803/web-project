---
name: "dev-workflow"
description: "Structured Rust and TypeScript/Vue software development workflow with gated reviews for security, code quality, architecture/scalability, and data structures/algorithms. Use when building, refactoring, or reviewing a Rust backend or TypeScript frontend project end to end."
---
<!-- 原由 agent_tools 從 src/skills/dev-workflow/SKILL.md 產生，已於本 repo 手動改寫為 Rust / TypeScript 版。
     修改後請同步回源頭，否則下次產生會被覆蓋。 -->
# Dev Workflow

以固定關卡推進 Rust 後端與 TypeScript / Vue 前端專案開發，並在每個關卡由專責 subagent 從四個面向把關。

## When to Use
- 要新增一個功能模組，但還沒想清楚邊界與資料結構
- 已經寫完程式碼，需要一次完整的資安 / 品質 / 架構 / 效能審查
- 重構既有程式，想確認不會破壞介面契約
- 需要一份可交付的審查報告

不適用：單行修改、純文件更新、CI/CD 設定。

## Gates

依序推進，不可跳關。每關結束前先向使用者確認再進下一關。

### G1 需求釐清
釐清並寫下：
1. **目標**：這次要解決什麼問題，不解決什麼問題。
2. **非功能需求**：預期資料量、吞吐、延遲上限、併發數、可用性要求。沒有數字就問，不要自行假設。
3. **成功條件**：怎樣算做完，用什麼驗證。
4. **範圍邊界**：會動到哪些既有模組，哪些不動。

若需求包含多個獨立子系統，先拆解再逐一走完整流程。

### G2 設計
先自行提出設計草案（模組邊界、介面契約、擴展點、核心資料結構），再送審：

- 呼叫 `architecture-reviewer` 檢視模組邊界、耦合方向、擴展性
- 呼叫 `algorithm-advisor` 檢視資料結構選型與時間/空間複雜度

兩者可平行呼叫。收到 findings 後修正設計，Blocker 與 High 必須解決才進 G3。

前後端分離專案在此關必須定案 API 契約的單一事實來源，不留到實作時各自決定。

### G3 實作
- 依 G2 定案的邊界逐模組實作，一次一個模組。
- 在系統邊界（外部輸入、API、檔案、DB）做驗證，內部不重複驗證。
- 同步寫測試：正常路徑、邊界值、錯誤路徑。
- 實作中若發現設計有誤，退回 G2 修正設計，不要在 G3 偷改邊界。
- 進 G4 前先自行跑過 `cargo clippy`、`cargo test`、`tsc --noEmit`，把機器抓得到的問題清乾淨，審查人力留給工具抓不到的地方。

### G4 審查
依序呼叫，前一關的 Blocker 修完才進下一關：

1. `security-auditor` — 資安弱點
2. `code-reviewer` — 正確性、可讀性、測試、錯誤處理
3. `architecture-reviewer` — 實作是否偏離 G2 設計
4. `algorithm-advisor` — 實際複雜度與熱點

資安排第一：Blocker 級資安問題會讓後續審查白費。

### G5 交付
彙整成一份報告（見 [Dev Review Report](./assets/dev-review-report.md)），內容包含：
- 各面向的 findings 與處置狀態
- 已知但未修的風險與理由
- 後續建議事項

## Severity

四個 agent 共用同一套分級，報告可直接合併：

| 等級 | 定義 | 處置 |
| --- | --- | --- |
| Blocker | 資料外洩、資料損毀、功能完全不可用 | 必須修，擋交付 |
| High | 正確性錯誤或明確的效能/安全風險 | 交付前修 |
| Medium | 可維護性或擴展性問題，短期可運作 | 排入後續 |
| Low | 一致性、命名、文件缺漏 | 有餘裕再修 |
| Nit | 純風格偏好 | 可忽略 |

每筆 finding 必須含：檔案位置、問題描述、影響、建議修法。缺項視為報告不完整，退回重做。

## Checklists

| 面向 | 清單 | 對應 agent |
| --- | --- | --- |
| 資安 | [Security Checklist](./references/security-checklist.md) | security-auditor |
| 程式品質 | [Code Review Checklist](./references/code-review-checklist.md) | code-reviewer |
| 架構與擴展性 | [Architecture Checklist](./references/architecture-checklist.md) | architecture-reviewer |
| 資料結構與演算法 | [DSA Checklist](./references/dsa-checklist.md) | algorithm-advisor |

## Rules

- **專責 agent 唯讀**：四個 agent 只回報 findings，不改程式碼。所有修改由主 agent 執行，避免多 agent 覆寫同一檔案。
- **agent 可執行的指令限唯讀分析工具**：`cargo clippy`、`cargo audit`、`cargo deny check`、`cargo fmt --check`、`cargo tree`、`tsc --noEmit`、`npm audit`、`npm run lint`。不得執行建置、測試、安裝、啟動服務或任何寫入檔案的指令。
- **不擴大範圍**：只做 G1 界定的事。審查中發現的無關問題記在報告的「後續建議」，不當場改。
- **有依據**：findings 必須指向實際檔案與行號，不臆測不存在的程式碼。
- **工具與人工分工**：clippy 與 tsc 已報出的問題不重複列為 finding，只做摘要；agent 的價值在工具抓不到的判斷。
- **語言**：預設繁體中文，程式碼識別字與工具名稱保留原文。

## Output Contract
- 每個關卡結束時輸出該關卡產物並取得使用者確認
- G4 每個 agent 的 findings 依嚴重度排序
- G5 必須使用 [Dev Review Report](./assets/dev-review-report.md) 格式
- 明確列出未解風險與假設
