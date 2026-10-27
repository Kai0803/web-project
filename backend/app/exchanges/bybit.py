"""Bybit connector — V5 unified API.

Docs: https://bybit-exchange.github.io/docs/v5/intro
Signing: hex HMAC-SHA256 of timestamp + apiKey + recvWindow + (queryString | jsonBody).
Assumes a Unified Trading Account and one-way position mode for linear perpetuals.
"""
from __future__ import annotations

from urllib.parse import urlencode

from .base import (
    Balance,
    Candle,
    ExchangeClient,
    ExchangeError,
    OrderResult,
    Position,
    SymbolRules,
    compact_json,
    fmt_num,
    hmac_sha256,
    now_ms,
    split_symbol,
)

INTERVALS = {"1m": "1", "5m": "5", "15m": "15", "30m": "30", "1h": "60", "4h": "240", "12h": "720", "1d": "D"}
LEVERAGE_NOT_MODIFIED = 110043


class BybitClient(ExchangeClient):
    name = "bybit"
    base_url = "https://api.bybit.com"
    testnet_url = "https://api-testnet.bybit.com"
    max_candles_per_request = 1000
    recv_window = "5000"

    @staticmethod
    def to_symbol(symbol: str) -> str:
        base, quote = split_symbol(symbol)
        return base + quote

    @staticmethod
    def category(market: str) -> str:
        return "linear" if market == "swap" else "spot"

    def sign(self, timestamp: str, payload: str) -> str:
        return hmac_sha256(self.api_secret, timestamp + self.api_key + self.recv_window + payload).hex()

    async def _request(self, method: str, path: str, params: dict | None = None, signed: bool = False):
        params = {k: v for k, v in (params or {}).items() if v is not None}
        headers = {"Content-Type": "application/json"}
        body = ""
        if method == "GET":
            query = urlencode(params)
            url = f"{path}?{query}" if query else path
            payload = query
        else:
            body = compact_json(params)
            url, payload = path, body
        if signed:
            self._require_keys()
            ts = str(now_ms())
            headers.update(
                {
                    "X-BAPI-API-KEY": self.api_key,
                    "X-BAPI-TIMESTAMP": ts,
                    "X-BAPI-RECV-WINDOW": self.recv_window,
                    "X-BAPI-SIGN": self.sign(ts, payload),
                }
            )
        data = await self._send(method, url, headers=headers, content=body or None)
        code = data.get("retCode", 0)
        if code != 0:
            err = ExchangeError(f"bybit: {code} {data.get('retMsg')}")
            err.code = code  # type: ignore[attr-defined]
            raise err
        return data.get("result", {})

    async def _fetch_candles_page(self, symbol, market, timeframe, start_ms, end_ms, limit):
        res = await self._request(
            "GET",
            "/v5/market/kline",
            {
                "category": self.category(market),
                "symbol": self.to_symbol(symbol),
                "interval": INTERVALS[timeframe],
                "start": start_ms,
                "end": end_ms,
                "limit": min(limit, 1000),
            },
        )
        rows = res.get("list", [])
        candles = [Candle(int(r[0]), float(r[1]), float(r[2]), float(r[3]), float(r[4]), float(r[5])) for r in rows]
        return sorted(candles, key=lambda c: c.ts)

    async def fetch_price(self, symbol, market="spot"):
        res = await self._request("GET", "/v5/market/tickers", {"category": self.category(market), "symbol": self.to_symbol(symbol)})
        items = res.get("list") or []
        if not items:
            raise ExchangeError(f"bybit: no ticker for {symbol}")
        return float(items[0]["lastPrice"])

    async def fetch_rules(self, symbol, market):
        res = await self._request(
            "GET", "/v5/market/instruments-info", {"category": self.category(market), "symbol": self.to_symbol(symbol)}
        )
        items = res.get("list") or []
        if not items:
            raise ExchangeError(f"bybit: unknown symbol {symbol}")
        lot = items[0].get("lotSizeFilter", {})
        price = items[0].get("priceFilter", {})
        return SymbolRules(
            qty_step=self._f(lot.get("qtyStep") or lot.get("basePrecision")),
            min_qty=self._f(lot.get("minOrderQty")),
            price_step=self._f(price.get("tickSize")),
        )

    async def fetch_balances(self):
        res = await self._request("GET", "/v5/account/wallet-balance", {"accountType": "UNIFIED"}, signed=True)
        out = []
        for acct in res.get("list", []):
            for c in acct.get("coin", []):
                total = self._f(c.get("walletBalance"))
                if not total:
                    continue
                locked = self._f(c.get("locked"))
                out.append(Balance(c["coin"], max(total - locked, 0.0), locked, usd=self._f(c.get("usdValue")) or None, wallet="unified"))
        return out

    async def fetch_positions(self):
        res = await self._request("GET", "/v5/position/list", {"category": "linear", "settleCoin": "USDT"}, signed=True)
        out = []
        for p in res.get("list", []):
            size = self._f(p.get("size"))
            if not size:
                continue
            sym = p["symbol"]
            out.append(
                Position(
                    symbol=f"{sym[:-4]}/USDT" if sym.endswith("USDT") else sym,
                    side="long" if p.get("side") == "Buy" else "short",
                    size=size,
                    entry_price=self._f(p.get("avgPrice")),
                    mark_price=self._f(p.get("markPrice")),
                    unrealized_pnl=self._f(p.get("unrealisedPnl")),
                    leverage=self._f(p.get("leverage"), 1.0),
                )
            )
        return out

    async def set_leverage(self, symbol, leverage):
        lev = fmt_num(leverage)
        try:
            await self._request(
                "POST",
                "/v5/position/set-leverage",
                {"category": "linear", "symbol": self.to_symbol(symbol), "buyLeverage": lev, "sellLeverage": lev},
                signed=True,
            )
        except ExchangeError as e:
            if getattr(e, "code", None) != LEVERAGE_NOT_MODIFIED:
                raise

    async def place_order(self, symbol, market, side, qty, order_type="market", price=None, reduce_only=False):
        self._check_market(market)
        body = {
            "category": self.category(market),
            "symbol": self.to_symbol(symbol),
            "side": "Buy" if side.lower() == "buy" else "Sell",
            "orderType": "Limit" if order_type == "limit" else "Market",
            "qty": fmt_num(qty),
        }
        if order_type == "limit":
            if price is None:
                raise ExchangeError("limit order requires price")
            body.update(price=fmt_num(price), timeInForce="GTC")
        if market == "spot" and order_type == "market":
            body["marketUnit"] = "baseCoin"  # qty in base coin for spot market buys too
        if market == "swap" and reduce_only:
            body["reduceOnly"] = True
        res = await self._request("POST", "/v5/order/create", body, signed=True)
        return OrderResult(str(res.get("orderId", "")), symbol, side, qty, None, "submitted", res)
