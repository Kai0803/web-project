"""Pionex connector — Open API v1 (spot).

Docs: https://github.com/pionex-official/pionex-open-api  (https://www.pionex.com/docs/api-docs)
Signing: hex HMAC-SHA256 of METHOD + PATH?sorted_query(incl. timestamp) [+ body for POST/DELETE];
headers PIONEX-KEY / PIONEX-SIGNATURE. Pionex grid bots are not exposed by the API; only spot orders.
"""
from __future__ import annotations

from .base import (
    Balance,
    Candle,
    ExchangeClient,
    ExchangeError,
    OrderResult,
    SymbolRules,
    compact_json,
    decimals_to_step,
    floor_to_step,
    fmt_num,
    hmac_sha256,
    now_ms,
    split_symbol,
)

INTERVALS = {"1m": "1M", "5m": "5M", "15m": "15M", "30m": "30M", "1h": "60M", "4h": "4H", "12h": "12H", "1d": "1D"}


class PionexClient(ExchangeClient):
    name = "pionex"
    base_url = "https://api.pionex.com"
    supports_swap = False
    max_candles_per_request = 500

    @staticmethod
    def to_symbol(symbol: str) -> str:
        base, quote = split_symbol(symbol)
        return f"{base}_{quote}"

    def sign(self, method: str, path: str, params: dict, body: str = "") -> tuple[str, str]:
        """Return (path_with_sorted_query, signature)."""
        query = "&".join(f"{k}={params[k]}" for k in sorted(params))
        path_url = f"{path}?{query}" if query else path
        payload = method + path_url + (body if method in ("POST", "DELETE") else "")
        return path_url, hmac_sha256(self.api_secret, payload).hex()

    async def _request(self, method: str, path: str, params: dict | None = None, body: dict | None = None, signed: bool = False):
        params = {k: v for k, v in (params or {}).items() if v is not None}
        headers = {"Content-Type": "application/json"}
        body_str = compact_json(body) if body is not None else ""
        if signed:
            self._require_keys()
            params["timestamp"] = now_ms()
            url, sig = self.sign(method, path, params, body_str)
            headers.update({"PIONEX-KEY": self.api_key, "PIONEX-SIGNATURE": sig})
        else:
            query = "&".join(f"{k}={params[k]}" for k in sorted(params))
            url = f"{path}?{query}" if query else path
        data = await self._send(method, url, headers=headers, content=body_str or None)
        if not data.get("result", False):
            raise ExchangeError(f"pionex: {data.get('code')} {data.get('message')}")
        return data.get("data", {})

    async def _fetch_candles_page(self, symbol, market, timeframe, start_ms, end_ms, limit):
        data = await self._request(
            "GET",
            "/api/v1/market/klines",
            {"symbol": self.to_symbol(symbol), "interval": INTERVALS[timeframe], "endTime": end_ms, "limit": min(limit, 500)},
        )
        candles = [
            Candle(int(k["time"]), float(k["open"]), float(k["high"]), float(k["low"]), float(k["close"]), float(k["volume"]))
            for k in data.get("klines", [])
        ]
        return sorted(candles, key=lambda c: c.ts)

    async def fetch_price(self, symbol, market="spot"):
        self._check_market(market)
        data = await self._request("GET", "/api/v1/market/tickers", {"symbol": self.to_symbol(symbol)})
        tickers = data.get("tickers") or []
        if not tickers:
            raise ExchangeError(f"pionex: no ticker for {symbol}")
        return float(tickers[0]["close"])

    async def _symbol_info(self, symbol: str) -> dict:
        data = await self._request("GET", "/api/v1/common/symbols", {"symbols": self.to_symbol(symbol)})
        items = data.get("symbols") or []
        if not items:
            raise ExchangeError(f"pionex: unknown symbol {symbol}")
        return items[0]

    async def fetch_rules(self, symbol, market):
        d = await self._symbol_info(symbol)
        return SymbolRules(
            qty_step=decimals_to_step(int(self._f(d.get("basePrecision"), 6))),
            min_qty=self._f(d.get("minTradeSize")),
            price_step=decimals_to_step(int(self._f(d.get("quotePrecision"), 2))),
        )

    async def fetch_balances(self):
        data = await self._request("GET", "/api/v1/account/balances", signed=True)
        out = []
        for b in data.get("balances", []):
            free, frozen = self._f(b.get("free")), self._f(b.get("frozen"))
            if free or frozen:
                out.append(Balance(b["coin"], free, frozen, wallet="spot"))
        return out

    async def place_order(self, symbol, market, side, qty, order_type="market", price=None, reduce_only=False):
        self._check_market(market)
        body: dict = {"symbol": self.to_symbol(symbol), "side": side.upper(), "type": order_type.upper()}
        if order_type == "limit":
            if price is None:
                raise ExchangeError("limit order requires price")
            body.update(size=fmt_num(qty), price=fmt_num(price))
        elif side.lower() == "buy":
            # Pionex market buys are specified in quote amount.
            info = await self._symbol_info(symbol)
            last = await self.fetch_price(symbol)
            step = decimals_to_step(int(self._f(info.get("amountPrecision"), 2)))
            body["amount"] = fmt_num(floor_to_step(qty * last, step))
        else:
            body["size"] = fmt_num(qty)
        data = await self._request("POST", "/api/v1/trade/order", body=body, signed=True)
        return OrderResult(str(data.get("orderId", "")), symbol, side, qty, None, "submitted", data)
