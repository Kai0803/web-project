from __future__ import annotations

import itertools
from datetime import datetime, timezone

from fastapi import APIRouter, Depends, HTTPException
from pydantic import BaseModel, Field
from sqlalchemy.orm import Session

from ..ai.gemini import GeminiError, analyze_backtest, review_strategy
from ..backtest.data import load_exchange_candles, parse_csv, synthetic_candles
from ..backtest.engine import BacktestConfig, run_backtest
from ..backtest.strategy import StrategyLoadError, load_strategy_class
from ..db import get_session
from ..exchanges import Candle, ExchangeError
from ..models import BacktestRun, Strategy, iso
from .system import gemini_for

router = APIRouter(prefix="/api")


# ---- strategies -----------------------------------------------------------
class StrategyIn(BaseModel):
    name: str
    description: str = ""
    code: str
    params: dict = Field(default_factory=dict)


def strategy_out(s: Strategy, with_code: bool = True) -> dict:
    out = {"id": s.id, "name": s.name, "description": s.description, "params": s.params, "updated_at": iso(s.updated_at)}
    if with_code:
        out["code"] = s.code
    return out


def validate_code(code: str) -> dict:
    try:
        cls = load_strategy_class(code)
    except StrategyLoadError as e:
        raise HTTPException(400, f"策略程式錯誤: {e}")
    return dict(cls.params)


@router.get("/strategies")
def list_strategies(db: Session = Depends(get_session)):
    return [strategy_out(s, False) for s in db.query(Strategy).order_by(Strategy.name).all()]


@router.get("/strategies/{sid}")
def get_strategy(sid: int, db: Session = Depends(get_session)):
    s = db.get(Strategy, sid)
    if not s:
        raise HTTPException(404)
    return strategy_out(s)


@router.post("/strategies")
def create_strategy(body: StrategyIn, db: Session = Depends(get_session)):
    defaults = validate_code(body.code)
    if db.query(Strategy).filter(Strategy.name == body.name).first():
        raise HTTPException(400, "策略名稱已存在")
    s = Strategy(name=body.name, description=body.description, code=body.code, params={**defaults, **body.params})
    db.add(s)
    db.commit()
    return strategy_out(s)


@router.put("/strategies/{sid}")
def update_strategy(sid: int, body: StrategyIn, db: Session = Depends(get_session)):
    s = db.get(Strategy, sid)
    if not s:
        raise HTTPException(404)
    defaults = validate_code(body.code)
    s.name, s.description, s.code = body.name, body.description, body.code
    s.params = {k: body.params.get(k, v) for k, v in defaults.items()}
    db.commit()
    return strategy_out(s)


@router.delete("/strategies/{sid}")
def delete_strategy(sid: int, db: Session = Depends(get_session)):
    s = db.get(Strategy, sid)
    if not s:
        raise HTTPException(404)
    db.delete(s)
    db.commit()
    return {"ok": True}


class CodeIn(BaseModel):
    code: str
    note: str = ""


@router.post("/strategies/validate")
def validate_strategy(body: CodeIn):
    return {"ok": True, "params": validate_code(body.code)}


@router.post("/strategies/ai-review")
async def ai_review_strategy(body: CodeIn, db: Session = Depends(get_session)):
    client = gemini_for(db, "backtest")
    try:
        return {"review": await review_strategy(client, body.code, body.note)}
    except GeminiError as e:
        raise HTTPException(400, str(e))


# ---- backtests -----------------------------------------------------------
class BacktestIn(BaseModel):
    strategy_id: int
    params: dict = Field(default_factory=dict)
    source: str = "exchange"  # exchange / csv / synthetic
    exchange: str = "binance"
    symbol: str = "BTC/USDT"
    market: str = "spot"
    timeframe: str = "1h"
    start: str | None = None  # ISO date
    end: str | None = None
    csv: str = ""
    initial_capital: float = 10_000
    fee_rate: float = 0.001
    slippage: float = 0.0005
    leverage: float = 1.0
    stop_loss: float = 0.0
    take_profit: float = 0.0
    ai: bool = False  # ask Gemini to analyze the result


def _to_ms(value: str | None, default: datetime) -> int:
    dt = datetime.fromisoformat(value) if value else default
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return int(dt.timestamp() * 1000)


async def _load_candles(body: BacktestIn) -> list[Candle]:
    if body.source == "csv":
        try:
            candles = parse_csv(body.csv)
        except (ValueError, StopIteration, IndexError) as e:
            raise HTTPException(400, f"CSV 格式錯誤: {e}")
    elif body.source == "synthetic":
        candles = synthetic_candles(3000, body.timeframe)
    else:
        now = datetime.now(timezone.utc)
        end = _to_ms(body.end, now)
        start = _to_ms(body.start, now.replace(year=now.year - 1))
        if start >= end:
            raise HTTPException(400, "開始日期需早於結束日期")
        try:
            candles = await load_exchange_candles(body.exchange, body.symbol, body.market, body.timeframe, start, end)
        except ExchangeError as e:
            raise HTTPException(400, f"下載歷史資料失敗: {e}")
    if len(candles) < 30:
        raise HTTPException(400, f"資料太少（{len(candles)} 根 K 線）")
    return candles


def _config(body: BacktestIn) -> BacktestConfig:
    return BacktestConfig(
        initial_capital=body.initial_capital,
        fee_rate=body.fee_rate,
        slippage=body.slippage,
        leverage=body.leverage if body.market == "swap" else 1.0,
        allow_short=body.market == "swap",
        stop_loss=body.stop_loss,
        take_profit=body.take_profit,
        timeframe=body.timeframe,
    )


def _run(strategy: Strategy, params: dict, candles: list[Candle], cfg: BacktestConfig) -> dict:
    cls = load_strategy_class(strategy.code)
    try:
        return run_backtest(cls(**{**(strategy.params or {}), **params}), candles, cfg)
    except Exception as e:  # noqa: BLE001 - user strategy runtime error
        raise HTTPException(400, f"回測執行錯誤: {type(e).__name__}: {e}")


def run_out(r: BacktestRun, full: bool = True) -> dict:
    out = {"id": r.id, "strategy_id": r.strategy_id, "config": r.config, "metrics": r.metrics, "created_at": iso(r.created_at), "ai_review": r.ai_review}
    if full:
        out.update(equity_curve=r.equity_curve, trades=r.trades)
    return out


@router.post("/backtests")
async def create_backtest(body: BacktestIn, db: Session = Depends(get_session)):
    strategy = db.get(Strategy, body.strategy_id)
    if not strategy:
        raise HTTPException(404, "策略不存在")
    candles = await _load_candles(body)
    cfg = _config(body)
    result = _run(strategy, body.params, candles, cfg)
    meta = body.model_dump(exclude={"csv", "ai"})
    meta.update(params={**(strategy.params or {}), **body.params}, first_ts=candles[0].ts, last_ts=candles[-1].ts)
    run = BacktestRun(
        strategy_id=strategy.id,
        config=meta,
        metrics=result["metrics"],
        equity_curve=result["equity_curve"],
        trades=result["trades"],
    )
    db.add(run)
    db.commit()
    out = run_out(run)
    out["candles"] = [[c.ts, c.open, c.high, c.low, c.close] for c in candles]
    if body.ai:
        try:
            client = gemini_for(db, "backtest")
            run.ai_review = await analyze_backtest(client, strategy.code, meta, run.metrics, run.trades)
            db.commit()
            out["ai_review"] = run.ai_review
        except (GeminiError, HTTPException) as e:
            out["ai_error"] = getattr(e, "detail", str(e))
    return out


@router.get("/backtests")
def list_backtests(strategy_id: int | None = None, db: Session = Depends(get_session)):
    q = db.query(BacktestRun).order_by(BacktestRun.created_at.desc())
    if strategy_id:
        q = q.filter(BacktestRun.strategy_id == strategy_id)
    return [run_out(r, False) for r in q.limit(100).all()]


@router.get("/backtests/{rid}")
def get_backtest(rid: int, db: Session = Depends(get_session)):
    r = db.get(BacktestRun, rid)
    if not r:
        raise HTTPException(404)
    return run_out(r)


@router.delete("/backtests/{rid}")
def delete_backtest(rid: int, db: Session = Depends(get_session)):
    r = db.get(BacktestRun, rid)
    if not r:
        raise HTTPException(404)
    db.delete(r)
    db.commit()
    return {"ok": True}


@router.post("/backtests/{rid}/ai-analyze")
async def ai_analyze_backtest(rid: int, db: Session = Depends(get_session)):
    r = db.get(BacktestRun, rid)
    if not r:
        raise HTTPException(404)
    strategy = db.get(Strategy, r.strategy_id)
    client = gemini_for(db, "backtest")
    try:
        r.ai_review = await analyze_backtest(client, strategy.code if strategy else "", r.config, r.metrics, r.trades)
    except GeminiError as e:
        raise HTTPException(400, str(e))
    db.commit()
    return {"review": r.ai_review}


class OptimizeIn(BacktestIn):
    grid: dict[str, list] = Field(default_factory=dict)  # {"fast": [5, 10, 20], "slow": [30, 50]}
    sort_by: str = "sharpe"


@router.post("/backtests/optimize")
async def optimize(body: OptimizeIn, db: Session = Depends(get_session)):
    """Grid-search parameters. Beware of overfitting: confirm winners on out-of-sample dates."""
    strategy = db.get(Strategy, body.strategy_id)
    if not strategy:
        raise HTTPException(404, "策略不存在")
    keys = list(body.grid)
    combos = list(itertools.product(*(body.grid[k] for k in keys)))
    if not combos or len(combos) > 300:
        raise HTTPException(400, "參數組合需介於 1 到 300 組")
    candles = await _load_candles(body)
    cfg = _config(body)
    rows = []
    for combo in combos:
        params = {**body.params, **dict(zip(keys, combo))}
        res = _run(strategy, params, candles, cfg)
        rows.append({"params": params, "metrics": res["metrics"]})
    rows.sort(key=lambda r: -(r["metrics"].get(body.sort_by) or 0))
    return {"results": rows, "bars": len(candles)}
