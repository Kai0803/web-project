"""Strategy interface shared by the backtester and the live bot runner.

A strategy is a Python class deriving from `Strategy`:

    class SmaCross(Strategy):
        params = {"fast": 10, "slow": 30}

        def init(self, data):
            self.fast = ta.sma(data.close, self.p.fast)
            self.slow = ta.sma(data.close, self.p.slow)

        def next(self, i, data):
            if ta.crossover(self.fast, self.slow, i):
                return 1.0      # 100% long
            if ta.crossunder(self.fast, self.slow, i):
                return 0.0      # flat (use -1.0 for a full short on perpetuals)
            return None         # keep current position

`next` runs after bar i has closed and returns the *target exposure* as a fraction of
equity (-1..1, multiplied by leverage), or None to keep the current position.
The order is executed at the next bar's open (backtest) or immediately (live).
Within `next`, `self.position` is the current exposure and `self.entry_price` the
average entry of the open position (0 when flat).
"""
from __future__ import annotations

import math
from dataclasses import dataclass, field
from types import SimpleNamespace
from typing import Any

from ..exchanges.base import Candle
from . import indicators as ta


@dataclass
class Data:
    ts: list[int] = field(default_factory=list)
    open: list[float] = field(default_factory=list)
    high: list[float] = field(default_factory=list)
    low: list[float] = field(default_factory=list)
    close: list[float] = field(default_factory=list)
    volume: list[float] = field(default_factory=list)

    @classmethod
    def from_candles(cls, candles: list[Candle]) -> "Data":
        return cls(
            ts=[c.ts for c in candles],
            open=[c.open for c in candles],
            high=[c.high for c in candles],
            low=[c.low for c in candles],
            close=[c.close for c in candles],
            volume=[c.volume for c in candles],
        )

    def __len__(self) -> int:
        return len(self.close)


class Strategy:
    params: dict[str, Any] = {}
    # Minimum number of bars the live runner fetches before calling next().
    lookback: int = 300

    def __init__(self, **overrides: Any) -> None:
        merged = {**self.params, **{k: v for k, v in overrides.items() if k in self.params}}
        self.p = SimpleNamespace(**merged)
        self.position: float = 0.0
        self.entry_price: float = 0.0

    def init(self, data: Data) -> None:  # pragma: no cover - default no-op
        pass

    def next(self, i: int, data: Data) -> float | None:  # pragma: no cover - must override
        raise NotImplementedError


class StrategyLoadError(Exception):
    pass


def load_strategy_class(code: str) -> type[Strategy]:
    """Execute user strategy code and return the Strategy subclass it defines.

    NOTE: this runs arbitrary Python. The platform is meant to run locally for a single
    user; never expose it to the internet without authentication.
    """
    namespace: dict[str, Any] = {"Strategy": Strategy, "ta": ta, "math": math, "__name__": "user_strategy"}
    try:
        exec(compile(code, "<strategy>", "exec"), namespace)
    except Exception as e:  # noqa: BLE001 - surface any user error
        raise StrategyLoadError(f"{type(e).__name__}: {e}") from e
    classes = [v for v in namespace.values() if isinstance(v, type) and issubclass(v, Strategy) and v is not Strategy]
    if not classes:
        raise StrategyLoadError("No class deriving from Strategy found in code")
    return classes[-1]
