from __future__ import annotations

from fastapi import APIRouter, Depends, HTTPException
from pydantic import BaseModel, Field
from sqlalchemy.orm import Session

from ..accounts import client_for_account
from ..ai.gemini import GeminiError, analyze_portfolio, review_bot
from ..db import get_session
from ..exchanges import EXCHANGE_INFO, ExchangeError
from ..exchanges.base import TIMEFRAME_MS
from ..models import BalanceSnapshot, Bot, BotLog, ExchangeAccount, PositionEvent, Strategy, TradeRecord, iso
from ..portfolio import service as portfolio
from ..trading.runner import add_log, init_state, manager
from .system import gemini_for

router = APIRouter(prefix="/api")


# ---- bots -------------------------------------------------------------------
class BotIn(BaseModel):
    name: str
    strategy_id: int
    account_id: int | None = None
    exchange: str = "binance"
    symbol: str = "BTC/USDT"
    market: str = "spot"
    timeframe: str = "1h"
    params: dict = Field(default_factory=dict)
    mode: str = "paper"
    capital: float = 1000.0
    leverage: float = 1.0
    max_drawdown_stop: float = 0.3
    ai_guard: bool = False


def bot_out(b: Bot, db: Session) -> dict:
    state = b.state or {}
    equity = None
    if state:
        equity = state.get("cash", 0.0) + state.get("qty", 0.0) * (state.get("last_price") or state.get("entry") or 0.0)
    strategy = db.get(Strategy, b.strategy_id)
    return {
        "id": b.id,
        "name": b.name,
        "strategy_id": b.strategy_id,
        "strategy_name": strategy.name if strategy else "(已刪除)",
        "account_id": b.account_id,
        "exchange": b.exchange,
        "symbol": b.symbol,
        "market": b.market,
        "timeframe": b.timeframe,
        "params": b.params,
        "mode": b.mode,
        "capital": b.capital,
        "leverage": b.leverage,
        "max_drawdown_stop": b.max_drawdown_stop,
        "ai_guard": b.ai_guard,
        "status": b.status,
        "running": manager.is_running(b.id),
        "state": state,
        "equity_estimate": equity,
        "last_error": b.last_error,
        "created_at": iso(b.created_at),
    }


def _validate_bot(body: BotIn, db: Session) -> None:
    if body.exchange not in EXCHANGE_INFO:
        raise HTTPException(400, "不支援的交易所")
    if body.timeframe not in TIMEFRAME_MS:
        raise HTTPException(400, "不支援的週期")
    if body.market == "swap" and not EXCHANGE_INFO[body.exchange]["swap"]:
        raise HTTPException(400, f"{body.exchange} 連接器僅支援現貨")
    if body.mode not in ("paper", "live"):
        raise HTTPException(400, "mode 必須是 paper 或 live")
    if not db.get(Strategy, body.strategy_id):
        raise HTTPException(400, "策略不存在")
    if body.mode == "live":
        acc = db.get(ExchangeAccount, body.account_id) if body.account_id else None
        if not acc or acc.exchange != body.exchange:
            raise HTTPException(400, "實盤模式需要選擇同一交易所的 API 帳戶")
    if body.capital <= 0 or not (1 <= body.leverage <= 125):
        raise HTTPException(400, "資金需 > 0，槓桿需介於 1~125")


@router.get("/bots")
def list_bots(db: Session = Depends(get_session)):
    return [bot_out(b, db) for b in db.query(Bot).order_by(Bot.id).all()]


@router.post("/bots")
def create_bot(body: BotIn, db: Session = Depends(get_session)):
    _validate_bot(body, db)
    bot = Bot(**body.model_dump())
    bot.state = init_state(bot)
    db.add(bot)
    db.commit()
    return bot_out(bot, db)


@router.put("/bots/{bot_id}")
def update_bot(bot_id: int, body: BotIn, db: Session = Depends(get_session)):
    bot = db.get(Bot, bot_id)
    if not bot:
        raise HTTPException(404)
    if manager.is_running(bot_id):
        raise HTTPException(400, "請先停止機器人再修改")
    _validate_bot(body, db)
    for k, v in body.model_dump().items():
        setattr(bot, k, v)
    db.commit()
    return bot_out(bot, db)


@router.delete("/bots/{bot_id}")
async def delete_bot(bot_id: int, db: Session = Depends(get_session)):
    bot = db.get(Bot, bot_id)
    if not bot:
        raise HTTPException(404)
    await manager.stop(bot_id)
    db.delete(bot)
    db.commit()
    return {"ok": True}


@router.post("/bots/{bot_id}/start")
def start_bot(bot_id: int, db: Session = Depends(get_session)):
    bot = db.get(Bot, bot_id)
    if not bot:
        raise HTTPException(404)
    bot.status, bot.last_error = "running", ""
    state = dict(bot.state or init_state(bot))
    state["errors"] = 0
    bot.state = state
    db.commit()
    add_log(db, bot, f"啟動（{'模擬' if bot.mode == 'paper' else '實盤'}模式）")
    manager.start(bot_id)
    return bot_out(bot, db)


@router.post("/bots/{bot_id}/stop")
async def stop_bot(bot_id: int, db: Session = Depends(get_session)):
    bot = db.get(Bot, bot_id)
    if not bot:
        raise HTTPException(404)
    bot.status = "stopped"
    db.commit()
    await manager.stop(bot_id)
    add_log(db, bot, "已停止（持倉維持不變，如需平倉請手動下單）")
    return bot_out(bot, db)


@router.post("/bots/{bot_id}/reset")
def reset_bot(bot_id: int, db: Session = Depends(get_session)):
    bot = db.get(Bot, bot_id)
    if not bot:
        raise HTTPException(404)
    if manager.is_running(bot_id):
        raise HTTPException(400, "請先停止機器人")
    bot.state = init_state(bot)
    db.commit()
    add_log(db, bot, "帳務狀態已重設")
    return bot_out(bot, db)


@router.get("/bots/{bot_id}/logs")
def bot_logs(bot_id: int, limit: int = 200, db: Session = Depends(get_session)):
    rows = db.query(BotLog).filter(BotLog.bot_id == bot_id).order_by(BotLog.id.desc()).limit(limit).all()
    return [{"id": r.id, "level": r.level, "message": r.message, "created_at": iso(r.created_at)} for r in rows]


def trade_out(t: TradeRecord) -> dict:
    return {
        "id": t.id, "bot_id": t.bot_id, "account_id": t.account_id, "exchange": t.exchange, "symbol": t.symbol,
        "market": t.market, "side": t.side, "qty": t.qty, "price": t.price, "fee": t.fee, "mode": t.mode,
        "order_id": t.order_id, "note": t.note, "created_at": iso(t.created_at),
    }


@router.get("/trades")
def list_trades(bot_id: int | None = None, limit: int = 300, db: Session = Depends(get_session)):
    q = db.query(TradeRecord).order_by(TradeRecord.id.desc())
    if bot_id:
        q = q.filter(TradeRecord.bot_id == bot_id)
    return [trade_out(t) for t in q.limit(limit).all()]


@router.post("/bots/{bot_id}/ai-review")
async def ai_review_bot(bot_id: int, db: Session = Depends(get_session)):
    bot = db.get(Bot, bot_id)
    if not bot:
        raise HTTPException(404)
    client = gemini_for(db, "trading")
    trades = [trade_out(t) for t in db.query(TradeRecord).filter(TradeRecord.bot_id == bot_id).order_by(TradeRecord.id).all()]
    logs = [f"{iso(r.created_at)} [{r.level}] {r.message}" for r in db.query(BotLog).filter(BotLog.bot_id == bot_id).order_by(BotLog.id).all()]
    info = bot_out(bot, db)
    try:
        return {"review": await review_bot(client, info, trades, logs)}
    except GeminiError as e:
        raise HTTPException(400, str(e))


# ---- manual orders ---------------------------------------------------------
class OrderIn(BaseModel):
    account_id: int
    symbol: str
    market: str = "spot"
    side: str
    qty: float
    order_type: str = "market"
    price: float | None = None
    reduce_only: bool = False


@router.post("/orders")
async def manual_order(body: OrderIn, db: Session = Depends(get_session)):
    acc = db.get(ExchangeAccount, body.account_id)
    if not acc:
        raise HTTPException(404, "帳戶不存在")
    if body.side not in ("buy", "sell") or body.qty <= 0:
        raise HTTPException(400, "方向或數量錯誤")
    try:
        async with client_for_account(acc) as client:
            qty = await client.normalize_qty(body.symbol, body.market, body.qty)
            if qty <= 0:
                raise HTTPException(400, "數量低於交易所最小下單量")
            res = await client.place_order(body.symbol, body.market, body.side, qty, body.order_type, body.price, body.reduce_only)
            price = res.price or body.price or await client.fetch_price(body.symbol, body.market)
    except ExchangeError as e:
        raise HTTPException(400, str(e))
    rec = TradeRecord(
        account_id=acc.id, exchange=acc.exchange, symbol=body.symbol, market=body.market, side=body.side, qty=qty,
        price=price, mode="manual", order_id=res.order_id, note=body.order_type,
    )
    db.add(rec)
    db.commit()
    return {"ok": True, "order_id": res.order_id, "trade": trade_out(rec)}


# ---- portfolio -------------------------------------------------------------
@router.post("/portfolio/snapshot")
async def take_snapshot(db: Session = Depends(get_session)):
    return await portfolio.snapshot_all(db)


@router.get("/portfolio/overview")
def portfolio_overview(db: Session = Depends(get_session)):
    return portfolio.overview(db)


@router.get("/portfolio/history")
def portfolio_history(days: int = 90, db: Session = Depends(get_session)):
    return portfolio.equity_history(db, days)


@router.get("/portfolio/events")
def portfolio_events(account_id: int | None = None, limit: int = 300, db: Session = Depends(get_session)):
    q = db.query(PositionEvent).order_by(PositionEvent.created_at.desc(), PositionEvent.id.desc())
    if account_id:
        q = q.filter(PositionEvent.account_id == account_id)
    names = {a.id: a.name for a in db.query(ExchangeAccount).all()}
    return [
        {
            "id": e.id, "account": names.get(e.account_id, "?"), "kind": e.kind, "symbol": e.symbol, "side": e.side,
            "size_before": e.size_before, "size_after": e.size_after, "price": e.price, "detail": e.detail,
            "created_at": iso(e.created_at),
        }
        for e in q.limit(limit).all()
    ]


@router.get("/portfolio/snapshots/{account_id}")
def account_snapshots(account_id: int, limit: int = 50, db: Session = Depends(get_session)):
    rows = (
        db.query(BalanceSnapshot).filter(BalanceSnapshot.account_id == account_id)
        .order_by(BalanceSnapshot.taken_at.desc()).limit(limit).all()
    )
    return [{"id": s.id, "taken_at": iso(s.taken_at), "total_usd": s.total_usd, "balances": s.balances, "positions": s.positions} for s in rows]


@router.post("/portfolio/ai-analyze")
async def ai_portfolio(db: Session = Depends(get_session)):
    client = gemini_for(db, "portfolio")
    ov = portfolio.overview(db)
    hist = portfolio.equity_history(db, 30)
    summary = {
        "overview": ov,
        "equity_30d": [{"t": h["t"], "usd": h["total_usd"]} for h in hist[-60:]],
        "recent_events": portfolio_events(None, 40, db),
        "bots": [
            {k: v for k, v in bot_out(b, db).items() if k in ("name", "exchange", "symbol", "market", "mode", "capital", "leverage", "status")}
            for b in db.query(Bot).all()
        ],
    }
    try:
        return {"review": await analyze_portfolio(client, summary)}
    except GeminiError as e:
        raise HTTPException(400, str(e))
