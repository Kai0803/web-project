"""Live / paper execution of strategies.

Each running bot is an asyncio task. On every newly *closed* candle it rebuilds the
strategy on the latest `lookback` bars, asks for the target exposure, and trades the
difference. Bot-level accounting (cash, qty, entry) lives in `bot.state`, so several
bots can share one exchange account with separate capital allocations.
"""
from __future__ import annotations

import asyncio
import logging
import time
from collections.abc import Callable

from sqlalchemy.orm import Session

from .. import settings_store
from ..accounts import client_for_account
from ..ai.gemini import GeminiClient, GeminiError, guard_trade
from ..backtest.strategy import Data, load_strategy_class
from ..db import SessionLocal
from ..exchanges import ExchangeClient, ExchangeError, make_client
from ..exchanges.base import TIMEFRAME_MS
from ..models import Bot, BotLog, ExchangeAccount, Strategy, TradeRecord

log = logging.getLogger(__name__)

PAPER_FEE = 0.001
PAPER_SLIPPAGE = 0.0005
MAX_CONSECUTIVE_ERRORS = 5
BAR_CLOSE_GRACE_MS = 5_000

ClientFactory = Callable[[Bot, Session], ExchangeClient]


def default_client_factory(bot: Bot, db: Session) -> ExchangeClient:
    if bot.mode == "live" or bot.account_id:
        account = db.get(ExchangeAccount, bot.account_id) if bot.account_id else None
        if account is None:
            if bot.mode == "live":
                raise ExchangeError("Live bot has no exchange account")
            return make_client(bot.exchange)
        return client_for_account(account)
    return make_client(bot.exchange)


def add_log(db: Session, bot: Bot, message: str, level: str = "info") -> None:
    db.add(BotLog(bot_id=bot.id, level=level, message=message[:2000]))
    db.commit()


def init_state(bot: Bot) -> dict:
    return {"cash": bot.capital, "qty": 0.0, "entry": 0.0, "last_bar_ts": 0, "peak_equity": bot.capital, "errors": 0}


async def step(bot: Bot, db: Session, client: ExchangeClient, now_ms: int | None = None) -> str:
    """Process the latest closed candle once. Returns a short status string (for logs/tests)."""
    state = dict(bot.state or init_state(bot))
    strategy_row = db.get(Strategy, bot.strategy_id)
    if strategy_row is None:
        raise RuntimeError("strategy deleted")
    cls = load_strategy_class(strategy_row.code)
    strat = cls(**{**(strategy_row.params or {}), **(bot.params or {})})
    tf = TIMEFRAME_MS[bot.timeframe]
    now = now_ms or int(time.time() * 1000)

    candles = await client.fetch_candles(bot.symbol, bot.market, bot.timeframe, limit=max(strat.lookback, 50) + 1)
    candles = [c for c in candles if c.ts + tf <= now]  # closed bars only
    if not candles:
        return "no-data"
    last = candles[-1]
    if last.ts <= state.get("last_bar_ts", 0):
        return "no-new-bar"
    state["last_bar_ts"] = last.ts

    price = last.close
    try:
        price = await client.fetch_price(bot.symbol, bot.market)
    except ExchangeError:
        pass
    state["last_price"] = price
    equity = state["cash"] + state["qty"] * price
    state["peak_equity"] = max(state.get("peak_equity", equity), equity)
    lev = bot.leverage if bot.market == "swap" else 1.0

    data = Data.from_candles(candles)
    strat.position = (state["qty"] * price) / (equity * lev) if equity > 0 else 0.0
    strat.entry_price = state["entry"]
    strat.init(data)
    signal = strat.next(len(data) - 1, data)

    stop_reason = ""
    if bot.max_drawdown_stop and equity < bot.capital * (1 - bot.max_drawdown_stop):
        signal, stop_reason = 0.0, f"資金回撤超過 {bot.max_drawdown_stop:.0%}，自動平倉並停止"

    result = "hold"
    if signal is not None:
        target = max(-1.0, min(1.0, float(signal)))
        if bot.market == "spot":
            target = max(target, 0.0)
        desired_qty = target * max(equity, 0.0) * lev / price
        delta = desired_qty - state["qty"]
        if abs(delta) * price >= max(equity * 0.002, 1.0):
            result = await execute(bot, db, client, state, delta, price, target, data)
    bot.state = state
    if stop_reason:
        add_log(db, bot, stop_reason, "warn")
        bot.status = "stopped"
    db.commit()
    return result


async def execute(bot: Bot, db: Session, client: ExchangeClient, state: dict, delta: float, price: float, target: float, data: Data) -> str:
    side = "buy" if delta > 0 else "sell"
    qty = abs(delta)
    current = state["qty"]
    reducing = current != 0 and (current > 0) != (delta > 0) and qty <= abs(current) + 1e-12

    if bot.ai_guard and settings_store.ai_allowed(db, "trading"):
        values = settings_store.get_all(db)
        try:
            gem = GeminiClient(values["gemini_api_key"], values["gemini_model"], language=values["ai_language"])
            verdict = await guard_trade(
                gem,
                {
                    "symbol": bot.symbol,
                    "market": bot.market,
                    "side": side,
                    "qty": qty,
                    "price": price,
                    "current_position_qty": current,
                    "target_exposure": target,
                    "leverage": bot.leverage,
                    "recent_closes": data.close[-30:],
                },
            )
        except GeminiError as e:
            verdict = {"approve": True, "reason": f"AI 無法使用，照常執行 ({e})"}
        add_log(db, bot, f"AI 守門: {'通過' if verdict['approve'] else '否決'} — {verdict['reason']}", "ai")
        if not verdict["approve"]:
            return "vetoed"

    if bot.mode == "paper":
        fill = price * (1 + PAPER_SLIPPAGE) if side == "buy" else price * (1 - PAPER_SLIPPAGE)
        fee = qty * fill * PAPER_FEE
        order_id = f"paper-{int(time.time() * 1000)}"
    else:
        if bot.market == "spot" and side == "sell":
            # Fees may have been charged in the base coin, so never sell more than is actually free.
            base = bot.symbol.split("/")[0].upper()
            free = sum(b.free for b in await client.fetch_balances() if b.asset.upper() == base)
            qty = min(qty, free)
        qty = await client.normalize_qty(bot.symbol, bot.market, qty)
        if qty <= 0:
            add_log(db, bot, f"下單數量低於交易所最小值，略過 ({side} {abs(delta):.8f})", "warn")
            return "too-small"
        if bot.market == "swap" and not state.get("leverage_set"):
            try:
                await client.set_leverage(bot.symbol, bot.leverage)
                state["leverage_set"] = True
            except ExchangeError as e:
                add_log(db, bot, f"設定槓桿失敗: {e}", "warn")
        order = await client.place_order(bot.symbol, bot.market, side, qty, "market", reduce_only=reducing and bot.market == "swap")
        fill = order.price or price
        fee = qty * fill * PAPER_FEE  # estimate; actual fee depends on your VIP tier
        order_id = order.order_id

    signed = qty if side == "buy" else -qty
    new_qty = current + signed
    if abs(new_qty) < 1e-12:
        new_qty, state["entry"] = 0.0, 0.0
    elif current == 0 or (current > 0) != (new_qty > 0):
        state["entry"] = fill
    elif abs(new_qty) > abs(current):
        state["entry"] = (state["entry"] * abs(current) + fill * qty) / abs(new_qty)
    state["cash"] -= signed * fill + fee
    state["qty"] = new_qty

    db.add(
        TradeRecord(
            bot_id=bot.id,
            account_id=bot.account_id,
            exchange=bot.exchange,
            symbol=bot.symbol,
            market=bot.market,
            side=side,
            qty=qty,
            price=fill,
            fee=fee,
            mode=bot.mode,
            order_id=order_id,
            note=f"target={target:+.2f}",
        )
    )
    add_log(db, bot, f"{'[模擬]' if bot.mode == 'paper' else '[實盤]'} {side.upper()} {qty:.8g} {bot.symbol} @ {fill:.8g} → 目標曝險 {target:+.2f}")
    return side


class BotManager:
    def __init__(self, client_factory: ClientFactory = default_client_factory) -> None:
        self.tasks: dict[int, asyncio.Task] = {}
        self.client_factory = client_factory

    def is_running(self, bot_id: int) -> bool:
        task = self.tasks.get(bot_id)
        return task is not None and not task.done()

    def start(self, bot_id: int) -> None:
        if self.is_running(bot_id):
            return
        self.tasks[bot_id] = asyncio.create_task(self._loop(bot_id), name=f"bot-{bot_id}")

    async def stop(self, bot_id: int) -> None:
        task = self.tasks.pop(bot_id, None)
        if task and not task.done():
            task.cancel()
            try:
                await task
            except (asyncio.CancelledError, Exception):  # noqa: BLE001
                pass

    async def stop_all(self) -> None:
        for bot_id in list(self.tasks):
            await self.stop(bot_id)

    def resume_running(self) -> None:
        with SessionLocal() as db:
            for bot in db.query(Bot).filter(Bot.status == "running").all():
                self.start(bot.id)

    async def _loop(self, bot_id: int) -> None:
        while True:
            with SessionLocal() as db:
                bot = db.get(Bot, bot_id)
                if bot is None or bot.status != "running":
                    return
                tf = TIMEFRAME_MS[bot.timeframe]
                try:
                    client = self.client_factory(bot, db)
                    async with client:
                        result = await step(bot, db, client)
                    state = dict(bot.state)
                    state["errors"] = 0
                    bot.state = state
                    bot.last_error = ""
                    db.commit()
                    log.debug("bot %s step: %s", bot_id, result)
                except Exception as e:  # noqa: BLE001 - keep the bot alive, surface the error
                    db.rollback()
                    bot = db.get(Bot, bot_id)
                    state = dict(bot.state or init_state(bot))
                    state["errors"] = state.get("errors", 0) + 1
                    bot.state = state
                    bot.last_error = f"{type(e).__name__}: {e}"
                    add_log(db, bot, f"錯誤: {bot.last_error}", "error")
                    if state["errors"] >= MAX_CONSECUTIVE_ERRORS:
                        bot.status = "error"
                        add_log(db, bot, f"連續 {MAX_CONSECUTIVE_ERRORS} 次錯誤，機器人已停止", "error")
                    db.commit()
                    if bot.status != "running":
                        return
                    await asyncio.sleep(60)  # retry soon after an error
                    continue
                if bot.status != "running":
                    return
            now = int(time.time() * 1000)
            next_close = (now // tf + 1) * tf + BAR_CLOSE_GRACE_MS
            await asyncio.sleep(min(max((next_close - now) / 1000, 5), 3600))


manager = BotManager()
