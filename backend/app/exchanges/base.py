"""Common types and helpers shared by every exchange connector.

Every connector speaks the exchange's *current* REST API directly (no third-party SDK):
  Binance  Spot /api/v3, USDⓈ-M Futures /fapi (v1 orders/klines, v3 account/positionRisk)
  Bybit    V5 unified API (/v5)
  OKX      V5 API (/api/v5)
  Bitget   UTA V3 API (/api/v3)
  Pionex   Open API v1 (/api/v1)

Symbols are unified as "BASE/QUOTE" (e.g. "BTC/USDT"); `market` is "spot" or "swap"
(USDT-margined perpetual). Quantities are always in BASE currency.
"""
from __future__ import annotations

import hashlib
import hmac
import json
import math
import time
from abc import ABC, abstractmethod
from dataclasses import asdict, dataclass, field
from decimal import ROUND_DOWN, ROUND_HALF_UP, Decimal
from typing import Any

import httpx

TIMEFRAMES = ["1m", "5m", "15m", "30m", "1h", "4h", "12h", "1d"]
TIMEFRAME_MS = {
    "1m": 60_000,
    "5m": 300_000,
    "15m": 900_000,
    "30m": 1_800_000,
    "1h": 3_600_000,
    "4h": 14_400_000,
    "12h": 43_200_000,
    "1d": 86_400_000,
}


class ExchangeError(Exception):
    pass


@dataclass
class Candle:
    ts: int  # open time, ms
    open: float
    high: float
    low: float
    close: float
    volume: float

    def to_dict(self) -> dict:
        return asdict(self)


@dataclass
class Balance:
    asset: str
    free: float
    locked: float = 0.0
    usd: float | None = None  # USD value when the exchange reports it
    wallet: str = "spot"  # spot / futures / unified / funding

    @property
    def total(self) -> float:
        return self.free + self.locked

    def to_dict(self) -> dict:
        return {**asdict(self), "total": self.total}


@dataclass
class Position:
    symbol: str  # unified
    side: str  # long / short
    size: float  # base qty, positive
    entry_price: float = 0.0
    mark_price: float = 0.0
    unrealized_pnl: float = 0.0
    leverage: float = 1.0

    def to_dict(self) -> dict:
        return asdict(self)


@dataclass
class OrderResult:
    order_id: str
    symbol: str
    side: str
    qty: float
    price: float | None = None
    status: str = "submitted"
    raw: dict = field(default_factory=dict)


@dataclass
class SymbolRules:
    qty_step: float = 0.0  # base qty step (0 = unknown)
    min_qty: float = 0.0
    contract_size: float = 1.0  # base per contract (OKX swaps)
    price_step: float = 0.0


def split_symbol(symbol: str) -> tuple[str, str]:
    base, _, quote = symbol.upper().replace("-", "/").replace("_", "/").partition("/")
    if not quote:
        raise ExchangeError(f"Symbol must look like BASE/QUOTE, got {symbol!r}")
    return base, quote


def floor_to_step(value: float, step: float) -> float:
    if step <= 0:
        return value
    d = (Decimal(str(value)) / Decimal(str(step))).to_integral_value(rounding=ROUND_DOWN) * Decimal(str(step))
    return float(d)


def round_to_step(value: float, step: float) -> float:
    if step <= 0:
        return value
    d = (Decimal(str(value)) / Decimal(str(step))).to_integral_value(rounding=ROUND_HALF_UP) * Decimal(str(step))
    return float(d)


def fmt_num(value: float) -> str:
    """Plain decimal string without exponent or trailing zeros."""
    s = format(Decimal(str(value)).normalize(), "f")
    return s


def decimals_to_step(decimals: int) -> float:
    return 10.0 ** -int(decimals)


def hmac_sha256(secret: str, message: str) -> bytes:
    return hmac.new(secret.encode(), message.encode(), hashlib.sha256).digest()


def now_ms() -> int:
    return int(time.time() * 1000)


def compact_json(data: dict | list) -> str:
    return json.dumps(data, separators=(",", ":"))


class ExchangeClient(ABC):
    name: str = ""
    base_url: str = ""
    testnet_url: str | None = None
    supports_swap: bool = True
    needs_passphrase: bool = False

    def __init__(
        self,
        api_key: str = "",
        api_secret: str = "",
        passphrase: str = "",
        testnet: bool = False,
        transport: httpx.AsyncBaseTransport | None = None,
        timeout: float = 15.0,
    ) -> None:
        self.api_key = api_key
        self.api_secret = api_secret
        self.passphrase = passphrase
        self.testnet = testnet
        url = self.testnet_url if (testnet and self.testnet_url) else self.base_url
        self._http = httpx.AsyncClient(base_url=url, timeout=timeout, transport=transport)
        self._rules: dict[tuple[str, str], SymbolRules] = {}

    async def close(self) -> None:
        await self._http.aclose()

    async def __aenter__(self):
        return self

    async def __aexit__(self, *exc):
        await self.close()

    def _require_keys(self) -> None:
        if not self.api_key or not self.api_secret:
            raise ExchangeError(f"{self.name}: API key/secret not configured")
        if self.needs_passphrase and not self.passphrase:
            raise ExchangeError(f"{self.name}: API passphrase not configured")

    def _check_market(self, market: str) -> None:
        if market not in ("spot", "swap"):
            raise ExchangeError(f"Unknown market {market!r}")
        if market == "swap" and not self.supports_swap:
            raise ExchangeError(f"{self.name}: perpetual trading is not supported by this connector")

    async def _send(self, method: str, path: str, **kwargs: Any) -> Any:
        try:
            resp = await self._http.request(method, path, **kwargs)
        except httpx.HTTPError as e:
            raise ExchangeError(f"{self.name}: network error {e!r}") from e
        try:
            data = resp.json()
        except ValueError:
            raise ExchangeError(f"{self.name}: HTTP {resp.status_code} {resp.text[:300]}")
        if resp.status_code >= 400:
            raise ExchangeError(f"{self.name}: HTTP {resp.status_code} {compact_json(data)[:500]}")
        return data

    # ---- market data -------------------------------------------------------
    @abstractmethod
    async def _fetch_candles_page(
        self, symbol: str, market: str, timeframe: str, start_ms: int | None, end_ms: int | None, limit: int
    ) -> list[Candle]:
        """Return candles (ascending) within [start_ms, end_ms]; may return fewer than requested."""

    max_candles_per_request: int = 500

    async def fetch_candles(
        self,
        symbol: str,
        market: str = "spot",
        timeframe: str = "1h",
        start_ms: int | None = None,
        end_ms: int | None = None,
        limit: int = 500,
    ) -> list[Candle]:
        """Fetch up to `limit` candles ending at end_ms (default now), paging backwards as needed."""
        self._check_market(market)
        if timeframe not in TIMEFRAME_MS:
            raise ExchangeError(f"Unsupported timeframe {timeframe}")
        tf = TIMEFRAME_MS[timeframe]
        end = end_ms or now_ms()
        if start_ms is not None:
            limit = min(max(1, (end - start_ms) // tf + 1), 100_000)
        out: dict[int, Candle] = {}
        cursor_end = end
        while len(out) < limit:
            want = min(self.max_candles_per_request, limit - len(out))
            page_start = cursor_end - want * tf
            if start_ms is not None:
                page_start = max(page_start, start_ms)
            page = await self._fetch_candles_page(symbol, market, timeframe, page_start, cursor_end, want)
            page = [c for c in page if c.ts <= cursor_end and (start_ms is None or c.ts >= start_ms)]
            new = [c for c in page if c.ts not in out]
            if not new:
                break
            for c in new:
                out[c.ts] = c
            cursor_end = min(c.ts for c in new) - 1
            if start_ms is not None and cursor_end < start_ms:
                break
        candles = sorted(out.values(), key=lambda c: c.ts)
        return candles[-limit:]

    @abstractmethod
    async def fetch_price(self, symbol: str, market: str = "spot") -> float: ...

    async def fetch_rules(self, symbol: str, market: str) -> SymbolRules:
        return SymbolRules()

    async def get_rules(self, symbol: str, market: str) -> SymbolRules:
        key = (symbol, market)
        if key not in self._rules:
            try:
                self._rules[key] = await self.fetch_rules(symbol, market)
            except ExchangeError:
                self._rules[key] = SymbolRules()
        return self._rules[key]

    # ---- account -----------------------------------------------------------
    @abstractmethod
    async def fetch_balances(self) -> list[Balance]: ...

    async def fetch_positions(self) -> list[Position]:
        return []

    @abstractmethod
    async def place_order(
        self,
        symbol: str,
        market: str,
        side: str,
        qty: float,
        order_type: str = "market",
        price: float | None = None,
        reduce_only: bool = False,
    ) -> OrderResult: ...

    async def set_leverage(self, symbol: str, leverage: float) -> None:
        return None

    async def normalize_qty(self, symbol: str, market: str, qty: float) -> float:
        rules = await self.get_rules(symbol, market)
        q = floor_to_step(qty, rules.qty_step) if rules.qty_step else float(f"{qty:.8f}")
        if rules.min_qty and q < rules.min_qty:
            return 0.0
        return q

    @staticmethod
    def _f(value: Any, default: float = 0.0) -> float:
        try:
            v = float(value)
            return v if math.isfinite(v) else default
        except (TypeError, ValueError):
            return default
