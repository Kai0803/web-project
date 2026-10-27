"""Historical candle sources for backtesting: exchange download (cached), CSV upload, synthetic."""
from __future__ import annotations

import csv
import io
import json
import math
import random
import re
from datetime import datetime, timezone

from .. import config
from ..exchanges import Candle, make_client
from ..exchanges.base import TIMEFRAME_MS

CACHE_DIR = config.DATA_DIR / "candles"
CACHE_DIR.mkdir(parents=True, exist_ok=True)


def _cache_file(exchange: str, symbol: str, market: str, timeframe: str):
    safe = re.sub(r"[^A-Za-z0-9]+", "", symbol.upper())
    return CACHE_DIR / f"{exchange}_{market}_{safe}_{timeframe}.json"


def _read_cache(path) -> dict[int, Candle]:
    if not path.exists():
        return {}
    rows = json.loads(path.read_text())
    return {r[0]: Candle(*r) for r in rows}


def _write_cache(path, candles: dict[int, Candle]) -> None:
    rows = [[c.ts, c.open, c.high, c.low, c.close, c.volume] for c in sorted(candles.values(), key=lambda c: c.ts)]
    path.write_text(json.dumps(rows, separators=(",", ":")))


async def load_exchange_candles(exchange: str, symbol: str, market: str, timeframe: str, start_ms: int, end_ms: int) -> list[Candle]:
    """Fetch [start_ms, end_ms] candles, using and updating a local cache (public endpoints, no keys)."""
    tf = TIMEFRAME_MS[timeframe]
    path = _cache_file(exchange, symbol, market, timeframe)
    cache = _read_cache(path)
    in_range = [ts for ts in cache if start_ms <= ts <= end_ms]
    expected = (end_ms - start_ms) // tf
    if len(in_range) < expected * 0.98:
        async with make_client(exchange) as client:
            fresh = await client.fetch_candles(symbol, market, timeframe, start_ms=start_ms, end_ms=end_ms)
        now = datetime.now(timezone.utc).timestamp() * 1000
        for c in fresh:
            if c.ts + tf <= now:  # only cache closed candles
                cache[c.ts] = c
        _write_cache(path, cache)
    return sorted((c for c in cache.values() if start_ms <= c.ts <= end_ms), key=lambda c: c.ts)


def _parse_time(value: str) -> int:
    value = value.strip()
    if re.fullmatch(r"\d+(\.\d+)?", value):
        n = float(value)
        return int(n if n > 1e11 else n * 1000)  # seconds or ms
    dt = datetime.fromisoformat(value.replace("Z", "+00:00").replace("/", "-"))
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return int(dt.timestamp() * 1000)


def parse_csv(text: str) -> list[Candle]:
    """Accept CSV with columns time/timestamp/date, open, high, low, close[, volume] (header optional)."""
    reader = csv.reader(io.StringIO(text.strip()))
    rows = [r for r in reader if r]
    if not rows:
        return []
    header = [h.strip().lower() for h in rows[0]]
    if any(h in header for h in ("open", "close")):
        idx = {
            "ts": next(i for i, h in enumerate(header) if h in ("time", "timestamp", "date", "datetime", "open_time", "ts")),
            "open": header.index("open"),
            "high": header.index("high"),
            "low": header.index("low"),
            "close": header.index("close"),
            "volume": header.index("volume") if "volume" in header else None,
        }
        rows = rows[1:]
    else:
        idx = {"ts": 0, "open": 1, "high": 2, "low": 3, "close": 4, "volume": 5 if len(rows[0]) > 5 else None}
    out = []
    for r in rows:
        out.append(
            Candle(
                _parse_time(r[idx["ts"]]),
                float(r[idx["open"]]),
                float(r[idx["high"]]),
                float(r[idx["low"]]),
                float(r[idx["close"]]),
                float(r[idx["volume"]]) if idx["volume"] is not None else 0.0,
            )
        )
    out.sort(key=lambda c: c.ts)
    return out


def synthetic_candles(n: int = 2000, timeframe: str = "1h", start_price: float = 30_000.0, seed: int = 7) -> list[Candle]:
    """Geometric random walk with regime changes — for trying strategies offline."""
    rng = random.Random(seed)
    tf = TIMEFRAME_MS[timeframe]
    ts = int(datetime(2024, 1, 1, tzinfo=timezone.utc).timestamp() * 1000)
    price, drift = start_price, 0.0
    vol = 0.01 * math.sqrt(tf / 3_600_000)
    out = []
    for i in range(n):
        if i % 200 == 0:
            drift = rng.uniform(-0.0006, 0.0008) * (tf / 3_600_000)
        o = price
        c = o * math.exp(drift + rng.gauss(0, vol))
        h = max(o, c) * (1 + abs(rng.gauss(0, vol / 2)))
        l = min(o, c) * (1 - abs(rng.gauss(0, vol / 2)))
        out.append(Candle(ts + i * tf, round(o, 2), round(h, 2), round(l, 2), round(c, 2), round(rng.uniform(10, 500), 3)))
        price = c
    return out
