"""OKX connector — V5 API.

Docs: https://www.okx.com/docs-v5/en/
Signing: base64 HMAC-SHA256 of ISO-timestamp + METHOD + requestPath(+query) + body.
Demo trading uses the same host with header `x-simulated-trading: 1`.
Perpetuals are traded cross-margin in net (one-way) mode; quantities are converted to contracts via ctVal.
"""
from __future__ import annotations

import base64
from datetime import datetime, timezone
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
    floor_to_step,
    fmt_num,
    hmac_sha256,
    split_symbol,
)

BARS = {"1m": "1m", "5m": "5m", "15m": "15m", "30m": "30m", "1h": "1H", "4h": "4H", "12h": "12Hutc", "1d": "1Dutc"}


def iso_timestamp() -> str:
    now = datetime.now(timezone.utc)
    return now.strftime("%Y-%m-%dT%H:%M:%S.") + f"{now.microsecond // 1000:03d}Z"


class OKXClient(ExchangeClient):
    name = "okx"
    base_url = "https://www.okx.com"
    needs_passphrase = True
    max_candles_per_request = 100  # history-candles page size

    @staticmethod
    def inst_id(symbol: str, market: str) -> str:
        base, quote = split_symbol(symbol)
        return f"{base}-{quote}-SWAP" if market == "swap" else f"{base}-{quote}"

    def sign(self, timestamp: str, method: str, request_path: str, body: str) -> str:
        return base64.b64encode(hmac_sha256(self.api_secret, timestamp + method + request_path + body)).decode()

    async def _request(self, method: str, path: str, params: dict | None = None, signed: bool = False):
        params = {k: v for k, v in (params or {}).items() if v is not None}
        headers = {"Content-Type": "application/json"}
        body = ""
        if method == "GET" and params:
            path = f"{path}?{urlencode(params)}"
        elif method != "GET":
            body = compact_json(params)
        if self.testnet:
            headers["x-simulated-trading"] = "1"
        if signed:
            self._require_keys()
            ts = iso_timestamp()
            headers.update(
                {
                    "OK-ACCESS-KEY": self.api_key,
                    "OK-ACCESS-SIGN": self.sign(ts, method, path, body),
                    "OK-ACCESS-TIMESTAMP": ts,
                    "OK-ACCESS-PASSPHRASE": self.passphrase,
                }
            )
        data = await self._send(method, path, headers=headers, content=body or None)
        if str(data.get("code", "0")) != "0":
            detail = data.get("data") or ""
            raise ExchangeError(f"okx: {data.get('code')} {data.get('msg')} {detail}")
        return data.get("data", [])

    async def _fetch_candles_page(self, symbol, market, timeframe, start_ms, end_ms, limit):
        params = {"instId": self.inst_id(symbol, market), "bar": BARS[timeframe], "limit": min(limit, 100)}
        if end_ms is not None:
            params["after"] = end_ms + 1  # records strictly earlier than `after`
        rows = await self._request("GET", "/api/v5/market/history-candles", params)
        candles = [Candle(int(r[0]), float(r[1]), float(r[2]), float(r[3]), float(r[4]), float(r[5])) for r in rows]
        return sorted(candles, key=lambda c: c.ts)

    async def fetch_price(self, symbol, market="spot"):
        data = await self._request("GET", "/api/v5/market/ticker", {"instId": self.inst_id(symbol, market)})
        if not data:
            raise ExchangeError(f"okx: no ticker for {symbol}")
        return float(data[0]["last"])

    async def fetch_rules(self, symbol, market):
        data = await self._request(
            "GET",
            "/api/v5/public/instruments",
            {"instType": "SWAP" if market == "swap" else "SPOT", "instId": self.inst_id(symbol, market)},
        )
        if not data:
            raise ExchangeError(f"okx: unknown instrument {symbol}")
        d = data[0]
        ct = self._f(d.get("ctVal"), 1.0) if market == "swap" else 1.0
        return SymbolRules(
            qty_step=self._f(d.get("lotSz")) * ct,
            min_qty=self._f(d.get("minSz")) * ct,
            contract_size=ct,
            price_step=self._f(d.get("tickSz")),
        )

    async def fetch_balances(self):
        data = await self._request("GET", "/api/v5/account/balance", signed=True)
        out = []
        for acct in data:
            for d in acct.get("details", []):
                total = self._f(d.get("cashBal")) or self._f(d.get("eq"))
                if not total:
                    continue
                free = self._f(d.get("availBal"), total)
                out.append(Balance(d["ccy"], free, max(total - free, 0.0), usd=self._f(d.get("eqUsd")) or None, wallet="trading"))
        return out

    async def fetch_positions(self):
        data = await self._request("GET", "/api/v5/account/positions", {"instType": "SWAP"}, signed=True)
        out = []
        for p in data:
            pos = self._f(p.get("pos"))
            if not pos:
                continue
            parts = p["instId"].split("-")
            symbol = f"{parts[0]}/{parts[1]}"
            rules = await self.get_rules(symbol, "swap")
            side = p.get("posSide")
            if side not in ("long", "short"):
                side = "long" if pos > 0 else "short"
            out.append(
                Position(
                    symbol=symbol,
                    side=side,
                    size=abs(pos) * rules.contract_size,
                    entry_price=self._f(p.get("avgPx")),
                    mark_price=self._f(p.get("markPx")),
                    unrealized_pnl=self._f(p.get("upl")),
                    leverage=self._f(p.get("lever"), 1.0),
                )
            )
        return out

    async def set_leverage(self, symbol, leverage):
        await self._request(
            "POST",
            "/api/v5/account/set-leverage",
            {"instId": self.inst_id(symbol, "swap"), "lever": fmt_num(leverage), "mgnMode": "cross"},
            signed=True,
        )

    async def place_order(self, symbol, market, side, qty, order_type="market", price=None, reduce_only=False):
        self._check_market(market)
        body = {
            "instId": self.inst_id(symbol, market),
            "tdMode": "cross" if market == "swap" else "cash",
            "side": side.lower(),
            "ordType": "limit" if order_type == "limit" else "market",
        }
        if market == "swap":
            rules = await self.get_rules(symbol, market)
            lot = rules.qty_step / rules.contract_size if rules.qty_step else 1.0
            contracts = floor_to_step(qty / rules.contract_size, lot)
            if contracts <= 0:
                raise ExchangeError(f"okx: qty {qty} is below one contract ({rules.contract_size})")
            body["sz"] = fmt_num(contracts)
            if reduce_only:
                body["reduceOnly"] = True
        else:
            body["sz"] = fmt_num(qty)
            if order_type == "market":
                body["tgtCcy"] = "base_ccy"
        if order_type == "limit":
            if price is None:
                raise ExchangeError("limit order requires price")
            body["px"] = fmt_num(price)
        data = await self._request("POST", "/api/v5/trade/order", body, signed=True)
        d = data[0] if data else {}
        if str(d.get("sCode", "0")) != "0":
            raise ExchangeError(f"okx: order rejected {d.get('sCode')} {d.get('sMsg')}")
        return OrderResult(str(d.get("ordId", "")), symbol, side, qty, None, "submitted", d)
