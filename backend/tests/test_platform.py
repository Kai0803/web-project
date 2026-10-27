"""Bot runner, portfolio diffs, Gemini client and API flows."""
import json
from datetime import datetime, timezone

import httpx
import pytest
from fastapi.testclient import TestClient

from app.ai.gemini import GeminiClient
from app.db import SessionLocal, init_db
from app.exchanges import Balance, Candle, ExchangeClient, ExchangeError, OrderResult, Position
from app.models import BalanceSnapshot, Bot, ExchangeAccount, Strategy, TradeRecord
from app.portfolio.service import diff_snapshots, snapshot_account
from app.trading.runner import init_state, step

HOUR = 3_600_000


class FakeExchange(ExchangeClient):
    name = "fake"
    base_url = "https://fake.invalid"

    def __init__(self, closes, **kw):
        super().__init__(**kw)
        self.closes = closes
        self.orders = []
        self.balances = [Balance("USDT", 1000.0), Balance("BTC", 0.5, 0.0)]
        self.positions = []

    async def _fetch_candles_page(self, symbol, market, timeframe, start_ms, end_ms, limit):
        return [Candle(i * HOUR, c, c, c, c, 1) for i, c in enumerate(self.closes) if i * HOUR <= end_ms]

    async def fetch_price(self, symbol, market="spot"):
        if symbol.startswith("BTC"):
            return self.closes[-1]
        raise ExchangeError("no price")

    async def fetch_balances(self):
        return self.balances

    async def fetch_positions(self):
        return self.positions

    async def place_order(self, symbol, market, side, qty, order_type="market", price=None, reduce_only=False):
        self.orders.append((side, qty, reduce_only))
        return OrderResult("x1", symbol, side, qty, self.closes[-1])


CROSS = """
class Cross(Strategy):
    params = {"n": 3}
    lookback = 10
    def init(self, data):
        self.ma = ta.sma(data.close, self.p.n)
    def next(self, i, data):
        if self.ma[i] is None:
            return None
        return 1.0 if data.close[i] > self.ma[i] else 0.0
"""


@pytest.fixture()
def db():
    init_db()
    with SessionLocal() as s:
        yield s


def make_bot(db, mode="paper", market="spot"):
    st = Strategy(name=f"cross-{datetime.now().timestamp()}", code=CROSS, params={"n": 3})
    db.add(st)
    db.commit()
    bot = Bot(name="b", strategy_id=st.id, exchange="binance", symbol="BTC/USDT", market=market, timeframe="1h", mode=mode, capital=1000, leverage=1, max_drawdown_stop=0.5)
    bot.state = init_state(bot)
    db.add(bot)
    db.commit()
    return bot


async def test_paper_bot_enters_and_exits(db):
    bot = make_bot(db)
    up = FakeExchange([10, 10, 10, 11, 12])
    now = 6 * HOUR  # all 5 bars closed
    assert await step(bot, db, up, now_ms=now) == "buy"
    assert bot.state["qty"] == pytest.approx(1000 / (12 * 1.0005) / 1.001, rel=1e-2)
    assert await step(bot, db, up, now_ms=now) == "no-new-bar"
    down = FakeExchange([10, 10, 10, 11, 12, 9])
    assert await step(bot, db, down, now_ms=7 * HOUR) == "sell"
    assert bot.state["qty"] == 0
    trades = db.query(TradeRecord).filter(TradeRecord.bot_id == bot.id).all()
    assert [t.side for t in trades] == ["buy", "sell"] and all(t.mode == "paper" for t in trades)
    assert not up.orders  # paper never touches the exchange


async def test_live_bot_places_orders_and_unclosed_bar_ignored(db):
    bot = make_bot(db, mode="live")
    ex = FakeExchange([10, 10, 10, 11, 12])
    # bar 4 (ts=4h) is still open at 4.5h, so the last closed bar is 3 -> close 11 > sma(10,10,11) -> buy
    assert await step(bot, db, ex, now_ms=int(4.5 * HOUR)) == "buy"
    assert ex.orders and ex.orders[0][0] == "buy"
    assert bot.state["last_bar_ts"] == 3 * HOUR


async def test_drawdown_stop(db):
    bot = make_bot(db)
    bot.state = {**init_state(bot), "cash": 0.0, "qty": 100.0, "entry": 10.0}
    db.commit()
    ex = FakeExchange([5, 5, 5, 5, 4])  # equity 400 < 1000 * 0.5
    await step(bot, db, ex, now_ms=6 * HOUR)
    assert bot.status == "stopped" and bot.state["qty"] == 0


def test_diff_snapshots_detects_changes():
    t = datetime.now(timezone.utc)
    prev = BalanceSnapshot(account_id=1, taken_at=t, balances=[{"asset": "USDT", "wallet": "spot", "total": 1000, "usd": 1000}],
                           positions=[{"symbol": "BTC/USDT", "side": "long", "size": 1.0, "mark_price": 1, "unrealized_pnl": 5}, {"symbol": "ETH/USDT", "side": "short", "size": 2.0, "mark_price": 1, "unrealized_pnl": 0}])
    cur = BalanceSnapshot(account_id=1, taken_at=t, balances=[{"asset": "USDT", "wallet": "spot", "total": 1500, "usd": 1500}],
                          positions=[{"symbol": "BTC/USDT", "side": "long", "size": 0.4, "mark_price": 1, "unrealized_pnl": 1}, {"symbol": "SOL/USDT", "side": "long", "size": 3.0, "mark_price": 1, "unrealized_pnl": 0}])
    kinds = sorted((e.kind, e.symbol) for e in diff_snapshots(prev, cur))
    assert kinds == [("balance", "USDT"), ("close", "ETH/USDT"), ("open", "SOL/USDT"), ("reduce", "BTC/USDT")]
    assert diff_snapshots(None, cur) == []


async def test_snapshot_account_values_balances(db):
    acc = ExchangeAccount(name="fake", exchange="binance")
    db.add(acc)
    db.commit()
    ex = FakeExchange([100])
    ex.positions = [Position("BTC/USDT", "long", 1, 90, 100, 10, 2)]
    snap = await snapshot_account(db, acc, ex)
    assert snap.total_usd == pytest.approx(1000 + 0.5 * 100 + 10)
    ex.balances = [Balance("USDT", 2000.0)]
    ex.positions = []
    await snapshot_account(db, acc, ex)
    from app.models import PositionEvent

    kinds = {e.kind for e in db.query(PositionEvent).filter(PositionEvent.account_id == acc.id)}
    assert kinds == {"balance", "close"}


async def test_gemini_client_request_shape():
    seen = {}

    def handler(req):
        seen["url"] = str(req.url)
        seen["key"] = req.headers["x-goog-api-key"]
        seen["body"] = json.loads(req.content)
        return httpx.Response(200, json={"candidates": [{"content": {"parts": [{"text": "{\"approve\": false, \"reason\": \"太危險\"}"}]}}]})

    client = GeminiClient("gk", "gemini-flash-latest", transport=httpx.MockTransport(handler))
    out = await client.generate_json("hi")
    assert out == {"approve": False, "reason": "太危險"}
    assert seen["url"].endswith("/v1beta/models/gemini-flash-latest:generateContent")
    assert seen["key"] == "gk"
    assert seen["body"]["generationConfig"]["responseMimeType"] == "application/json"
    assert "繁體中文" in seen["body"]["systemInstruction"]["parts"][0]["text"]


def test_api_end_to_end():
    from app.main import app

    with TestClient(app) as c:
        # settings: AI disabled by default; key is masked
        assert c.get("/api/meta").json()["ai"] == {"backtest": False, "trading": False, "portfolio": False}
        r = c.put("/api/settings", json={"ai_enabled": True, "gemini_api_key": "AIzaSyTESTKEY123456"}).json()
        assert r["ai_enabled"] is True and r["gemini_api_key"] == "AIza…3456"
        assert c.get("/api/meta").json()["ai"]["backtest"] is True
        c.put("/api/settings", json={"ai_enabled": False, "gemini_api_key": "AIza…3456"})
        assert c.post("/api/strategies/ai-review", json={"code": CROSS}).status_code == 400

        # accounts: secrets encrypted and masked
        acc = c.post("/api/accounts", json={"name": "okx main", "exchange": "okx", "api_key": "abcdefgh1234", "api_secret": "s", "passphrase": "p"}).json()
        assert acc["api_key"] == "abcd…1234" and acc["has_passphrase"]
        assert c.post("/api/accounts", json={"name": "x", "exchange": "okx", "api_key": "k", "api_secret": "s"}).status_code == 400
        with SessionLocal() as s:
            row = s.get(ExchangeAccount, acc["id"])
            assert "abcdefgh1234" not in row.api_key_enc

        # strategies + backtest + optimize
        st = c.post("/api/strategies", json={"name": "api-cross", "code": CROSS}).json()
        assert st["params"] == {"n": 3}
        assert c.post("/api/strategies", json={"name": "bad", "code": "nope("}).status_code == 400
        bt = c.post("/api/backtests", json={"strategy_id": st["id"], "source": "synthetic"}).json()
        assert bt["metrics"]["bars"] == 3000 and bt["candles"]
        assert c.get(f"/api/backtests/{bt['id']}").json()["id"] == bt["id"]
        csv = "time,open,high,low,close\n" + "\n".join(f"{1704067200 + i * 3600},{100 + i},{101 + i},{99 + i},{100 + i}" for i in range(50))
        assert c.post("/api/backtests", json={"strategy_id": st["id"], "source": "csv", "csv": csv}).status_code == 200
        opt = c.post("/api/backtests/optimize", json={"strategy_id": st["id"], "source": "synthetic", "grid": {"n": [3, 5, 8]}}).json()
        assert len(opt["results"]) == 3

        # bots: live requires a matching account
        bad = c.post("/api/bots", json={"name": "b", "strategy_id": st["id"], "mode": "live", "exchange": "binance"})
        assert bad.status_code == 400
        assert c.post("/api/bots", json={"name": "b", "strategy_id": st["id"], "exchange": "pionex", "market": "swap"}).status_code == 400
        bot = c.post("/api/bots", json={"name": "paper", "strategy_id": st["id"], "exchange": "okx", "symbol": "BTC/USDT"}).json()
        assert bot["status"] == "stopped" and bot["state"]["cash"] == 1000
        assert c.post(f"/api/bots/{bot['id']}/reset").status_code == 200
        assert c.delete(f"/api/bots/{bot['id']}").json()["ok"]

        assert c.get("/api/portfolio/overview").json()["total_usd"] >= 0
        assert c.get("/").status_code == 200
