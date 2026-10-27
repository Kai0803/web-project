"""Bitget connector — Unified Trading Account (UTA) API V3.

Docs: https://www.bitget.com/api-doc/uta/intro
Signing: base64 HMAC-SHA256 of timestamp(ms) + METHOD + requestPath(+?query) + body.
Demo trading uses the same host with header `paptrading: 1`.
Requires the account to be upgraded to UTA (classic-account keys must use the V2 API instead).
"""
from __future__ import annotations

import base64
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
    decimals_to_step,
    fmt_num,
    hmac_sha256,
    now_ms,
    round_to_step,
    split_symbol,
)

INTERVALS = {"1m": "1m", "5m": "5m", "15m": "15m", "30m": "30m", "1h": "1H", "4h": "4H", "12h": "12H", "1d": "1D"}
SPOT_MARKET_BUY_SLIPPAGE = 0.005  # spot market buys are sent as marketable IOC limits so qty stays in base coin


class BitgetClient(ExchangeClient):
    name = "bitget"
    base_url = "https://api.bitget.com"
    needs_passphrase = True
    max_candles_per_request = 1000

    @staticmethod
    def to_symbol(symbol: str) -> str:
        base, quote = split_symbol(symbol)
        return base + quote

    @staticmethod
    def category(market: str) -> str:
        return "USDT-FUTURES" if market == "swap" else "SPOT"

    def sign(self, timestamp: str, method: str, request_path: str, body: str) -> str:
        return base64.b64encode(hmac_sha256(self.api_secret, timestamp + method + request_path + body)).decode()

    async def _request(self, method: str, path: str, params: dict | None = None, signed: bool = False):
        params = {k: v for k, v in (params or {}).items() if v is not None}
        headers = {"Content-Type": "application/json", "locale": "en-US"}
        body = ""
        if method == "GET" and params:
            path = f"{path}?{urlencode(sorted(params.items()))}"
        elif method != "GET":
            body = compact_json(params)
        if self.testnet:
            headers["paptrading"] = "1"
        if signed:
            self._require_keys()
            ts = str(now_ms())
            headers.update(
                {
                    "ACCESS-KEY": self.api_key,
                    "ACCESS-SIGN": self.sign(ts, method, path, body),
                    "ACCESS-TIMESTAMP": ts,
                    "ACCESS-PASSPHRASE": self.passphrase,
                }
            )
        data = await self._send(method, path, headers=headers, content=body or None)
        if str(data.get("code", "00000")) not in ("00000", "0"):
            raise ExchangeError(f"bitget: {data.get('code')} {data.get('msg')}")
        return data.get("data")

    async def _fetch_candles_page(self, symbol, market, timeframe, start_ms, end_ms, limit):
        rows = await self._request(
            "GET",
            "/api/v3/market/candles",
            {
                "category": self.category(market),
                "symbol": self.to_symbol(symbol),
                "interval": INTERVALS[timeframe],
                "startTime": start_ms,
                "endTime": end_ms,
                "limit": min(limit, 1000),
            },
        ) or []
        candles = [Candle(int(r[0]), float(r[1]), float(r[2]), float(r[3]), float(r[4]), float(r[5])) for r in rows]
        return sorted(candles, key=lambda c: c.ts)

    async def fetch_price(self, symbol, market="spot"):
        data = await self._request("GET", "/api/v3/market/tickers", {"category": self.category(market), "symbol": self.to_symbol(symbol)})
        items = data if isinstance(data, list) else [data] if data else []
        if not items:
            raise ExchangeError(f"bitget: no ticker for {symbol}")
        return float(items[0]["lastPrice"])

    async def fetch_rules(self, symbol, market):
        data = await self._request(
            "GET", "/api/v3/market/instruments", {"category": self.category(market), "symbol": self.to_symbol(symbol)}
        )
        items = data if isinstance(data, list) else [data] if data else []
        if not items:
            raise ExchangeError(f"bitget: unknown symbol {symbol}")
        d = items[0]
        return SymbolRules(
            qty_step=decimals_to_step(int(self._f(d.get("quantityPrecision"), 6))),
            min_qty=self._f(d.get("minOrderQty")),
            price_step=decimals_to_step(int(self._f(d.get("pricePrecision"), 2))),
        )

    async def fetch_balances(self):
        data = await self._request("GET", "/api/v3/account/assets", signed=True) or {}
        out = []
        for a in data.get("assets", []):
            total = self._f(a.get("balance")) or self._f(a.get("equity"))
            if not total:
                continue
            free = self._f(a.get("available"), total)
            out.append(Balance(a["coin"], free, max(total - free, 0.0), usd=self._f(a.get("usdValue")) or None, wallet="unified"))
        return out

    async def fetch_positions(self):
        data = await self._request("GET", "/api/v3/position/current-position", {"category": "USDT-FUTURES"}, signed=True) or {}
        out = []
        for p in data.get("list", []):
            size = self._f(p.get("total"))
            if not size:
                continue
            sym = p["symbol"]
            out.append(
                Position(
                    symbol=f"{sym[:-4]}/USDT" if sym.endswith("USDT") else sym,
                    side=p.get("posSide", "long"),
                    size=size,
                    entry_price=self._f(p.get("avgPrice")),
                    mark_price=self._f(p.get("markPrice")),
                    unrealized_pnl=self._f(p.get("unrealisedPnl")),
                    leverage=self._f(p.get("leverage"), 1.0),
                )
            )
        return out

    async def set_leverage(self, symbol, leverage):
        await self._request(
            "POST",
            "/api/v3/account/set-leverage",
            {"category": "USDT-FUTURES", "symbol": self.to_symbol(symbol), "leverage": str(int(leverage))},
            signed=True,
        )

    async def place_order(self, symbol, market, side, qty, order_type="market", price=None, reduce_only=False):
        self._check_market(market)
        body = {
            "category": self.category(market),
            "symbol": self.to_symbol(symbol),
            "qty": fmt_num(qty),
            "side": side.lower(),
            "orderType": "limit" if order_type == "limit" else "market",
        }
        if order_type == "limit":
            if price is None:
                raise ExchangeError("limit order requires price")
            body.update(price=fmt_num(price), timeInForce="gtc")
        elif market == "spot" and side.lower() == "buy":
            last = await self.fetch_price(symbol, market)
            rules = await self.get_rules(symbol, market)
            limit_px = last * (1 + SPOT_MARKET_BUY_SLIPPAGE)
            if rules.price_step:
                limit_px = round_to_step(limit_px, rules.price_step)
            body.update(orderType="limit", price=fmt_num(limit_px), timeInForce="ioc")
        if market == "swap" and reduce_only:
            body["reduceOnly"] = "yes"
        data = await self._request("POST", "/api/v3/trade/place-order", body, signed=True) or {}
        return OrderResult(str(data.get("orderId", "")), symbol, side, qty, None, "submitted", data)
