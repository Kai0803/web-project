"""Causal technical indicators on plain Python lists.

Every function returns a list the same length as its input; values that cannot be
computed yet (warm-up) are None. Value at index i only depends on inputs[0..i],
so the same strategy code is safe for backtests and live trading.
"""
from __future__ import annotations

import math
from collections.abc import Sequence

Series = Sequence[float | None]


def sma(values: Series, period: int) -> list[float | None]:
    out: list[float | None] = [None] * len(values)
    total, count = 0.0, 0
    for i, v in enumerate(values):
        if v is None:
            total, count = 0.0, 0
            continue
        total += v
        count += 1
        if count > period:
            total -= values[i - period]  # type: ignore[operator]
            count = period
        if count == period:
            out[i] = total / period
    return out


def ema(values: Series, period: int) -> list[float | None]:
    out: list[float | None] = [None] * len(values)
    k = 2 / (period + 1)
    prev: float | None = None
    seed: list[float] = []
    for i, v in enumerate(values):
        if v is None:
            continue
        if prev is None:
            seed.append(v)
            if len(seed) == period:
                prev = sum(seed) / period
                out[i] = prev
            continue
        prev = v * k + prev * (1 - k)
        out[i] = prev
    return out


def rsi(values: Series, period: int = 14) -> list[float | None]:
    out: list[float | None] = [None] * len(values)
    avg_gain = avg_loss = 0.0
    for i in range(1, len(values)):
        change = values[i] - values[i - 1]  # type: ignore[operator]
        gain, loss = max(change, 0.0), max(-change, 0.0)
        if i <= period:
            avg_gain += gain / period
            avg_loss += loss / period
            if i < period:
                continue
        else:
            avg_gain = (avg_gain * (period - 1) + gain) / period
            avg_loss = (avg_loss * (period - 1) + loss) / period
        out[i] = 100.0 if avg_loss == 0 else 100 - 100 / (1 + avg_gain / avg_loss)
    return out


def macd(values: Series, fast: int = 12, slow: int = 26, signal: int = 9):
    """Return (macd_line, signal_line, histogram)."""
    f, s = ema(values, fast), ema(values, slow)
    line = [a - b if a is not None and b is not None else None for a, b in zip(f, s)]
    sig = ema(line, signal)
    hist = [a - b if a is not None and b is not None else None for a, b in zip(line, sig)]
    return line, sig, hist


def stdev(values: Series, period: int) -> list[float | None]:
    out: list[float | None] = [None] * len(values)
    for i in range(period - 1, len(values)):
        window = values[i - period + 1 : i + 1]
        if any(v is None for v in window):
            continue
        m = sum(window) / period  # type: ignore[arg-type]
        out[i] = math.sqrt(sum((v - m) ** 2 for v in window) / period)  # type: ignore[operator]
    return out


def bollinger(values: Series, period: int = 20, mult: float = 2.0):
    """Return (middle, upper, lower)."""
    mid, sd = sma(values, period), stdev(values, period)
    upper = [m + mult * s if m is not None and s is not None else None for m, s in zip(mid, sd)]
    lower = [m - mult * s if m is not None and s is not None else None for m, s in zip(mid, sd)]
    return mid, upper, lower


def true_range(high: Series, low: Series, close: Series) -> list[float | None]:
    out: list[float | None] = []
    for i in range(len(close)):
        if i == 0:
            out.append(high[i] - low[i])  # type: ignore[operator]
        else:
            out.append(max(high[i] - low[i], abs(high[i] - close[i - 1]), abs(low[i] - close[i - 1])))  # type: ignore[operator]
    return out


def atr(high: Series, low: Series, close: Series, period: int = 14) -> list[float | None]:
    tr = true_range(high, low, close)
    out: list[float | None] = [None] * len(tr)
    prev: float | None = None
    for i, v in enumerate(tr):
        if i < period - 1:
            continue
        if prev is None:
            prev = sum(tr[: period]) / period  # type: ignore[arg-type]
        else:
            prev = (prev * (period - 1) + v) / period  # type: ignore[operator]
        out[i] = prev
    return out


def highest(values: Series, period: int) -> list[float | None]:
    return [max(values[i - period + 1 : i + 1]) if i >= period - 1 else None for i in range(len(values))]  # type: ignore[type-var]


def lowest(values: Series, period: int) -> list[float | None]:
    return [min(values[i - period + 1 : i + 1]) if i >= period - 1 else None for i in range(len(values))]  # type: ignore[type-var]


def crossover(a: Series, b: Series, i: int) -> bool:
    """a crosses above b at bar i."""
    if i < 1 or None in (a[i], b[i], a[i - 1], b[i - 1]):
        return False
    return a[i - 1] <= b[i - 1] and a[i] > b[i]  # type: ignore[operator]


def crossunder(a: Series, b: Series, i: int) -> bool:
    if i < 1 or None in (a[i], b[i], a[i - 1], b[i - 1]):
        return False
    return a[i - 1] >= b[i - 1] and a[i] < b[i]  # type: ignore[operator]
