"""Binance connector — Spot API v3 and USDⓈ-M Futures (fapi).

Docs: https://developers.binance.com/docs/binance-spot-api-docs/rest-api
      https://developers.binance.com/docs/derivatives/usds-margined-futures
Signing: HMAC-SHA256 (hex) of the full query string, sent as `signature`; key in X-MBX-APIKEY.
Futures assumes one-way position mode.
"""
from __future__ import annotations

import hashlib
import hmac
from urllib.parse import urlencode

import httpx

from .base import (
    Balance,
    Candle,
    ExchangeClient,
    ExchangeError,
    OrderResult,
    Position,
    SymbolRules,
    fmt_num,
    now_ms,
    split_symbol,
)

INTERVALS = {"1m": "1m", "5m": "5m", "15m": "15m", "30m": "30m", "1h": "1h", "4h": "4h", "12h": "12h", "1d": "1d"}


class BinanceClient(ExchangeClient):
    name = "binance"
    base_url = "https://api.binance.com"
    testnet_url = "https://testnet.binance.vision"
    futures_url = "https://fapi.binance.com"
    futures_testnet_url = "https://demo-fapi.binance.com"
    max_candles_per_request = 1000
    recv_window = 5000

    def __init__(self, *args, transport: httpx.AsyncBaseTransport | None = None, **kwargs) -> None:
        super().__init__(*args, transport=transport, **kwargs)
        fut = self.futures_testnet_url if self.testnet else self.futures_url
        self._fut = httpx.AsyncClient(base_url=fut, timeout=self._http.timeout, transport=transport)

    async def close(self) -> None:
        await super().close()
        await self._fut.aclose()

    @staticmethod
    def to_symbol(symbol: str) -> str:
        base, quote = split_symbol(symbol)
        return base + quote

    def sign(self, params: dict) -> str:
        """Return the signed query string (params + timestamp + recvWindow + signature)."""
        query = urlencode(params)
        sig = hmac.new(self.api_secret.encode(), query.encode(), hashlib.sha256).hexdigest()
        return f"{query}&signature={sig}"

    async def _request(self, method: str, path: str, params: dict | None = None, signed: bool = False, futures: bool = False):
        params = {k: v for k, v in (params or {}).items() if v is not None}
        headers = {}
        if signed:
            self._require_keys()
            params.update(recvWindow=self.recv_window, timestamp=now_ms())
            query = self.sign(params)
            headers["X-MBX-APIKEY"] = self.api_key
        else:
            query = urlencode(params)
        url = f"{path}?{query}" if query else path
        client = self._fut if futures else self._http
        try:
            resp = await client.request(method, url, headers=headers)
        except httpx.HTTPError as e:
            raise ExchangeError(f"binance: network error {e!r}") from e
        try:
            data = resp.json()
        except ValueError:
            raise ExchangeError(f"binance: HTTP {resp.status_code} {resp.text[:300]}")
        if resp.status_code >= 400 or (isinstance(data, dict) and data.get("code", 0) not in (0, 200) and "msg" in data):
            raise ExchangeError(f"binance: {data}")
        return data

    async def _fetch_candles_page(self, symbol, market, timeframe, start_ms, end_ms, limit):
        path = "/fapi/v1/klines" if market == "swap" else "/api/v3/klines"
        rows = await self._request(
            "GET",
            path,
            {"symbol": self.to_symbol(symbol), "interval": INTERVALS[timeframe], "startTime": start_ms, "endTime": end_ms, "limit": min(limit, 1000)},
            futures=market == "swap",
        )
        return [Candle(int(r[0]), float(r[1]), float(r[2]), float(r[3]), float(r[4]), float(r[5])) for r in rows]

    async def fetch_price(self, symbol, market="spot"):
        path = "/fapi/v2/ticker/price" if market == "swap" else "/api/v3/ticker/price"
        data = await self._request("GET", path, {"symbol": self.to_symbol(symbol)}, futures=market == "swap")
        return float(data["price"])

    async def fetch_rules(self, symbol, market):
        sym = self.to_symbol(symbol)
        if market == "swap":
            info = await self._request("GET", "/fapi/v1/exchangeInfo", futures=True)
        else:
            info = await self._request("GET", "/api/v3/exchangeInfo", {"symbol": sym})
        for s in info.get("symbols", []):
            if s.get("symbol") != sym:
                continue
            rules = SymbolRules()
            for f in s.get("filters", []):
                if f.get("filterType") in ("LOT_SIZE", "MARKET_LOT_SIZE"):
                    step = self._f(f.get("stepSize"))
                    if step and (not rules.qty_step or step > rules.qty_step):
                        rules.qty_step = step
                    rules.min_qty = max(rules.min_qty, self._f(f.get("minQty")))
                elif f.get("filterType") == "PRICE_FILTER":
                    rules.price_step = self._f(f.get("tickSize"))
            return rules
        raise ExchangeError(f"binance: unknown symbol {sym}")

    async def fetch_balances(self):
        out: list[Balance] = []
        spot = await self._request("GET", "/api/v3/account", {"omitZeroBalances": "true"}, signed=True)
        for b in spot.get("balances", []):
            free, locked = self._f(b.get("free")), self._f(b.get("locked"))
            if free or locked:
                out.append(Balance(b["asset"], free, locked, wallet="spot"))
        try:
            fut = await self._request("GET", "/fapi/v3/account", signed=True, futures=True)
        except ExchangeError:
            fut = {}  # futures not enabled for this key
        for a in fut.get("assets", []):
            wallet = self._f(a.get("walletBalance"))
            if wallet:
                avail = self._f(a.get("availableBalance"))
                out.append(Balance(a["asset"], avail, max(wallet - avail, 0.0), wallet="futures"))
        return out

    async def fetch_positions(self):
        try:
            rows = await self._request("GET", "/fapi/v3/positionRisk", signed=True, futures=True)
        except ExchangeError:
            return []
        out = []
        for p in rows:
            amt = self._f(p.get("positionAmt"))
            if not amt:
                continue
            sym = p["symbol"]
            quote = "USDT" if sym.endswith("USDT") else "USDC" if sym.endswith("USDC") else sym[-4:]
            notional, im = abs(self._f(p.get("notional"))), self._f(p.get("initialMargin"))
            out.append(
                Position(
                    symbol=f"{sym[: -len(quote)]}/{quote}",
                    side="long" if amt > 0 else "short",
                    size=abs(amt),
                    entry_price=self._f(p.get("entryPrice")),
                    mark_price=self._f(p.get("markPrice")),
                    unrealized_pnl=self._f(p.get("unRealizedProfit")),
                    leverage=round(notional / im, 2) if im else 1.0,
                )
            )
        return out

    async def set_leverage(self, symbol, leverage):
        await self._request(
            "POST", "/fapi/v1/leverage", {"symbol": self.to_symbol(symbol), "leverage": int(leverage)}, signed=True, futures=True
        )

    async def place_order(self, symbol, market, side, qty, order_type="market", price=None, reduce_only=False):
        self._check_market(market)
        params = {
            "symbol": self.to_symbol(symbol),
            "side": side.upper(),
            "type": order_type.upper(),
            "quantity": fmt_num(qty),
        }
        if order_type == "limit":
            if price is None:
                raise ExchangeError("limit order requires price")
            params.update(price=fmt_num(price), timeInForce="GTC")
        if market == "swap":
            if reduce_only:
                params["reduceOnly"] = "true"
            params["newOrderRespType"] = "RESULT"
            data = await self._request("POST", "/fapi/v1/order", params, signed=True, futures=True)
            avg = self._f(data.get("avgPrice")) or None
        else:
            params["newOrderRespType"] = "FULL"
            data = await self._request("POST", "/api/v3/order", params, signed=True)
            exec_qty = self._f(data.get("executedQty"))
            avg = self._f(data.get("cummulativeQuoteQty")) / exec_qty if exec_qty else None
        return OrderResult(str(data.get("orderId", "")), symbol, side, qty, avg, str(data.get("status", "")).lower(), data)
