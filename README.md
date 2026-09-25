# 冷氣工程預約與管理系統

一個提供「對外預約」與「對內管理」的空調工程服務系統，適合冷氣清洗 / 維修 / 安裝的工程行使用。

## 功能

### 對外（一般客戶，免登入）
- 首頁瀏覽服務項目（清洗保養 / 維修 / 安裝）與價格
- 線上預約表單：填寫聯絡資訊、地址、希望日期時段、勾選服務項目
- 送出後取得「預約編號」，可用電話 + 預約編號查詢處理進度

### 對內（工程師 / 管理員，需登入）
- **儀表板**：管理員看到待確認預約、今日排程、庫存偏低零件、維修中工具；工程師看到自己的今日工單、近期工單、持有工具
- **預約 / 工單管理**：指派工程師、排定服務時間、更新狀態（待確認 → 已排程 → 服務中 → 已完成 / 已取消）、登記使用的服務項目與零件（自動計算金額）
- **服務項目（工項）管理**（管理員）：新增 / 編輯 / 刪除服務項目與價格，並可設定是否開放給客戶線上預約選擇
- **零件庫存管理**：管理員可新增 / 編輯 / 刪除零件與庫存量；工程師可查詢庫存並在工單中登記使用零件（自動扣庫存，低於警戒值會標示）
- **工具管理**：所有登入人員可借用 / 歸還工具；管理員可新增工具、送修 / 修復 / 報廢
- **人員管理**（管理員）：新增工程師 / 管理員帳號、停用帳號、重設密碼
- **施工照片**：工單可上傳施工前 / 後照片（JPG / PNG / WebP，單張 8MB 內），手機可直接開相機拍攝
- **完工簽收**：客戶於手機螢幕簽名確認，簽收後工單自動標記為已完成
- **現場快捷操作**：工單頁可直接撥號、開啟地圖導航、一鍵開始服務，手機版放大按鈕

## 安全性

- 所有會改變資料的表單皆帶 session 綁定的 CSRF token
- 登入狀態存於資料庫，重啟服務不會將所有人登出
- 登入失敗 5 次鎖定帳號 15 分鐘，另有每 IP 的嘗試次數上限
- 預設密碼與管理員設定的初始密碼，皆強制本人首次登入時更換
- helmet 安全標頭與 CSP（`script-src 'self'`，頁面無 inline script）
- 施工照片僅限有該工單權限的人員透過驗證路由讀取，不對外公開

## 技術棧
Node.js + Express + EJS（伺服器端渲染）+ SQLite（better-sqlite3，資料庫與上傳檔案存放於 `data/`）+ express-session。不需額外的前端建置流程。

### 環境變數

| 變數 | 預設 | 說明 |
| --- | --- | --- |
| `PORT` | `3000` | 服務埠號 |
| `SESSION_SECRET` | 開發用字串 | session 簽章金鑰，正式環境必須設定 |
| `COOKIE_SECURE` | `false` | 以 HTTPS 提供服務時設為 `true` |
| `TRUST_PROXY` | `false` | 置於 nginx / ingress 之後時設為 `true` |

## 開發環境啟動

```bash
npm install
npm start
```

伺服器預設啟動於 http://localhost:3000 ，首次啟動會自動建立資料表與預設測試資料：

- 服務項目（工項）、零件庫存、工具清單範例資料
- 預設帳號：
  - 管理員：`admin` / `admin1234`
  - 工程師：`engineer1` / `engineer1234`

> 正式上線前請務必修改預設密碼，並設定環境變數 `SESSION_SECRET` 為隨機字串。

若要重新產生測試資料，刪除 `data/app.sqlite` 後重新啟動即可。

## 容器化部署（Podman / Kubernetes YAML）

以 `podman kube play` 啟動，資料庫存放在 PVC volume，容器重建不會遺失資料：

```bash
podman build -t ac-service:latest .
podman kube play deploy/ac-service.yaml
```

啟動後開啟 http://localhost:3000 。常用指令：

```bash
podman pod ps                                  # 查看 pod 狀態
podman logs -f ac-service-app                  # 查看 log
podman kube down deploy/ac-service.yaml        # 停止並移除 pod（保留資料）
podman volume rm ac-service-data               # 清除資料庫
podman kube generate ac-service                # 由現有 pod 反向產生 YAML
```

`/healthz` 端點供 readiness / liveness probe 使用，會實際查詢資料庫確認連線正常。

程式碼修改後需重新 build 並重啟：

```bash
podman kube down deploy/ac-service.yaml
podman build -t ac-service:latest .
podman kube play deploy/ac-service.yaml
```

### 正式環境注意事項

`deploy/ac-service.yaml` 內的 Secret 為開發用預設值，正式環境請替換：

```bash
openssl rand -base64 32    # 將結果填入 session-secret
```

此 YAML 也可直接套用於 Kubernetes（`kubectl apply -f`），但需先將 image 推送至 registry、把 `imagePullPolicy` 改為 `IfNotPresent`，並移除 `hostPort` 改用 Service 曝露服務。另外目前 session 存於記憶體，僅支援單一 replica。
