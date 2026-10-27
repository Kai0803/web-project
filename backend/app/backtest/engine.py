"""Bar-by-bar backtester.

Signals are evaluated on bar close and filled at the next bar's open, with fees and
slippage applied. Optional stop-loss / take-profit are checked against each bar's
high/low (stop first when both are touched — the conservative assumption).
"""
from __future__ import annotations

import math
from dataclasses import asdict, dataclass

from ..exchanges.base import TIMEFRAME_MS, Candle
from .strategy import Data, Strategy


@dataclass
class BacktestConfig:
    initial_capital: float = 10_000.0
    fee_rate: float = 0.001  # per side, e.g. 0.1% taker
    slippage: float = 0.0005  # fraction of price
    leverage: float = 1.0
    allow_short: bool = False  # true for perpetual markets
    stop_loss: float = 0.0  # e.g. 0.05 = 5% adverse move from entry; 0 disables
    take_profit: float = 0.0
    timeframe: str = "1h"


class Portfolio:
    def __init__(self, cash: float) -> None:
        self.cash = cash
        self.qty = 0.0  # signed base quantity
        self.entry = 0.0  # average entry price of open position
        self.fills: list[dict] = []

    def equity(self, price: float) -> float:
        return self.cash + self.qty * price

    def trade(self, ts: int, delta: float, price: float, fee_rate: float, reason: str) -> None:
        if abs(delta) < 1e-12:
            return
        fee = abs(delta) * price * fee_rate
        realized, closed = 0.0, 0.0
        if self.qty and (self.qty > 0) != (delta > 0):  # reducing / closing / flipping
            closed = min(abs(delta), abs(self.qty))
            realized = closed * (price - self.entry) * (1 if self.qty > 0 else -1)
        new_qty = self.qty + delta
        if abs(new_qty) < 1e-12:
            new_qty, self.entry = 0.0, 0.0
        elif self.qty == 0 or (self.qty > 0) != (new_qty > 0):
            self.entry = price  # opened or flipped
        elif abs(new_qty) > abs(self.qty):
            self.entry = (self.entry * abs(self.qty) + price * abs(delta)) / abs(new_qty)
        self.cash -= delta * price + fee
        self.qty = new_qty
        self.fills.append(
            {
                "ts": ts,
                "side": "buy" if delta > 0 else "sell",
                "qty": abs(delta),
                "price": price,
                "fee": fee,
                "pnl": realized - fee,
                "reason": reason,
                "closed_qty": closed,
                "position_after": new_qty,
            }
        )


def _exposure(pf: Portfolio, price: float, leverage: float) -> float:
    eq = pf.equity(price)
    return (pf.qty * price) / (eq * leverage) if eq > 0 else 0.0


def run_backtest(strategy: Strategy, candles: list[Candle], cfg: BacktestConfig) -> dict:
    if len(candles) < 2:
        raise ValueError("Need at least 2 candles to backtest")
    data = Data.from_candles(candles)
    strategy.init(data)
    pf = Portfolio(cfg.initial_capital)
    equity_curve: list[list[float]] = []
    pending: float | None = None  # target exposure to execute at next open
    bars_in_market = 0

    for i in range(len(data)):
        ts, o, h, l, c = data.ts[i], data.open[i], data.high[i], data.low[i], data.close[i]

        # 1) execute last bar's signal at this bar's open
        if pending is not None:
            target = pending if cfg.allow_short else max(pending, 0.0)
            target = max(-1.0, min(1.0, target))
            eq = pf.equity(o)
            if eq > 0:
                desired_qty = target * eq * cfg.leverage / o
                delta = desired_qty - pf.qty
                if abs(delta) * o > eq * 1e-6:
                    fill = o * (1 + cfg.slippage) if delta > 0 else o * (1 - cfg.slippage)
                    pf.trade(ts, delta, fill, cfg.fee_rate, "signal")
            pending = None

        # 2) intrabar stop-loss / take-profit
        if pf.qty and (cfg.stop_loss or cfg.take_profit):
            long = pf.qty > 0
            sl = pf.entry * (1 - cfg.stop_loss if long else 1 + cfg.stop_loss) if cfg.stop_loss else None
            tp = pf.entry * (1 + cfg.take_profit if long else 1 - cfg.take_profit) if cfg.take_profit else None
            exit_px, reason = None, ""
            if sl is not None and ((long and l <= sl) or (not long and h >= sl)):
                exit_px, reason = (min(o, sl) if long else max(o, sl)), "stop_loss"
            elif tp is not None and ((long and h >= tp) or (not long and l <= tp)):
                exit_px, reason = (max(o, tp) if long else min(o, tp)), "take_profit"
            if exit_px is not None:
                fill = exit_px * (1 - cfg.slippage) if long else exit_px * (1 + cfg.slippage)
                pf.trade(ts, -pf.qty, fill, cfg.fee_rate, reason)

        if pf.qty:
            bars_in_market += 1
        eq = pf.equity(c)
        equity_curve.append([ts, round(eq, 6)])
        if eq <= 0:
            break  # account blown

        # 3) ask the strategy for a new target on bar close
        strategy.position = _exposure(pf, c, cfg.leverage)
        strategy.entry_price = pf.entry
        if i < len(data) - 1:
            signal = strategy.next(i, data)
            if signal is not None and not (isinstance(signal, float) and math.isnan(signal)):
                if abs(float(signal) - strategy.position) > 1e-3:
                    pending = float(signal)

    metrics = compute_metrics(equity_curve, pf.fills, cfg, data, bars_in_market)
    return {"metrics": metrics, "equity_curve": equity_curve, "trades": pf.fills, "config": asdict(cfg)}


def compute_metrics(equity_curve, fills, cfg: BacktestConfig, data: Data, bars_in_market: int) -> dict:
    eqs = [e for _, e in equity_curve]
    start, end = cfg.initial_capital, eqs[-1]
    peak, max_dd = eqs[0], 0.0
    for e in eqs:
        peak = max(peak, e)
        if peak > 0:
            max_dd = max(max_dd, (peak - e) / peak)
    rets = [(eqs[i] / eqs[i - 1] - 1) for i in range(1, len(eqs)) if eqs[i - 1] > 0]
    bar_ms = TIMEFRAME_MS.get(cfg.timeframe, 3_600_000)
    bars_per_year = 365 * 86_400_000 / bar_ms
    mean = sum(rets) / len(rets) if rets else 0.0
    var = sum((r - mean) ** 2 for r in rets) / (len(rets) - 1) if len(rets) > 1 else 0.0
    sd = math.sqrt(var)
    downside = [min(r, 0.0) for r in rets]
    dsd = math.sqrt(sum(d * d for d in downside) / len(downside)) if downside else 0.0
    years = len(eqs) / bars_per_year
    total_return = end / start - 1
    cagr = (end / start) ** (1 / years) - 1 if years > 0 and end > 0 else -1.0
    closing = [f for f in fills if f["closed_qty"] > 0]
    wins = [f["pnl"] for f in closing if f["pnl"] > 0]
    losses = [f["pnl"] for f in closing if f["pnl"] <= 0]
    gross_loss = -sum(losses)
    bh = data.close[len(eqs) - 1] / data.open[0] - 1
    return {
        "start_equity": round(start, 2),
        "end_equity": round(end, 2),
        "total_return_pct": round(total_return * 100, 2),
        "cagr_pct": round(cagr * 100, 2),
        "max_drawdown_pct": round(max_dd * 100, 2),
        "sharpe": round(mean / sd * math.sqrt(bars_per_year), 2) if sd else 0.0,
        "sortino": round(mean / dsd * math.sqrt(bars_per_year), 2) if dsd else 0.0,
        "calmar": round(cagr / max_dd, 2) if max_dd else 0.0,
        "num_fills": len(fills),
        "num_closed_trades": len(closing),
        "win_rate_pct": round(len(wins) / len(closing) * 100, 2) if closing else 0.0,
        "profit_factor": round(sum(wins) / gross_loss, 2) if gross_loss else None,
        "avg_win": round(sum(wins) / len(wins), 2) if wins else 0.0,
        "avg_loss": round(sum(losses) / len(losses), 2) if losses else 0.0,
        "total_fees": round(sum(f["fee"] for f in fills), 2),
        "open_position_pnl": round(end - start - sum(f["pnl"] for f in fills), 2),  # unrealized at the last bar
        "exposure_pct": round(bars_in_market / len(eqs) * 100, 2),
        "buy_hold_return_pct": round(bh * 100, 2),
        "bars": len(eqs),
    }
