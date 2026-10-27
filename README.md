# 個人交易平台

個人用的加密貨幣交易平台，在自己的電腦上執行。支援 **Binance、Bybit、OKX、Bitget、Pionex**，分成三個模組，每個模組都可以選擇要不要串接 **Gemini AI**：

| 模組 | 功能 | Gemini（可選） |
|---|---|---|
| **策略回測** | 用 Python 寫策略，拿交易所歷史 K 線、CSV 或模擬資料回測。會算手續費、滑價、槓桿、停損停利，也能用網格搜尋找參數 | 審查策略並產生改良版程式碼、分析回測結果 |
| **交易平台** | 把回測過的策略部署成機器人，分「模擬」和「實盤」兩種模式，另外有手動下單面板 | 檢討機器人的表現；「AI 守門」會在每次下單前請 Gemini 審核，Gemini 可以否決 |
| **倉位管理** | 定期同步各交易所的餘額和合約持倉，換算成 USD 彙總，並自動記錄開倉、平倉、加減倉和餘額變動 | 分析資產配置與風險 |

## 交易所 API 版本

所有連接器都直接呼叫交易所目前的 REST API，沒有透過第三方 SDK。實作在 `backend/app/exchanges/`。

| 交易所 | API | 簽名方式 | 支援市場 |
|---|---|---|---|
| Binance | Spot `/api/v3`、USDⓈ-M Futures `/fapi`（帳戶、持倉用 `/fapi/v3`） | HMAC-SHA256 hex | 現貨、USDT 永續 |
| Bybit | **V5** 統一 API `/v5`（需要統一交易帳戶 UTA） | HMAC-SHA256 hex | 現貨、USDT 永續 |
| OKX | **V5** `/api/v5` | HMAC-SHA256 base64 + Passphrase | 現貨、USDT 永續（自動換算張數 ctVal） |
| Bitget | **UTA V3** `/api/v3`（帳戶需要升級成統一帳戶） | HMAC-SHA256 base64 + Passphrase | 現貨、USDT 永續 |
| Pionex | Open API v1 `/api/v1` | HMAC-SHA256 hex | 現貨（API 沒有開放網格機器人） |

測試網和模擬盤：Binance 用 testnet，Bybit 用 testnet，OKX 和 Bitget 用模擬盤，分別透過 `x-simulated-trading` 和 `paptrading` header 切換。在新增 API 帳戶時勾選「測試網」即可。

合約一律假設為**單向持倉模式**（one-way），OKX 使用全倉模式。

## 安裝與啟動

需要 Python 3.11 以上。

```bash
python -m venv .venv
source .venv/bin/activate          # Windows: .venv\Scripts\activate
pip install -r backend/requirements.txt
cp .env.example .env               # 選填
cd backend
python -m app.main                 # 開啟 http://127.0.0.1:8000
```

資料庫、K 線快取和加密金鑰都存放在 `data/` 資料夾。**請備份 `data/.master.key`**，這個檔案遺失後，已經儲存的 API 金鑰就無法解密。

執行測試：

```bash
cd backend && python -m pytest
```

## 撰寫策略

把策略檔放在 `strategies/*.py`，啟動時會自動匯入；也可以直接在網頁編輯器裡寫。

```python
class SmaCross(Strategy):
    params = {"fast": 20, "slow": 60}

    def init(self, data):                      # 預先計算指標
        self.fast = ta.sma(data.close, self.p.fast)
        self.slow = ta.sma(data.close, self.p.slow)

    def next(self, i, data):                   # 第 i 根 K 線收盤後呼叫
        if ta.crossover(self.fast, self.slow, i):
            return 1.0                         # 目標曝險：1 = 全倉做多，0 = 空手，-1 = 全倉做空（只限永續）
        if ta.crossunder(self.fast, self.slow, i):
            return 0.0
        return None                            # 維持目前倉位
```

- 回測時，訊號在 K 線收盤產生，下一根 K 線開盤成交，不會用到未來資料
- 實盤和模擬只會使用**已收盤**的 K 線，每根 K 線收盤後執行一次。回測和實盤共用同一份策略程式碼
- 內建指標：`sma ema rsi macd bollinger atr stdev highest lowest crossover crossunder`
- 範例策略有三個：雙均線交叉、RSI 均值回歸、唐奇安通道突破（以 ATR 控制部位大小）

## 建議的上線流程

1. 先回測，再用**不同的日期區間**驗證（樣本外測試），避免只用最佳化找到的參數
2. 開「模擬交易」機器人跑一段時間，用真實行情模擬成交，不會下真單
3. 在交易所的測試網或模擬盤帳戶跑實盤模式
4. 用小資金上正式實盤。可以設定「最大回撤停止」，虧損達門檻時機器人會自動平倉並停止

## 安全注意事項

- 這個平台會**執行你寫的 Python 策略程式碼**，而且沒有登入驗證。只能在本機執行（預設綁定 `127.0.0.1`），**不要直接開放到網路上**
- 交易所 API Key 只開「讀取＋交易」權限，**不要開提幣權限**，並綁定 IP 白名單
- 機器人的手續費是用 0.1% 估算的，實際費用以交易所帳單為準
- 停止機器人**不會自動平倉**，需要時請手動下單平倉

## 專案結構

```
backend/app/
  exchanges/   五家交易所連接器（簽名、K 線、餘額、持倉、下單、精度）
  backtest/    指標、策略介面、回測引擎、資料來源（交易所/CSV/模擬）
  trading/     機器人執行器（模擬/實盤、AI 守門、回撤停止）
  portfolio/   快照、USD 估值、倉位變動偵測
  ai/          Gemini generateContent 用戶端與各模組提示詞
  routers/     REST API
frontend/      單頁介面（原生 JS + Chart.js）
strategies/    範例策略
```
