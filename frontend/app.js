"use strict";

// ---------- helpers ----------
const $ = (id) => document.getElementById(id);
const state = { meta: null, strategies: [], accounts: [], bots: [], currentStrategy: null, currentRun: null, selectedBot: null, charts: {} };

async function api(path, opts = {}) {
  const res = await fetch(path, {
    headers: { "Content-Type": "application/json" },
    ...opts,
    body: opts.body && typeof opts.body !== "string" ? JSON.stringify(opts.body) : opts.body,
  });
  const text = await res.text();
  let data;
  try { data = text ? JSON.parse(text) : null; } catch { data = text; }
  if (!res.ok) {
    let msg = data && data.detail ? data.detail : text;
    if (Array.isArray(msg)) msg = msg.map((d) => `${d.loc.join(".")}: ${d.msg}`).join("; ");
    throw new Error(msg || `HTTP ${res.status}`);
  }
  return data;
}

function toast(msg, err = false) {
  const el = document.createElement("div");
  el.textContent = msg;
  if (err) el.className = "err";
  $("toast").appendChild(el);
  setTimeout(() => el.remove(), err ? 8000 : 4000);
}

async function busy(btn, fn) {
  const label = btn.textContent;
  btn.disabled = true;
  btn.textContent = "處理中…";
  try { return await fn(); }
  catch (e) { toast(e.message, true); }
  finally { btn.disabled = false; btn.textContent = label; }
}

const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
const fmt = (n, d = 2) => (n === null || n === undefined || Number.isNaN(n) ? "-" : Number(n).toLocaleString("en-US", { maximumFractionDigits: d, minimumFractionDigits: 0 }));
const usd = (n) => "$" + fmt(n, 2);
const dt = (v) => (v ? new Date(v).toLocaleString("zh-TW", { hour12: false }) : "-");
const md = (text) => DOMPurify.sanitize(marked.parse(text || ""));
const signCls = (n) => (n > 0 ? "pos" : n < 0 ? "neg" : "");

function table(columns, rows, onClick) {
  if (!rows.length) return '<p class="muted">無資料</p>';
  const head = columns.map((c) => `<th>${esc(c[0])}</th>`).join("");
  const body = rows.map((r, i) => `<tr data-i="${i}" class="${onClick ? "clickable" : ""}">${columns.map((c) => `<td>${c[1](r)}</td>`).join("")}</tr>`).join("");
  return `<table><thead><tr>${head}</tr></thead><tbody>${body}</tbody></table>`;
}
function bindRows(container, rows, onClick) {
  container.querySelectorAll("tr[data-i]").forEach((tr) => tr.addEventListener("click", () => onClick(rows[+tr.dataset.i])));
}

function kpis(el, items) {
  el.innerHTML = items.map(([label, value, cls = ""]) => `<div class="kpi"><div class="label">${esc(label)}</div><div class="value ${cls}">${value}</div></div>`).join("");
}

function chart(id, config) {
  if (state.charts[id]) state.charts[id].destroy();
  const css = getComputedStyle(document.documentElement);
  Chart.defaults.color = css.getPropertyValue("--muted").trim();
  Chart.defaults.borderColor = css.getPropertyValue("--border").trim();
  state.charts[id] = new Chart($(id), config);
}

const timeAxis = { type: "linear", ticks: { maxTicksLimit: 8, callback: (v) => new Date(v).toLocaleDateString("zh-TW") } };

function lineChart(id, points, label, color = "#3b82f6") {
  chart(id, {
    type: "line",
    data: { datasets: [{ label, data: points, borderColor: color, borderWidth: 1.5, pointRadius: 0, fill: false }] },
    options: { animation: false, parsing: false, plugins: { legend: { display: false } }, scales: { x: timeAxis } },
  });
}

function parseJSON(text, field) {
  try { return JSON.parse(text || "{}"); } catch { throw new Error(`${field} 不是有效的 JSON`); }
}

// ---------- tabs ----------
document.querySelectorAll("#tabs button").forEach((b) =>
  b.addEventListener("click", () => {
    document.querySelectorAll("#tabs button").forEach((x) => x.classList.toggle("active", x === b));
    document.querySelectorAll(".tab").forEach((t) => t.classList.toggle("active", t.id === "tab-" + b.dataset.tab));
    loaders[b.dataset.tab]?.();
  })
);

// ---------- meta / selects ----------
async function loadMeta() {
  state.meta = await api("/api/meta");
  const exOpts = Object.entries(state.meta.exchanges).map(([k, v]) => `<option value="${k}">${v.label}</option>`).join("");
  document.querySelectorAll(".exchange-select").forEach((s) => { const v = s.value; s.innerHTML = exOpts; if (v) s.value = v; });
  const tfOpts = state.meta.timeframes.map((t) => `<option ${t === "1h" ? "selected" : ""}>${t}</option>`).join("");
  document.querySelectorAll(".tf-select").forEach((s) => { const v = s.value; s.innerHTML = tfOpts; if (v) s.value = v; });
  const anyAi = Object.values(state.meta.ai).some(Boolean);
  $("ai-badge").textContent = anyAi ? `AI 啟用 · ${state.meta.ai_model}` : "AI 關閉";
  $("ai-badge").classList.toggle("on", anyAi);
  document.querySelectorAll("button.ai").forEach((b) => (b.title = anyAi ? "" : "請先到「設定」啟用 Gemini"));
}

async function loadStrategies() {
  state.strategies = await api("/api/strategies");
  const opts = state.strategies.map((s) => `<option value="${s.id}">${esc(s.name)}</option>`).join("");
  document.querySelectorAll(".strategy-select").forEach((s) => { const v = s.value; s.innerHTML = opts; if (v) s.value = v; });
  $("strategy-list").innerHTML = state.strategies
    .map((s) => `<div class="item ${state.currentStrategy?.id === s.id ? "active" : ""}" data-id="${s.id}">${esc(s.name)}<small>${esc(s.description)}</small></div>`)
    .join("");
  $("strategy-list").querySelectorAll(".item").forEach((el) => el.addEventListener("click", () => openStrategy(+el.dataset.id)));
  if (!state.currentStrategy && state.strategies.length) openStrategy(state.strategies[0].id);
}

async function loadAccounts() {
  state.accounts = await api("/api/accounts");
  const opts = '<option value="">（無 / 僅模擬）</option>' + state.accounts.map((a) => `<option value="${a.id}">${esc(a.name)} · ${a.exchange}${a.testnet ? " (測試網)" : ""}</option>`).join("");
  document.querySelectorAll(".account-select").forEach((s) => { const v = s.value; s.innerHTML = opts; if (v) s.value = v; });
}

// ---------- dashboard ----------
async function loadDashboard() {
  const [ov, hist, bots, events] = await Promise.all([api("/api/portfolio/overview"), api("/api/portfolio/history?days=90"), api("/api/bots"), api("/api/portfolio/events?limit=10")]);
  const running = bots.filter((b) => b.status === "running");
  const first = hist.length ? hist[0].total_usd : null;
  const change = first ? (ov.total_usd / first - 1) * 100 : null;
  kpis($("dash-kpis"), [
    ["總資產 (USD)", usd(ov.total_usd)],
    ["90 天變化", change === null ? "-" : fmt(change) + "%", signCls(change)],
    ["交易所帳戶", ov.by_exchange.length],
    ["合約持倉", ov.positions.length],
    ["運行中機器人", running.length],
    ["策略數", state.strategies.length],
  ]);
  lineChart("dash-equity", hist.map((h) => ({ x: Date.parse(h.t), y: h.total_usd })), "總資產");
  $("dash-exchanges").innerHTML = table([["帳戶", (r) => esc(r.name)], ["交易所", (r) => r.exchange], ["資產 (USD)", (r) => usd(r.total_usd)], ["更新", (r) => dt(r.taken_at)]], ov.by_exchange);
  $("dash-bots").innerHTML = table(
    [["名稱", (r) => esc(r.name)], ["模式", (r) => `<span class="tag ${r.mode}">${r.mode === "live" ? "實盤" : "模擬"}</span>`], ["標的", (r) => `${r.exchange} ${esc(r.symbol)}`], ["權益估計", (r) => usd(r.equity_estimate)]],
    running
  );
  $("dash-events").innerHTML = eventsTable(events);
}

// ---------- strategies ----------
const TEMPLATE = `class MyStrategy(Strategy):
    """一句話描述策略"""

    params = {"fast": 10, "slow": 30}

    def init(self, data):
        self.fast = ta.ema(data.close, self.p.fast)
        self.slow = ta.ema(data.close, self.p.slow)

    def next(self, i, data):
        if ta.crossover(self.fast, self.slow, i):
            return 1.0
        if ta.crossunder(self.fast, self.slow, i):
            return 0.0
        return None
`;

async function openStrategy(id) {
  const s = await api(`/api/strategies/${id}`);
  state.currentStrategy = s;
  $("st-name").value = s.name;
  $("st-desc").value = s.description;
  $("st-code").value = s.code;
  $("bt-params").value = JSON.stringify(s.params);
  $("editor-title").textContent = `策略編輯器 — ${s.name}`;
  document.querySelectorAll("#strategy-list .item").forEach((el) => el.classList.toggle("active", +el.dataset.id === id));
  loadBacktestHistory();
}

$("btn-new-strategy").onclick = () => {
  state.currentStrategy = null;
  $("st-name").value = "my_strategy";
  $("st-desc").value = "";
  $("st-code").value = TEMPLATE;
  $("editor-title").textContent = "策略編輯器 — 新策略";
  document.querySelectorAll("#strategy-list .item").forEach((el) => el.classList.remove("active"));
};

$("st-code").addEventListener("keydown", (e) => {
  if (e.key === "Tab") {
    e.preventDefault();
    const t = e.target, s = t.selectionStart;
    t.value = t.value.slice(0, s) + "    " + t.value.slice(t.selectionEnd);
    t.selectionStart = t.selectionEnd = s + 4;
  }
});

$("btn-validate").onclick = (e) => busy(e.target, async () => {
  const r = await api("/api/strategies/validate", { method: "POST", body: { code: $("st-code").value } });
  toast("語法正確，參數: " + JSON.stringify(r.params));
});

$("btn-save-strategy").onclick = (e) => busy(e.target, async () => {
  const cur = state.currentStrategy;
  const body = { name: $("st-name").value.trim(), description: $("st-desc").value, code: $("st-code").value, params: cur ? cur.params : {} };
  const s = cur ? await api(`/api/strategies/${cur.id}`, { method: "PUT", body }) : await api("/api/strategies", { method: "POST", body });
  state.currentStrategy = s;
  toast("已儲存");
  await loadStrategies();
  openStrategy(s.id);
});

$("btn-delete-strategy").onclick = (e) => busy(e.target, async () => {
  const cur = state.currentStrategy;
  if (!cur || !confirm(`刪除策略「${cur.name}」及其回測紀錄？`)) return;
  await api(`/api/strategies/${cur.id}`, { method: "DELETE" });
  state.currentStrategy = null;
  await loadStrategies();
});

$("btn-ai-strategy").onclick = (e) => busy(e.target, async () => {
  const out = $("ai-strategy-out");
  out.classList.remove("hidden");
  out.innerHTML = "Gemini 思考中…";
  const r = await api("/api/strategies/ai-review", { method: "POST", body: { code: $("st-code").value } });
  out.innerHTML = md(r.review) + '<button class="secondary small" id="btn-apply-ai">套用 AI 提供的程式碼到編輯器</button>';
  $("btn-apply-ai").onclick = () => {
    const m = r.review.match(/```python\n([\s\S]*?)```/g);
    if (!m) return toast("回覆中沒有程式碼區塊", true);
    $("st-code").value = m[m.length - 1].replace(/```python\n|```/g, "");
    toast("已套用，請檢查後儲存並回測比較");
  };
});

// ---------- backtest ----------
$("bt-source").onchange = () => $("csv-box").classList.toggle("hidden", $("bt-source").value !== "csv");
(function initDates() {
  const end = new Date(), start = new Date();
  start.setFullYear(end.getFullYear() - 1);
  $("bt-end").value = end.toISOString().slice(0, 10);
  $("bt-start").value = start.toISOString().slice(0, 10);
})();

async function backtestBody() {
  if (!state.currentStrategy) throw new Error("請先選擇並儲存策略");
  const body = {
    strategy_id: state.currentStrategy.id,
    params: parseJSON($("bt-params").value, "參數覆寫"),
    source: $("bt-source").value,
    exchange: $("bt-exchange").value,
    symbol: $("bt-symbol").value.trim(),
    market: $("bt-market").value,
    timeframe: $("bt-timeframe").value,
    start: $("bt-start").value || null,
    end: $("bt-end").value || null,
    initial_capital: +$("bt-capital").value,
    fee_rate: +$("bt-fee").value,
    slippage: +$("bt-slippage").value,
    leverage: +$("bt-leverage").value,
    stop_loss: +$("bt-sl").value / 100,
    take_profit: +$("bt-tp").value / 100,
  };
  if (body.source === "csv") {
    const f = $("bt-csv-file").files[0];
    if (!f) throw new Error("請選擇 CSV 檔案");
    body.csv = await f.text();
  }
  return body;
}

$("btn-run-backtest").onclick = (e) => busy(e.target, async () => {
  const body = await backtestBody();
  body.ai = $("bt-ai").checked;
  const r = await api("/api/backtests", { method: "POST", body });
  showRun(r);
  if (r.ai_error) toast("AI 分析失敗: " + r.ai_error, true);
  loadBacktestHistory();
});

$("btn-optimize").onclick = (e) => busy(e.target, async () => {
  const body = await backtestBody();
  body.grid = parseJSON($("opt-grid").value, "參數網格");
  body.sort_by = $("opt-sort").value;
  const r = await api("/api/backtests/optimize", { method: "POST", body });
  $("opt-out").innerHTML = `<p class="muted">${r.results.length} 組參數 · ${r.bars} 根 K 線（點選列可套用參數）</p>` + table(
    [
      ["參數", (x) => esc(JSON.stringify(x.params))],
      ["總報酬%", (x) => `<span class="${signCls(x.metrics.total_return_pct)}">${fmt(x.metrics.total_return_pct)}</span>`],
      ["最大回撤%", (x) => fmt(x.metrics.max_drawdown_pct)],
      ["Sharpe", (x) => fmt(x.metrics.sharpe)],
      ["Calmar", (x) => fmt(x.metrics.calmar)],
      ["勝率%", (x) => fmt(x.metrics.win_rate_pct)],
      ["獲利因子", (x) => fmt(x.metrics.profit_factor)],
      ["交易數", (x) => x.metrics.num_closed_trades],
    ],
    r.results, true
  );
  bindRows($("opt-out"), r.results, (x) => { $("bt-params").value = JSON.stringify(x.params); toast("已套用參數，可按「執行回測」查看細節"); });
});

const METRIC_LABELS = [
  ["total_return_pct", "總報酬 %", true], ["buy_hold_return_pct", "買入持有 %", true], ["cagr_pct", "年化報酬 %", true],
  ["max_drawdown_pct", "最大回撤 %"], ["sharpe", "Sharpe"], ["sortino", "Sortino"], ["calmar", "Calmar"],
  ["win_rate_pct", "勝率 %"], ["profit_factor", "獲利因子"], ["num_closed_trades", "平倉次數"], ["total_fees", "總手續費"],
  ["exposure_pct", "持倉時間 %"], ["open_position_pnl", "未平倉損益", true], ["end_equity", "期末權益"],
];

function showRun(r) {
  state.currentRun = r;
  $("bt-result").classList.remove("hidden");
  kpis($("bt-metrics"), METRIC_LABELS.map(([k, label, colored]) => [label, fmt(r.metrics[k]), colored ? signCls(r.metrics[k]) : ""]));
  lineChart("bt-equity", r.equity_curve.map(([t, v]) => ({ x: t, y: v })), "權益", "#22c55e");
  const buys = r.trades.filter((t) => t.side === "buy").map((t) => ({ x: t.ts, y: t.price }));
  const sells = r.trades.filter((t) => t.side === "sell").map((t) => ({ x: t.ts, y: t.price }));
  const priceData = r.candles ? r.candles.map((c) => ({ x: c[0], y: c[4] })) : [];
  chart("bt-price", {
    type: "line",
    data: {
      datasets: [
        { label: "收盤價", data: priceData, borderColor: "#8b949e", borderWidth: 1, pointRadius: 0 },
        { type: "scatter", label: "買", data: buys, backgroundColor: "#22c55e", pointStyle: "triangle", pointRadius: 5 },
        { type: "scatter", label: "賣", data: sells, backgroundColor: "#ef4444", pointStyle: "triangle", rotation: 180, pointRadius: 5 },
      ],
    },
    options: { animation: false, parsing: false, scales: { x: timeAxis } },
  });
  $("bt-trades").innerHTML = table(
    [
      ["時間", (t) => dt(t.ts)], ["方向", (t) => (t.side === "buy" ? '<span class="pos">買</span>' : '<span class="neg">賣</span>')],
      ["數量", (t) => fmt(t.qty, 6)], ["價格", (t) => fmt(t.price, 4)], ["手續費", (t) => fmt(t.fee)],
      ["已實現損益", (t) => (t.closed_qty ? `<span class="${signCls(t.pnl)}">${fmt(t.pnl)}</span>` : "-")],
      ["原因", (t) => ({ signal: "訊號", stop_loss: "停損", take_profit: "停利" }[t.reason] || t.reason)],
    ],
    r.trades.slice().reverse()
  );
  $("bt-ai-out").innerHTML = r.ai_review ? md(r.ai_review) : '<span class="muted">尚未分析</span>';
}

async function loadBacktestHistory() {
  const sid = state.currentStrategy?.id;
  const runs = await api("/api/backtests" + (sid ? `?strategy_id=${sid}` : ""));
  $("bt-history").innerHTML = table(
    [
      ["時間", (r) => dt(r.created_at)], ["標的", (r) => `${r.config.source === "exchange" ? r.config.exchange + " " + esc(r.config.symbol) : r.config.source} ${r.config.timeframe}`],
      ["參數", (r) => esc(JSON.stringify(r.config.params))], ["總報酬%", (r) => `<span class="${signCls(r.metrics.total_return_pct)}">${fmt(r.metrics.total_return_pct)}</span>`],
      ["回撤%", (r) => fmt(r.metrics.max_drawdown_pct)], ["Sharpe", (r) => fmt(r.metrics.sharpe)], ["AI", (r) => (r.ai_review ? "✨" : "")],
    ],
    runs, true
  );
  bindRows($("bt-history"), runs, async (r) => showRun(await api(`/api/backtests/${r.id}`)));
}

$("btn-ai-backtest").onclick = (e) => busy(e.target, async () => {
  if (!state.currentRun) return;
  $("bt-ai-out").innerHTML = "Gemini 分析中…";
  const r = await api(`/api/backtests/${state.currentRun.id}/ai-analyze`, { method: "POST" });
  $("bt-ai-out").innerHTML = md(r.review);
});

// ---------- bots ----------
function botForm(b) {
  $("bot-id").value = b?.id ?? "";
  $("bot-form-title").textContent = b ? `編輯機器人 #${b.id}` : "新增機器人";
  if (!b) return;
  $("bot-name").value = b.name; $("bot-strategy").value = b.strategy_id; $("bot-mode").value = b.mode;
  $("bot-exchange").value = b.exchange; $("bot-account").value = b.account_id ?? ""; $("bot-symbol").value = b.symbol;
  $("bot-market").value = b.market; $("bot-timeframe").value = b.timeframe; $("bot-capital").value = b.capital;
  $("bot-leverage").value = b.leverage; $("bot-maxdd").value = b.max_drawdown_stop; $("bot-params").value = JSON.stringify(b.params);
  $("bot-ai-guard").checked = b.ai_guard;
}

$("btn-clear-bot").onclick = () => botForm(null);
$("btn-save-bot").onclick = (e) => busy(e.target, async () => {
  const body = {
    name: $("bot-name").value, strategy_id: +$("bot-strategy").value, mode: $("bot-mode").value, exchange: $("bot-exchange").value,
    account_id: $("bot-account").value ? +$("bot-account").value : null, symbol: $("bot-symbol").value.trim(), market: $("bot-market").value,
    timeframe: $("bot-timeframe").value, capital: +$("bot-capital").value, leverage: +$("bot-leverage").value,
    max_drawdown_stop: +$("bot-maxdd").value, params: parseJSON($("bot-params").value, "參數覆寫"), ai_guard: $("bot-ai-guard").checked,
  };
  if (body.mode === "live" && !confirm("實盤模式會使用真實資金下單，確定要儲存？")) return;
  const id = $("bot-id").value;
  await api(id ? `/api/bots/${id}` : "/api/bots", { method: id ? "PUT" : "POST", body });
  toast("已儲存");
  botForm(null);
  loadBots();
});

async function loadBots() {
  state.bots = await api("/api/bots");
  $("bot-list").innerHTML = state.bots.length ? state.bots.map((b) => {
    const s = b.state || {};
    const pnl = b.equity_estimate !== null ? b.equity_estimate - b.capital : null;
    return `<div class="bot-card ${state.selectedBot === b.id ? "selected" : ""}" data-id="${b.id}">
      <div class="row between">
        <div><b>${esc(b.name)}</b> <span class="tag ${b.mode}">${b.mode === "live" ? "實盤" : "模擬"}</span> <span class="status ${b.status}">${{ running: "運行中", stopped: "已停止", error: "錯誤停止" }[b.status] || b.status}</span>${b.ai_guard ? ' <span class="tag">AI 守門</span>' : ""}</div>
        <div class="row">
          ${b.status === "running" ? `<button class="small danger" data-act="stop">停止</button>` : `<button class="small" data-act="start">啟動</button>`}
          <button class="small secondary" data-act="edit">編輯</button>
          <button class="small secondary" data-act="reset">重設帳務</button>
          <button class="small secondary" data-act="delete">刪除</button>
        </div>
      </div>
      <div class="meta">${esc(b.strategy_name)} · ${b.exchange} ${esc(b.symbol)} ${b.market === "swap" ? "永續 " + b.leverage + "x" : "現貨"} · ${b.timeframe} · 資金 ${usd(b.capital)}</div>
      <div class="meta">持倉 ${fmt(s.qty, 6)} @ ${fmt(s.entry, 4)} · 權益估計 ${usd(b.equity_estimate)} <span class="${signCls(pnl)}">(${pnl === null ? "-" : fmt(pnl)})</span> · 最後K線 ${s.last_bar_ts ? dt(s.last_bar_ts) : "-"}</div>
      ${b.last_error ? `<div class="meta neg">⚠ ${esc(b.last_error)}</div>` : ""}
    </div>`;
  }).join("") : '<div class="card muted">尚無機器人。先在「策略回測」驗證策略，再於左側建立。</div>';
  $("bot-list").querySelectorAll(".bot-card").forEach((card) => {
    const id = +card.dataset.id;
    card.addEventListener("click", (ev) => {
      const act = ev.target.dataset?.act;
      if (act) return botAction(id, act, ev.target);
      selectBot(id);
    });
  });
  if (state.selectedBot) loadBotDetail();
}

async function botAction(id, act, btn) {
  const bot = state.bots.find((b) => b.id === id);
  if (act === "edit") return botForm(bot);
  if (act === "delete" && !confirm(`刪除機器人「${bot.name}」？（不會自動平倉）`)) return;
  if (act === "start" && bot.mode === "live" && !confirm(`啟動實盤機器人「${bot.name}」，將以真實資金下單。確定？`)) return;
  await busy(btn, async () => {
    if (act === "delete") await api(`/api/bots/${id}`, { method: "DELETE" });
    else await api(`/api/bots/${id}/${act}`, { method: "POST" });
  });
  selectBot(id);
  loadBots();
}

function selectBot(id) {
  state.selectedBot = id;
  document.querySelectorAll(".bot-card").forEach((c) => c.classList.toggle("selected", +c.dataset.id === id));
  loadBotDetail();
}

async function loadBotDetail() {
  const id = state.selectedBot;
  if (!id) return;
  const bot = state.bots.find((b) => b.id === id);
  $("bot-log-title").textContent = `機器人日誌 — ${bot ? bot.name : id}`;
  const [logs, trades] = await Promise.all([api(`/api/bots/${id}/logs`), api(`/api/trades?bot_id=${id}`)]);
  $("bot-logs").innerHTML = logs.map((l) => `<div class="${l.level}">${dt(l.created_at)}  ${esc(l.message)}</div>`).join("") || '<p class="muted">無日誌</p>';
  $("bot-trades").innerHTML = tradesTable(trades);
}

function tradesTable(trades) {
  return table(
    [
      ["時間", (t) => dt(t.created_at)], ["模式", (t) => `<span class="tag ${t.mode}">${t.mode}</span>`], ["標的", (t) => `${t.exchange} ${esc(t.symbol)}`],
      ["方向", (t) => (t.side === "buy" ? '<span class="pos">買</span>' : '<span class="neg">賣</span>')], ["數量", (t) => fmt(t.qty, 6)],
      ["價格", (t) => fmt(t.price, 4)], ["手續費(估)", (t) => fmt(t.fee)], ["訂單", (t) => esc(t.order_id)],
    ],
    trades
  );
}

$("btn-ai-bot").onclick = (e) => busy(e.target, async () => {
  if (!state.selectedBot) throw new Error("請先點選一個機器人");
  const out = $("bot-ai-out");
  out.classList.remove("hidden");
  out.innerHTML = "Gemini 分析中…";
  out.innerHTML = md((await api(`/api/bots/${state.selectedBot}/ai-review`, { method: "POST" })).review);
});

$("btn-order").onclick = (e) => busy(e.target, async () => {
  const body = {
    account_id: +$("ord-account").value, symbol: $("ord-symbol").value.trim(), market: $("ord-market").value, side: $("ord-side").value,
    order_type: $("ord-type").value, qty: +$("ord-qty").value, price: $("ord-price").value ? +$("ord-price").value : null, reduce_only: $("ord-reduce").checked,
  };
  if (!body.account_id) throw new Error("請選擇 API 帳戶");
  if (!confirm(`確認送出真實訂單：${body.side.toUpperCase()} ${body.qty} ${body.symbol} (${body.market}, ${body.order_type})？`)) return;
  const r = await api("/api/orders", { method: "POST", body });
  toast(`下單成功，訂單編號 ${r.order_id}`);
});

// ---------- portfolio ----------
function eventsTable(events) {
  const KIND = { open: "開倉", close: "平倉", increase: "加倉", reduce: "減倉", balance: "餘額變動" };
  return table(
    [
      ["時間", (e) => dt(e.created_at)], ["帳戶", (e) => esc(e.account)], ["類型", (e) => KIND[e.kind] || e.kind],
      ["標的", (e) => `${esc(e.symbol)} ${esc(e.side)}`], ["變動", (e) => `${fmt(e.size_before, 6)} → <b>${fmt(e.size_after, 6)}</b>`],
      ["價格", (e) => (e.price ? fmt(e.price, 4) : "-")], ["備註", (e) => esc(e.detail)],
    ],
    events
  );
}

async function loadPortfolio() {
  const [ov, hist, events] = await Promise.all([api("/api/portfolio/overview"), api("/api/portfolio/history?days=180"), api("/api/portfolio/events")]);
  const upnl = ov.positions.reduce((a, p) => a + p.unrealized_pnl, 0);
  const stable = ov.assets.filter((a) => ["USDT", "USDC", "FDUSD", "DAI", "USD"].includes(a.asset)).reduce((a, x) => a + x.usd, 0);
  kpis($("pf-kpis"), [
    ["總資產 (USD)", usd(ov.total_usd)],
    ["穩定幣占比", ov.total_usd ? fmt((stable / ov.total_usd) * 100) + "%" : "-"],
    ["未實現損益", usd(upnl), signCls(upnl)],
    ["幣種數", ov.assets.length],
    ["合約持倉", ov.positions.length],
    ...ov.by_exchange.map((x) => [x.name, usd(x.total_usd)]),
  ]);
  lineChart("pf-equity", hist.map((h) => ({ x: Date.parse(h.t), y: h.total_usd })), "總資產");
  const top = ov.assets.slice(0, 10);
  const other = ov.assets.slice(10).reduce((a, x) => a + x.usd, 0);
  chart("pf-alloc", {
    type: "doughnut",
    data: {
      labels: [...top.map((a) => a.asset), ...(other > 0 ? ["其他"] : [])],
      datasets: [{ data: [...top.map((a) => a.usd), ...(other > 0 ? [other] : [])], backgroundColor: ["#3b82f6", "#22c55e", "#f59e0b", "#a855f7", "#ef4444", "#14b8a6", "#ec4899", "#84cc16", "#6366f1", "#f97316", "#64748b"] }],
    },
    options: { animation: false, plugins: { legend: { position: "right" } } },
  });
  $("pf-assets").innerHTML = table(
    [
      ["幣種", (a) => `<b>${esc(a.asset)}</b>`], ["數量", (a) => fmt(a.total, 8)], ["價值 (USD)", (a) => usd(a.usd)], ["占比", (a) => fmt(a.pct) + "%"],
      ["分佈", (a) => Object.entries(a.exchanges).map(([k, v]) => `${esc(k)}: ${fmt(v, 6)}`).join(" · ")],
    ],
    ov.assets
  );
  $("pf-positions").innerHTML = table(
    [
      ["帳戶", (p) => esc(p.account)], ["標的", (p) => esc(p.symbol)], ["方向", (p) => (p.side === "long" ? '<span class="pos">多</span>' : '<span class="neg">空</span>')],
      ["數量", (p) => fmt(p.size, 6)], ["開倉均價", (p) => fmt(p.entry_price, 4)], ["標記價格", (p) => fmt(p.mark_price, 4)],
      ["未實現損益", (p) => `<span class="${signCls(p.unrealized_pnl)}">${fmt(p.unrealized_pnl)}</span>`], ["槓桿", (p) => fmt(p.leverage) + "x"],
    ],
    ov.positions
  );
  $("pf-events").innerHTML = eventsTable(events);
}

$("btn-snapshot").onclick = (e) => busy(e.target, async () => {
  const r = await api("/api/portfolio/snapshot", { method: "POST" });
  const errs = Object.entries(r).filter(([, v]) => !v.ok);
  $("snap-status").textContent = `已同步 ${Object.keys(r).length - errs.length}/${Object.keys(r).length} 個帳戶 · ${new Date().toLocaleTimeString("zh-TW")}`;
  errs.forEach(([k, v]) => toast(`${k}: ${v.error}`, true));
  loadPortfolio();
});

$("btn-ai-portfolio").onclick = (e) => busy(e.target, async () => {
  const out = $("pf-ai-out");
  out.classList.remove("hidden");
  out.innerHTML = "Gemini 分析中…";
  out.innerHTML = md((await api("/api/portfolio/ai-analyze", { method: "POST" })).review);
});

// ---------- accounts ----------
function accForm(a) {
  $("acc-id").value = a?.id ?? "";
  $("acc-form-title").textContent = a ? `編輯 ${a.name}` : "新增 API 帳戶";
  $("acc-name").value = a?.name ?? "";
  if (a) $("acc-exchange").value = a.exchange;
  $("acc-exchange").disabled = !!a;
  $("acc-key").value = a?.api_key ?? "";
  $("acc-secret").value = "";
  $("acc-pass").value = "";
  $("acc-testnet").checked = a?.testnet ?? false;
  $("acc-enabled").checked = a?.enabled ?? true;
  updateAccHint();
}
function updateAccHint() {
  const info = state.meta?.exchanges[$("acc-exchange").value];
  if (!info) return;
  $("acc-api-version").textContent = `使用 API：${info.api}${info.swap ? "" : "（僅現貨）"}`;
  $("acc-pass-wrap").classList.toggle("hidden", !info.passphrase);
}
$("acc-exchange").onchange = updateAccHint;
$("btn-clear-acc").onclick = () => accForm(null);
$("btn-save-acc").onclick = (e) => busy(e.target, async () => {
  const id = $("acc-id").value;
  const body = {
    name: $("acc-name").value.trim(), exchange: $("acc-exchange").value, api_key: $("acc-key").value.trim(), api_secret: $("acc-secret").value.trim(),
    passphrase: $("acc-pass").value, testnet: $("acc-testnet").checked, enabled: $("acc-enabled").checked,
  };
  if (!body.name) throw new Error("請輸入名稱");
  await api(id ? `/api/accounts/${id}` : "/api/accounts", { method: id ? "PUT" : "POST", body });
  toast("已儲存");
  accForm(null);
  loadAccountsTab();
});

async function loadAccountsTab() {
  await loadAccounts();
  $("acc-list").innerHTML = table(
    [
      ["名稱", (a) => esc(a.name)], ["交易所", (a) => a.exchange], ["API Key", (a) => esc(a.api_key)],
      ["環境", (a) => (a.testnet ? '<span class="tag paper">測試網</span>' : '<span class="tag live">正式</span>')], ["啟用", (a) => (a.enabled ? "✓" : "")],
      ["", (a) => `<button class="small secondary" data-act="test" data-id="${a.id}">測試連線</button> <button class="small secondary" data-act="edit" data-id="${a.id}">編輯</button> <button class="small danger" data-act="del" data-id="${a.id}">刪除</button>`],
    ],
    state.accounts
  );
  $("acc-list").querySelectorAll("button[data-act]").forEach((b) => (b.onclick = () => busy(b, async () => {
    const a = state.accounts.find((x) => x.id === +b.dataset.id);
    if (b.dataset.act === "edit") return accForm(a);
    if (b.dataset.act === "del") {
      if (!confirm(`刪除帳戶「${a.name}」及其快照紀錄？`)) return;
      await api(`/api/accounts/${a.id}`, { method: "DELETE" });
      return loadAccountsTab();
    }
    const r = await api(`/api/accounts/${a.id}/test`, { method: "POST" });
    toast(`${a.name} 連線成功：${r.balances.length} 個幣種有餘額`);
  })));
}

// ---------- settings ----------
async function loadSettings() {
  const s = await api("/api/settings");
  $("set-ai-enabled").checked = s.ai_enabled; $("set-ai-backtest").checked = s.ai_backtest;
  $("set-ai-trading").checked = s.ai_trading; $("set-ai-portfolio").checked = s.ai_portfolio;
  $("set-key").value = s.gemini_api_key; $("set-model").value = s.gemini_model; $("set-lang").value = s.ai_language;
}
$("btn-save-settings").onclick = (e) => busy(e.target, async () => {
  await api("/api/settings", {
    method: "PUT",
    body: {
      ai_enabled: $("set-ai-enabled").checked, ai_backtest: $("set-ai-backtest").checked, ai_trading: $("set-ai-trading").checked,
      ai_portfolio: $("set-ai-portfolio").checked, gemini_api_key: $("set-key").value.trim(), gemini_model: $("set-model").value.trim(), ai_language: $("set-lang").value.trim(),
    },
  });
  toast("設定已儲存");
  await loadMeta();
  loadSettings();
});
$("btn-test-ai").onclick = (e) => busy(e.target, async () => {
  const r = await api("/api/ai/test", { method: "POST" });
  $("set-msg").textContent = "Gemini 回覆：" + r.reply;
});

// ---------- boot ----------
const loaders = {
  dashboard: loadDashboard,
  backtest: () => { loadStrategies(); loadBacktestHistory(); },
  bots: () => { loadStrategies(); loadAccounts(); loadBots(); },
  portfolio: loadPortfolio,
  accounts: loadAccountsTab,
  settings: loadSettings,
};

(async function boot() {
  try {
    await loadMeta();
    await Promise.all([loadStrategies(), loadAccounts()]);
    accForm(null);
    loadDashboard();
  } catch (e) {
    toast("初始化失敗：" + e.message, true);
  }
  setInterval(() => {
    if ($("tab-bots").classList.contains("active")) loadBots();
  }, 15000);
})();
