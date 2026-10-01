# 空調配管工程管理系統

小型空調工程公司的整合管理系統。第一階段涵蓋客戶、專案 / 案場、報價單（含版本與核准）、報價轉合約。

後續規劃見 [CLAUDE.md](CLAUDE.md)：第二階段供應鏈（料件、BOM、採購、庫存），第三階段專案執行（排程、派工、工時、成本）。

## 啟動

```bash
podman build -t acms-app:latest .
podman kube play deploy/acms.yaml
```

第一次啟動後建立管理員帳號（密碼至少 10 字元）：

```bash
podman exec acms-app acms-app create-admin <帳號> <密碼>
```

開啟 http://localhost:8080 登入。

常用指令：

```bash
podman logs -f acms-app                      # 查看日誌（JSON 格式）
podman kube down deploy/acms.yaml            # 停止，保留資料
podman volume rm acms-pgdata acms-backup     # 清除資料與備份
```

## 開發

後端需要一個可連線的 PostgreSQL 供 sqlx 在**編譯期**驗證 SQL：

```bash
podman run -d --name acms-pg-dev -p 5432:5432 \
  -e POSTGRES_USER=acms -e POSTGRES_PASSWORD=devpass -e POSTGRES_DB=acms_dev \
  postgres:17-alpine

export DATABASE_URL='postgres://acms:devpass@localhost:5432/acms_dev'
cargo test --workspace
```

PDF 測試需要一個含中文字型的目錄。容器裡是 Noto CJK，macOS 上要另外指定，
否則那兩個測試會以 `NoCjkFont` 失敗：

```bash
export FONT_DIR=/System/Library/Fonts     # macOS
```

前端：

```bash
cd web && npm install && npm run dev      # http://localhost:5173，API 轉給後端
```

### 改了 SQL 之後

`.sqlx` 是編譯期查詢驗證的離線快取，已納入版控。它與 schema 不同步時**容器建置會直接失敗**——這是刻意的，它逼著 migration 與查詢一起更新。

```bash
cargo sqlx prepare --workspace -- --all-targets
```

`--all-targets` 不能省。少了它，快取只含 `src/` 的查詢，`cargo clippy
--all-targets` 與 `cargo test` 在沒有 `DATABASE_URL` 時會以
`E0282 type annotations needed` 編譯失敗——而失敗的是測試檔，
`cargo clippy --lib --bins` 仍然乾淨，所以很容易誤判為沒問題。

### 改了 API 型別之後

前端的 TypeScript 型別由後端的 OpenAPI 規格產生，契約只有一份來源：

```bash
cargo run -p acms-app openapi > web/openapi.json
cd web && npm run api:types && npm run typecheck
```

## 架構

```
crates/domain   純業務邏輯：金額進位、報價狀態機、授權判斷
                Cargo.toml 不含 sqlx / axum / serde——domain 寫 use sqlx 會編譯失敗
crates/app
  ├ api         handler、DTO、OpenAPI
  ├ service     use case：交易邊界與編排
  ├ db          sqlx 查詢，函式收 executor 而非 pool
  ├ auth        密碼雜湊與 session（機制；授權政策在 domain）
  └ pdf         報價單排版（typst）
web             Vue 3 + TypeScript，型別由 OpenAPI 產生
```

### 幾個貫穿全系統的規則

- **金額全程是字串**，從 `NUMERIC` 到 JSON 到前端型別。前端不做任何試算，一律由後端計算後回傳，因此畫面上的數字與列印出來的報價單必然一致。
- **進位只有一份實作**，在 `acms_domain::money`。`rust_decimal` 的預設 `round()` 是銀行家進位而非四捨五入，全專案禁止直接呼叫。
- **每列小計四捨五入至整數元，合計為各列相加**，客戶逐列驗算對得上。
- **不變量交給資料庫**：唯一核准版本、核准後明細凍結、金額為整數元等，都是約束或 trigger，應用層的檢查是第二道防線而非唯一防線。
- **成本欄位依權限在序列化階段剝除**，無權限者收到的 JSON 沒有該欄位，不是前端隱藏。

## 環境變數

| 變數 | 預設 | 說明 |
| --- | --- | --- |
| `DATABASE_URL` | 無（必填） | PostgreSQL 連線字串 |
| `PORT` | `8080` | 服務埠號 |
| `STATIC_DIR` | `web/dist` | 前端建置產物位置 |
| `FONT_DIR` | `/usr/share/fonts` | 報價單 PDF 的字型來源，需含中文字型 |
| `COOKIE_SECURE` | `false` | 以 HTTPS 提供服務時設為 `true` |
| `TRUST_PROXY` | `false` | 置於反向代理之後時設為 `true` |
| `LOG_FORMAT` | 人眼可讀 | 設為 `json` 輸出結構化日誌 |

## 備份

部署含一個每日 `pg_dump` 的 sidecar，輸出至獨立的 volume，保留最近 14 份。還原：

```bash
podman exec acms-backup pg_restore -h 127.0.0.1 -U acms -d acms /backup/<檔名>.dump
```
