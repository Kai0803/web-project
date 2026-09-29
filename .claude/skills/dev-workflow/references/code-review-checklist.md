<!-- 原由 agent_tools 從 src/skills/dev-workflow/references/code-review-checklist.md 產生，
     已於本 repo 手動改寫為 Rust / TypeScript 版。修改後請同步回源頭，否則下次產生會被覆蓋。 -->
# Code Review Checklist (Rust + TypeScript)

聚焦「這段程式碼對不對、好不好維護」。架構層級問題交給 Architecture Checklist，複雜度問題交給 DSA Checklist。
E 節為 Rust 專章，F 節為前端專章。

## A. 正確性
- 邊界條件正確：空集合、單一元素、最大值、負數、`None` / `null`。
- 迴圈的 off-by-one、切片邊界正確；切片越界在 Rust 會 panic 而非回傳空值。
- 整數溢位：release 模式會 wrap 而不 panic。金額、計數使用 `checked_*` / `saturating_*`。
- `as` 轉型會靜默截斷；跨型別轉換用 `try_into()` 並處理錯誤。
- 浮點數比較不使用 `==`；金額不使用 `f64`，改用整數最小單位或 `rust_decimal`。
- 迭代中未修改正在迭代的容器（Rust 由借用檢查器擋下，TS 需人工確認）。
- 時區與日期：統一以 UTC 儲存，僅在顯示層轉換。

## B. 錯誤處理
- 可失敗的操作回傳 `Result`，不用 `.unwrap()` / `.expect()` 敷衍。
- `.unwrap()` 僅出現在「不可能失敗」處，且附註解說明為何不可能。
- 錯誤型別分層：函式庫層用 `thiserror` 定義具體型別，應用層用 `anyhow` 聚合。
- 使用 `?` 傳播時，以 `.context()` 補上足以定位問題的上下文（哪個輸入、哪個資源）。
- 錯誤不被靜默吞掉；`let _ = ...` 忽略 `Result` 需有理由。
- 資源釋放依賴 RAII / `Drop`，不手動清理；TS 端用 `try/finally` 或 `onUnmounted`。
- 只在系統邊界驗證輸入，內部函式不重複防禦性檢查。

## C. 測試
- 有測試覆蓋：正常路徑、邊界值、錯誤路徑。
- 測試斷言具體結果，不只斷言「沒有 panic」或「有回傳值」。
- 測試彼此獨立，不依賴執行順序或共享可變狀態。
- 外部依賴（網路、DB、時間、隨機）被隔離或注入，測試可重現。
- 非同步測試使用 `#[tokio::test]`；涉及 DB 的測試使用交易回滾或獨立測試資料庫。
- 測試名稱說明「在什麼條件下應該有什麼結果」。
- 單元測試放 `#[cfg(test)] mod tests`，跨模組的整合測試放 `tests/`。

## D. 可讀性
- 命名表達意圖：函式是動詞、集合是複數、布林是 `is_` / `has_` 開頭。
- 函式只做一件事；超過約 50 行或巢狀超過 3 層是拆分訊號。
- 沒有魔術數字與魔術字串，改用 `const` 或 enum。
- 註解說明「為什麼」，不重述「做了什麼」。
- 沒有註解掉的死程式碼。
- 條件式為正向表述，避免多重否定。

## E. Rust 慣用法
- `cargo clippy` 無警告；有意忽略處以 `#[allow(...)]` 標註並說明理由。
- 參數優先使用借用型別：`&str` 而非 `&String`，`&[T]` 而非 `&Vec<T>`。
- 沒有為了規避借用檢查器而濫用 `.clone()`；每個 `clone` 都應是有意識的選擇。
- 用 `Option` / `Result` 的組合子（`map`、`and_then`、`ok_or`、`unwrap_or_default`）取代深層巢狀 `match`。
- 領域概念用 newtype 包裝，而非裸 `i64` / `String`（例如 `OrderId(i64)`），避免參數順序寫反。
- 該 derive 的 trait 有 derive：`Debug`、`Clone`、`PartialEq`；公開型別務必有 `Debug`。
- 回傳值不該被忽略時標 `#[must_use]`。
- 優先 `impl Trait` 或泛型；僅在需要動態分派時使用 `Box<dyn Trait>`。
- 模組可見性最小化：預設私有，只 `pub` 真正的對外介面。

## F. TypeScript / Vue 慣用法
- `tsconfig` 開啟 `strict`；`tsc --noEmit` 無錯誤。
- 沒有 `any`；未知型別先用 `unknown` 再以型別守衛收斂。
- 避免 `as` 斷言，尤其在處理 API 回應處；改用 runtime 驗證。
- 狀態以 discriminated union 表達（`{ status: 'loading' } | { status: 'error', error: E }`），不用多個各自獨立的 boolean。
- Vue 元件使用 `<script setup lang="ts">`，props 以 `defineProps<T>()` 型別化。
- composable 命名 `useXxx`，對外回傳唯讀 ref，不讓呼叫端直接改內部狀態。
- 副作用有對應清理：`onUnmounted`、`watch` 的 stop handle、事件監聽移除。
- 沒有直接操作 DOM 繞過響應式系統。

## G. 介面與相容性
- 公開函式的參數順序與命名一致；選項多時用 struct 參數而非長參數列。
- 修改既有簽章時，確認所有呼叫點已更新（Rust 由編譯器保證，TS 需確認無 `any` 遮蔽）。
- 回傳型別一致，不會有時回傳值有時回傳 `Option`。
- 公開項目有 doc comment（`///`），描述參數、回傳、可能的錯誤與 panic 條件。
- 前後端共用的資料契約有單一來源（OpenAPI 產生或共用型別定義），不兩邊各自手寫。

## H. 變更範圍
- 這次變更只做了需求界定的事，沒有夾帶無關重構。
- 沒有新增未被使用的參數、函式、抽象層。
- 刪除的程式碼確認無其他引用。

## 輸出要求
- Findings 依 Blocker / High / Medium / Low / Nit 排序。
- 每筆含：檔案位置、問題描述、影響、建議修法（附可套用的程式碼片段）。
- 明確標出「已檢查且無問題」的章節，避免使用者誤以為漏檢。
- 若已執行 `cargo clippy` 或 `tsc --noEmit`，附上結果摘要；工具已報出的問題不重複列為 finding，只挑需要人工判斷者。
