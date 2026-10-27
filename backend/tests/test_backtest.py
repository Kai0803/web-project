import pytest

from app.backtest import indicators as ta
from app.backtest.data import parse_csv, synthetic_candles
from app.backtest.engine import BacktestConfig, run_backtest
from app.backtest.strategy import Strategy, StrategyLoadError, load_strategy_class
from app.exchanges import Candle


def test_indicators_basic():
    vals = [1, 2, 3, 4, 5, 6]
    assert ta.sma(vals, 3) == [None, None, 2, 3, 4, 5]
    e = ta.ema(vals, 3)
    assert e[:2] == [None, None] and e[2] == 2 and e[3] == pytest.approx(3)
    r = ta.rsi([1, 2, 3, 4, 5, 6, 7], 3)
    assert r[3] == 100.0
    assert ta.highest(vals, 2)[1:3] == [2, 3]
    a, b = [1, 2, 3], [2, 2, 2]
    assert ta.crossover(a, b, 2) and not ta.crossover(a, b, 1)
    mid, up, low = ta.bollinger([2, 2, 2, 2], 2)
    assert mid[-1] == 2 and up[-1] == 2 and low[-1] == 2
    atr = ta.atr([2, 3, 4], [1, 1, 2], [1.5, 2, 3], 2)
    assert atr[0] is None and atr[1] == pytest.approx(1.5)  # TR = [1, 2]


def candles_from_closes(closes):
    return [Candle(i * 3_600_000, c, c, c, c, 1) for i, c in enumerate(closes)]


class AlwaysLong(Strategy):
    def next(self, i, data):
        return 1.0


def test_engine_buy_and_hold_no_costs():
    candles = candles_from_closes([100, 110, 121, 133.1])
    res = run_backtest(AlwaysLong(), candles, BacktestConfig(initial_capital=1000, fee_rate=0, slippage=0))
    # signal on bar 0 close, filled at bar 1 open (110); end close 133.1
    assert res["metrics"]["end_equity"] == pytest.approx(1000 * 133.1 / 110, rel=1e-6)
    assert res["trades"][0]["ts"] == candles[1].ts


def test_engine_fees_and_short_and_stop_loss():
    class ShortThenFlat(Strategy):
        def next(self, i, data):
            return -1.0 if i == 0 else None

    closes = [100, 100, 90, 80]
    cfg = BacktestConfig(initial_capital=1000, fee_rate=0.001, slippage=0, allow_short=True)
    res = run_backtest(ShortThenFlat(), candles_from_closes(closes), cfg)
    assert res["metrics"]["end_equity"] > 1150  # short gained ~20%, minus fee

    spot = run_backtest(ShortThenFlat(), candles_from_closes(closes), BacktestConfig(allow_short=False))
    assert spot["metrics"]["num_fills"] == 0  # shorts are clamped to flat on spot

    falling = [Candle(0, 100, 100, 100, 100, 1), Candle(1, 100, 100, 100, 100, 1), Candle(2, 100, 101, 90, 92, 1), Candle(3, 92, 92, 92, 92, 1)]
    sl = run_backtest(AlwaysLong(), falling, BacktestConfig(initial_capital=1000, fee_rate=0, slippage=0, stop_loss=0.05))
    reasons = [t["reason"] for t in sl["trades"]]
    assert "stop_loss" in reasons
    stop_fill = next(t for t in sl["trades"] if t["reason"] == "stop_loss")
    assert stop_fill["price"] == pytest.approx(95)


def test_engine_metrics_are_json_safe_and_consistent():
    import json

    code = open("../strategies/sma_cross.py", encoding="utf-8").read()
    cls = load_strategy_class(code)
    res = run_backtest(cls(), synthetic_candles(1500), BacktestConfig())
    json.dumps(res, allow_nan=False)
    m = res["metrics"]
    realized = sum(t["pnl"] for t in res["trades"])
    assert m["end_equity"] - m["start_equity"] == pytest.approx(realized + m["open_position_pnl"], abs=0.05)
    assert 0 <= m["max_drawdown_pct"] <= 100


def test_all_example_strategies_load_and_run():
    from pathlib import Path

    for path in Path("../strategies").glob("*.py"):
        cls = load_strategy_class(path.read_text(encoding="utf-8"))
        res = run_backtest(cls(), synthetic_candles(800), BacktestConfig())
        assert res["metrics"]["bars"] == 800, path.name


def test_strategy_load_errors():
    with pytest.raises(StrategyLoadError):
        load_strategy_class("x = 1")
    with pytest.raises(StrategyLoadError, match="SyntaxError"):
        load_strategy_class("class A(Strategy):\n  def next(self")


def test_param_overrides_only_known_keys():
    cls = load_strategy_class("class A(Strategy):\n    params = {'n': 5}\n    def next(self, i, d):\n        return None\n")
    s = cls(n=9, unknown=1)
    assert s.p.n == 9 and not hasattr(s.p, "unknown")


def test_parse_csv_formats():
    text = "date,open,high,low,close,volume\n2024-01-01 00:00:00,1,2,0.5,1.5,10\n2024-01-01 01:00:00,1.5,2,1,1.8,5\n"
    c = parse_csv(text)
    assert len(c) == 2 and c[0].ts == 1704067200000 and c[1].close == 1.8
    c2 = parse_csv("1704067200,1,2,0.5,1.5\n1704070800,1,2,0.5,1.6\n")
    assert c2[0].ts == 1704067200000 and c2[1].volume == 0
