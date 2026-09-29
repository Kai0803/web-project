<!-- 原由 agent_tools 從 src/skills/dev-workflow/references/security-checklist.md 產生，
     已於本 repo 手動改寫為 Rust / TypeScript 版。修改後請同步回源頭，否則下次產生會被覆蓋。 -->
# Security Checklist (Rust + TypeScript)

審查順序：先掃 A（注入類，最常見 Blocker），再依序往下。
I 節為前端專章，審查 Vue / TypeScript 時適用。

## A. 注入與外部指令
- SQL 一律使用參數綁定或 `sqlx::query!` / `query_as!` 編譯期巨集，不用 `format!` / `+` 拼接 SQL。
- 動態組出的 `ORDER BY` / 欄位名以白名單對照表映射，不直接代入使用者輸入。
- `std::process::Command` 直接指定執行檔與參數陣列，不經 `sh -c`。
- 路徑、檔名不由使用者輸入直接拼接（見 D 節）。

## B. 反序列化與請求解析
- serde 反序列化目標為具體型別，不用 `serde_json::Value` 後才臨時取欄位。
- 需要嚴格契約時加 `#[serde(deny_unknown_fields)]`。
- 請求 body 有大小上限（axum `DefaultBodyLimit`、multipart 逐欄位限制），避免記憶體耗盡。
- 反序列化成功不等於值合法；範圍、長度、格式仍需驗證。

## C. 認證、授權與 Secrets
- 沒有硬編碼的密碼、API key、token、連線字串。
- Secrets 來自環境變數或 secret manager，缺少時啟動即明確失敗，不使用預設值。
- 密碼雜湊使用 `argon2` 或 `bcrypt` crate，不用 `sha2` / `md5` 裸雜湊。
- 比對 token、簽章、雜湊使用常數時間比較（`subtle::ConstantTimeEq`），不用 `==`。
- 授權檢查發生在每個受保護端點（extractor 或 middleware），不只在前端隱藏按鈕。
- JWT 驗證演算法白名單，拒絕 `alg: none`；設定並檢查過期時間。
- Session / cookie 設定 `HttpOnly`、`SameSite`、正式環境 `Secure`。

## D. 輸入驗證與邊界
- 所有外部輸入（HTTP、檔案、環境變數、訊息佇列）在進入系統時驗證型別、範圍、長度。
- **對不可信輸入禁止 `.unwrap()` / `.expect()` / 直接索引 `v[i]`** — 這些會 panic，等同單一請求層級的 DoS。
- 路徑穿越：使用者提供的路徑經 `canonicalize()` 後確認 `starts_with` 允許目錄；或完全不採用使用者檔名，改為伺服器產生。
- 上傳檔案驗證 magic bytes 與大小，不只信 `Content-Type` 與副檔名。
- 金額、數量等算術使用 `checked_*` / `saturating_*`；release 模式整數溢位會靜默 wrap（見 G 節）。
- Rust 的 `regex` crate 有線性時間保證，不需擔心 ReDoS；但 JS 端的 `RegExp` 需避免巢狀量詞。

## E. 加密與隨機
- 安全用途的隨機值使用 `OsRng` / `getrandom`，不用可預測的來源。
- 不自行實作密碼學演算法；使用 `ring` 或 RustCrypto 系列。
- TLS 驗證未被關閉（`reqwest` 的 `danger_accept_invalid_certs`、`ClientBuilder` 相關設定）。

## F. 錯誤處理與資訊洩漏
- 回應給使用者的錯誤與記錄到日誌的錯誤分離：對外只給類別與追蹤 ID，不給 SQL、路徑、內部型別名。
- `IntoResponse` 實作確認沒有把 `anyhow` 的完整錯誤鏈直接輸出到 HTTP 回應。
- 日誌不記錄密碼、token、完整個資。
- 確認 panic 不會整個進程崩潰（axum 預設單一 task panic 不影響其他請求，但需確認沒有在啟動或共享狀態處 panic）。

## G. Rust 特有風險
- `unsafe` 區塊：是否必要、是否有註解說明所維持的不變量、能否用安全替代方案。
- 整數溢位：debug 會 panic、release 會 wrap。金額與計數使用整數最小單位或 `rust_decimal`，並用 `checked_add` 等。
- `as` 轉型會靜默截斷（`u64 as u32`）；跨型別轉換使用 `try_into()` 並處理錯誤。
- 跨 `.await` 持有 `std::sync::Mutex` 會造成死結；非同步情境使用 `tokio::sync::Mutex` 或縮短持鎖範圍。
- `mem::transmute`、裸指標、`from_raw_parts` 一律視為高風險，需逐一說明。

## H. 依賴與供應鏈
- `Cargo.lock` 納入版控（應用程式必須）。
- 建議執行 `cargo audit` 比對 RustSec advisory。
- 建議執行 `cargo deny check` 檢查授權相容性與重複依賴。
- 前端 lock 檔納管，建議執行 `npm audit`。
- 依賴數量與來源可信度：避免為了單一小功能引入低維護度的 crate。

## I. 前端（Vue / TypeScript）
- `v-html` 僅用於已消毒內容；預設使用文字插值。使用者產生的內容一律不進 `v-html`。
- 動態 `:href` / `:src` 檢查 scheme，拒絕 `javascript:` 與 `data:`。
- 認證 token 不存 `localStorage`（XSS 即可讀取）；優先使用 `HttpOnly` cookie 搭配 CSRF token。
- API 回應在邊界做 runtime 驗證（如 zod），不因為 TS 有型別就假設後端回傳必定相符。
- 避免 `any` 與 `as` 斷言穿透型別檢查，尤其在處理外部資料處。
- 物件展開合併外部 JSON 時注意原型污染（`__proto__` / `constructor`）。

## J. 併發與資源
- 共享可變狀態以 `Arc<RwLock<_>>` / channel 管理，或改用不可變資料。
- 對外請求設定 timeout（`tower::timeout` 或 client 層級）。
- 資料庫連線池設定上限與取得逾時。
- channel 使用有界版本，避免無界佇列造成記憶體膨脹。
- 迴圈與遞迴有明確終止條件；Rust 無遞迴深度上限，過深會 stack overflow。

## 輸出要求
- Findings 依 Blocker / High / Medium / Low / Nit 排序。
- 每筆含：檔案位置、弱點類別（對應本清單章節）、攻擊情境、建議修法。
- 若某章節無法驗證（例如看不到部署設定），明列為假設而非略過。
- 若專案僅涉及後端或僅涉及前端，明確說明哪些章節不適用。
