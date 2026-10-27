"""Connector tests against mocked HTTP: request shape, signing and response parsing."""
import base64
import hashlib
import hmac
import json
from urllib.parse import parse_qsl, urlsplit

import httpx
import pytest

from app.exchanges import BinanceClient, ExchangeError
from app.exchanges.bitget import BitgetClient
from app.exchanges.bybit import BybitClient
from app.exchanges.okx import OKXClient
from app.exchanges.pionex import PionexClient


class Recorder:
    """httpx MockTransport that routes by path and records requests."""

    def __init__(self, routes):
        self.routes = routes
        self.requests: list[httpx.Request] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        for (method, path), payload in self.routes.items():
            if request.method == method and request.url.path == path:
                return httpx.Response(200, json=payload(request) if callable(payload) else payload)
        return httpx.Response(404, json={"msg": f"no route {request.method} {request.url.path}"})

    @property
    def transport(self):
        return httpx.MockTransport(self)


def test_binance_signature_matches_official_example():
    # Example from Binance docs (SIGNED endpoint security).
    c = BinanceClient(api_secret="NhqPtmdSJYdKjVHjA7PZj4Mge3R5YNiP1e3UZjInClVN65XAbvqqM6A7H5fATj0j")
    params = {"symbol": "LTCBTC", "side": "BUY", "type": "LIMIT", "timeInForce": "GTC", "quantity": "1", "price": "0.1", "recvWindow": "5000", "timestamp": "1499827319559"}
    assert c.sign(params).endswith("signature=c8db56825ae71d6d79447849e617115f4a920fa2acdcab2b053c4b2838bd6b71")


async def test_binance_klines_balances_positions_and_order():
    def spot_order(req):
        q = dict(parse_qsl(req.url.query.decode()))
        assert q["side"] == "BUY" and q["type"] == "MARKET" and q["quantity"] == "0.01"
        assert req.headers["X-MBX-APIKEY"] == "k"
        query = req.url.query.decode()
        body, sig = query.rsplit("&signature=", 1)
        assert sig == hmac.new(b"s", body.encode(), hashlib.sha256).hexdigest()
        return {"orderId": 7, "status": "FILLED", "executedQty": "0.01", "cummulativeQuoteQty": "650"}

    rec = Recorder(
        {
            ("GET", "/api/v3/klines"): [[1000, "1", "2", "0.5", "1.5", "10", 1999], [2000, "1.5", "2", "1", "1.8", "5", 2999]],
            ("GET", "/api/v3/account"): {"balances": [{"asset": "BTC", "free": "0.5", "locked": "0.1"}, {"asset": "ETH", "free": "0", "locked": "0"}]},
            ("GET", "/fapi/v3/account"): {"assets": [{"asset": "USDT", "walletBalance": "100", "availableBalance": "80"}]},
            ("GET", "/fapi/v3/positionRisk"): [
                {"symbol": "BTCUSDT", "positionAmt": "-0.2", "entryPrice": "60000", "markPrice": "59000", "unRealizedProfit": "200", "notional": "-11800", "initialMargin": "1180"}
            ],
            ("POST", "/api/v3/order"): spot_order,
        }
    )
    async with BinanceClient(api_key="k", api_secret="s", transport=rec.transport) as c:
        candles = await c.fetch_candles("BTC/USDT", "spot", "1h", limit=2, end_ms=5000)
        assert [x.close for x in candles] == [1.5, 1.8]
        bals = await c.fetch_balances()
        assert [(b.asset, b.total, b.wallet) for b in bals] == [("BTC", 0.6, "spot"), ("USDT", 100.0, "futures")]
        pos = await c.fetch_positions()
        assert pos[0].symbol == "BTC/USDT" and pos[0].side == "short" and pos[0].size == 0.2 and pos[0].leverage == 10
        res = await c.place_order("BTC/USDT", "spot", "buy", 0.01)
        assert res.order_id == "7" and res.price == 65000


async def test_bybit_signing_and_parsing():
    def create(req):
        body = req.content.decode()
        ts = req.headers["X-BAPI-TIMESTAMP"]
        expected = hmac.new(b"sec", (ts + "key" + "5000" + body).encode(), hashlib.sha256).hexdigest()
        assert req.headers["X-BAPI-SIGN"] == expected
        data = json.loads(body)
        assert data == {"category": "linear", "symbol": "ETHUSDT", "side": "Sell", "orderType": "Market", "qty": "1.5", "reduceOnly": True}
        return {"retCode": 0, "result": {"orderId": "abc"}}

    def wallet(req):
        ts = req.headers["X-BAPI-TIMESTAMP"]
        expected = hmac.new(b"sec", (ts + "key" + "5000" + "accountType=UNIFIED").encode(), hashlib.sha256).hexdigest()
        assert req.headers["X-BAPI-SIGN"] == expected
        return {"retCode": 0, "result": {"list": [{"coin": [{"coin": "USDT", "walletBalance": "500", "locked": "50", "usdValue": "500"}]}]}}

    rec = Recorder(
        {
            ("GET", "/v5/market/kline"): {"retCode": 0, "result": {"list": [["2000", "2", "3", "1", "2.5", "9", "0"], ["1000", "1", "2", "1", "2", "8", "0"]]}},
            ("GET", "/v5/account/wallet-balance"): wallet,
            ("GET", "/v5/position/list"): {"retCode": 0, "result": {"list": [{"symbol": "ETHUSDT", "side": "Buy", "size": "1.5", "avgPrice": "3000", "markPrice": "3100", "unrealisedPnl": "150", "leverage": "3"}]}},
            ("POST", "/v5/order/create"): create,
        }
    )
    async with BybitClient(api_key="key", api_secret="sec", transport=rec.transport) as c:
        candles = await c.fetch_candles("BTC/USDT", "swap", "1h", limit=2, end_ms=5000)
        assert [x.ts for x in candles] == [1000, 2000]
        assert "category=linear" in str(rec.requests[0].url) and "interval=60" in str(rec.requests[0].url)
        bals = await c.fetch_balances()
        assert bals[0].free == 450 and bals[0].usd == 500
        pos = await c.fetch_positions()
        assert pos[0].symbol == "ETH/USDT" and pos[0].side == "long" and pos[0].leverage == 3
        res = await c.place_order("ETH/USDT", "swap", "sell", 1.5, reduce_only=True)
        assert res.order_id == "abc"


async def test_bybit_error_raises():
    rec = Recorder({("GET", "/v5/market/tickers"): {"retCode": 10001, "retMsg": "params error"}})
    async with BybitClient(transport=rec.transport) as c:
        with pytest.raises(ExchangeError, match="10001"):
            await c.fetch_price("BTC/USDT")


async def test_okx_signing_contracts_and_demo_header():
    def order(req):
        ts = req.headers["OK-ACCESS-TIMESTAMP"]
        body = req.content.decode()
        expected = base64.b64encode(hmac.new(b"sec", (ts + "POST" + "/api/v5/trade/order" + body).encode(), hashlib.sha256).digest()).decode()
        assert req.headers["OK-ACCESS-SIGN"] == expected
        assert req.headers["OK-ACCESS-PASSPHRASE"] == "pp"
        assert req.headers["x-simulated-trading"] == "1"
        data = json.loads(body)
        assert data["instId"] == "BTC-USDT-SWAP" and data["sz"] == "25" and data["tdMode"] == "cross"
        return {"code": "0", "data": [{"ordId": "o1", "sCode": "0"}]}

    rec = Recorder(
        {
            ("GET", "/api/v5/public/instruments"): {"code": "0", "data": [{"ctVal": "0.01", "lotSz": "1", "minSz": "1", "tickSz": "0.1"}]},
            ("POST", "/api/v5/trade/order"): order,
            ("GET", "/api/v5/account/positions"): {"code": "0", "data": [{"instId": "BTC-USDT-SWAP", "pos": "-30", "posSide": "net", "avgPx": "60000", "markPx": "61000", "upl": "-30", "lever": "5"}]},
            ("GET", "/api/v5/market/history-candles"): {"code": "0", "data": [["3000", "1", "1", "1", "1", "1", "1", "1", "1"]]},
        }
    )
    async with OKXClient(api_key="k", api_secret="sec", passphrase="pp", testnet=True, transport=rec.transport) as c:
        res = await c.place_order("BTC/USDT", "swap", "buy", 0.259)  # 25.9 contracts -> floor to 25
        assert res.order_id == "o1"
        pos = await c.fetch_positions()
        assert pos[0].side == "short" and pos[0].size == pytest.approx(0.3)
        candles = await c.fetch_candles("BTC/USDT", "spot", "1d", limit=1, end_ms=10_000)
        assert candles[0].ts == 3000
        assert "bar=1Dutc" in str(rec.requests[-1].url) and "after=10001" in str(rec.requests[-1].url)


async def test_bitget_uta_v3_requests():
    def order(req):
        ts = req.headers["ACCESS-TIMESTAMP"]
        body = req.content.decode()
        expected = base64.b64encode(hmac.new(b"sec", (ts + "POST" + "/api/v3/trade/place-order" + body).encode(), hashlib.sha256).digest()).decode()
        assert req.headers["ACCESS-SIGN"] == expected
        data = json.loads(body)
        # spot market buys become marketable IOC limit orders so qty stays in base coin
        assert data == {"category": "SPOT", "symbol": "BTCUSDT", "qty": "0.01", "side": "buy", "orderType": "limit", "price": "60300", "timeInForce": "ioc"}
        return {"code": "00000", "data": {"orderId": "b1"}}

    def assets(req):
        ts = req.headers["ACCESS-TIMESTAMP"]
        expected = base64.b64encode(hmac.new(b"sec", (ts + "GET" + "/api/v3/account/assets").encode(), hashlib.sha256).digest()).decode()
        assert req.headers["ACCESS-SIGN"] == expected
        return {"code": "00000", "data": {"assets": [{"coin": "USDT", "balance": "100", "available": "90", "usdValue": "100"}]}}

    rec = Recorder(
        {
            ("GET", "/api/v3/market/tickers"): {"code": "00000", "data": [{"lastPrice": "60000"}]},
            ("GET", "/api/v3/market/instruments"): {"code": "00000", "data": [{"quantityPrecision": "4", "pricePrecision": "1", "minOrderQty": "0.0001"}]},
            ("POST", "/api/v3/trade/place-order"): order,
            ("GET", "/api/v3/account/assets"): assets,
            ("GET", "/api/v3/market/candles"): {"code": "00000", "data": [["1000", "1", "2", "0.5", "1.5", "3", "4"]]},
        }
    )
    async with BitgetClient(api_key="k", api_secret="sec", passphrase="p", transport=rec.transport) as c:
        res = await c.place_order("BTC/USDT", "spot", "buy", 0.01)
        assert res.order_id == "b1"
        bals = await c.fetch_balances()
        assert bals[0].total == 100 and bals[0].locked == 10
        candles = await c.fetch_candles("BTC/USDT", "swap", "4h", limit=1, end_ms=5000)
        assert candles[0].close == 1.5
        url = str(rec.requests[-1].url)
        assert "category=USDT-FUTURES" in url and "interval=4H" in url
        assert await c.normalize_qty("BTC/USDT", "spot", 0.123456) == 0.1234


async def test_pionex_signature_and_market_buy_amount():
    def order(req):
        path_url = req.url.raw_path.decode()
        assert "timestamp=" in path_url
        body = req.content.decode()
        expected = hmac.new(b"sec", ("POST" + path_url + body).encode(), hashlib.sha256).hexdigest()
        assert req.headers["PIONEX-SIGNATURE"] == expected
        data = json.loads(body)
        assert data == {"symbol": "BTC_USDT", "side": "BUY", "type": "MARKET", "amount": "600.5"}
        return {"result": True, "data": {"orderId": 99}}

    def balances(req):
        path_url = req.url.raw_path.decode()
        assert req.headers["PIONEX-SIGNATURE"] == hmac.new(b"sec", ("GET" + path_url).encode(), hashlib.sha256).hexdigest()
        return {"result": True, "data": {"balances": [{"coin": "BTC", "free": "0.9", "frozen": "0.1"}]}}

    rec = Recorder(
        {
            ("GET", "/api/v1/common/symbols"): {"result": True, "data": {"symbols": [{"amountPrecision": 1, "basePrecision": 6, "minTradeSize": "0.0001"}]}},
            ("GET", "/api/v1/market/tickers"): {"result": True, "data": {"tickers": [{"close": "60050"}]}},
            ("POST", "/api/v1/trade/order"): order,
            ("GET", "/api/v1/account/balances"): balances,
            ("GET", "/api/v1/market/klines"): {"result": True, "data": {"klines": [{"time": 1000, "open": "1", "close": "2", "high": "3", "low": "0.5", "volume": "7"}]}},
        }
    )
    async with PionexClient(api_key="k", api_secret="sec", transport=rec.transport) as c:
        res = await c.place_order("BTC/USDT", "spot", "buy", 0.01)
        assert res.order_id == "99"
        assert (await c.fetch_balances())[0].total == 1.0
        candles = await c.fetch_candles("BTC/USDT", "spot", "1h", limit=1, end_ms=5000)
        assert candles[0].high == 3 and "interval=60M" in str(rec.requests[-1].url)
        with pytest.raises(ExchangeError, match="perpetual"):
            await c.place_order("BTC/USDT", "swap", "buy", 1)


async def test_candle_paging_backwards():
    calls = []

    def klines(req):
        q = dict(parse_qsl(urlsplit(str(req.url)).query))
        end = int(q["endTime"])
        limit = int(q["limit"])
        calls.append((end, limit))
        tf = 60_000
        last = end // tf * tf
        return [[t, "1", "1", "1", "1", "1", t + tf - 1] for t in range(last - (limit - 1) * tf, last + tf, tf) if t >= 0]

    rec = Recorder({("GET", "/api/v3/klines"): klines})
    async with BinanceClient(transport=rec.transport) as c:
        c.max_candles_per_request = 100
        candles = await c.fetch_candles("BTC/USDT", "spot", "1m", limit=250, end_ms=1_000 * 60_000)
    assert len(candles) == 250 and len(calls) == 3
    assert all(b.ts - a.ts == 60_000 for a, b in zip(candles, candles[1:]))
