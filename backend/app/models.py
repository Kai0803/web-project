from __future__ import annotations

from datetime import datetime, timezone

from sqlalchemy import JSON, Boolean, DateTime, Float, ForeignKey, Integer, String, Text
from sqlalchemy.orm import Mapped, mapped_column

from .db import Base


def utcnow() -> datetime:
    return datetime.now(timezone.utc)


class Setting(Base):
    """Key/value settings editable from the UI (e.g. Gemini key, AI toggles)."""

    __tablename__ = "settings"
    key: Mapped[str] = mapped_column(String(64), primary_key=True)
    value: Mapped[str] = mapped_column(Text, default="")


class ExchangeAccount(Base):
    __tablename__ = "exchange_accounts"
    id: Mapped[int] = mapped_column(Integer, primary_key=True)
    name: Mapped[str] = mapped_column(String(64))
    exchange: Mapped[str] = mapped_column(String(16))  # binance / bybit / okx / bitget / pionex
    api_key_enc: Mapped[str] = mapped_column(Text, default="")
    api_secret_enc: Mapped[str] = mapped_column(Text, default="")
    passphrase_enc: Mapped[str] = mapped_column(Text, default="")
    testnet: Mapped[bool] = mapped_column(Boolean, default=False)
    enabled: Mapped[bool] = mapped_column(Boolean, default=True)
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow)


class Strategy(Base):
    __tablename__ = "strategies"
    id: Mapped[int] = mapped_column(Integer, primary_key=True)
    name: Mapped[str] = mapped_column(String(128), unique=True)
    description: Mapped[str] = mapped_column(Text, default="")
    code: Mapped[str] = mapped_column(Text)
    params: Mapped[dict] = mapped_column(JSON, default=dict)
    updated_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, onupdate=utcnow)


class BacktestRun(Base):
    __tablename__ = "backtest_runs"
    id: Mapped[int] = mapped_column(Integer, primary_key=True)
    strategy_id: Mapped[int] = mapped_column(ForeignKey("strategies.id", ondelete="CASCADE"))
    config: Mapped[dict] = mapped_column(JSON)
    metrics: Mapped[dict] = mapped_column(JSON, default=dict)
    equity_curve: Mapped[list] = mapped_column(JSON, default=list)
    trades: Mapped[list] = mapped_column(JSON, default=list)
    ai_review: Mapped[str] = mapped_column(Text, default="")
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow)


class Bot(Base):
    """A strategy deployed against an exchange account."""

    __tablename__ = "bots"
    id: Mapped[int] = mapped_column(Integer, primary_key=True)
    name: Mapped[str] = mapped_column(String(128))
    strategy_id: Mapped[int] = mapped_column(ForeignKey("strategies.id"))
    account_id: Mapped[int | None] = mapped_column(ForeignKey("exchange_accounts.id"), nullable=True)
    exchange: Mapped[str] = mapped_column(String(16))
    symbol: Mapped[str] = mapped_column(String(32))  # unified, e.g. BTC/USDT
    market: Mapped[str] = mapped_column(String(8), default="spot")  # spot / swap
    timeframe: Mapped[str] = mapped_column(String(8), default="1h")
    params: Mapped[dict] = mapped_column(JSON, default=dict)
    mode: Mapped[str] = mapped_column(String(8), default="paper")  # paper / live
    capital: Mapped[float] = mapped_column(Float, default=1000.0)  # quote currency allocated
    leverage: Mapped[float] = mapped_column(Float, default=1.0)
    max_drawdown_stop: Mapped[float] = mapped_column(Float, default=0.3)  # auto-stop at -30%
    ai_guard: Mapped[bool] = mapped_column(Boolean, default=False)  # ask Gemini before each order
    status: Mapped[str] = mapped_column(String(16), default="stopped")  # running / stopped / error
    state: Mapped[dict] = mapped_column(JSON, default=dict)  # position, paper cash, last bar ts…
    last_error: Mapped[str] = mapped_column(Text, default="")
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow)


class BotLog(Base):
    __tablename__ = "bot_logs"
    id: Mapped[int] = mapped_column(Integer, primary_key=True)
    bot_id: Mapped[int] = mapped_column(ForeignKey("bots.id", ondelete="CASCADE"), index=True)
    level: Mapped[str] = mapped_column(String(8), default="info")
    message: Mapped[str] = mapped_column(Text)
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow)


class TradeRecord(Base):
    """Every order a bot (or the manual order panel) sent, live or paper."""

    __tablename__ = "trade_records"
    id: Mapped[int] = mapped_column(Integer, primary_key=True)
    bot_id: Mapped[int | None] = mapped_column(ForeignKey("bots.id", ondelete="SET NULL"), nullable=True, index=True)
    account_id: Mapped[int | None] = mapped_column(Integer, nullable=True)
    exchange: Mapped[str] = mapped_column(String(16))
    symbol: Mapped[str] = mapped_column(String(32))
    market: Mapped[str] = mapped_column(String(8))
    side: Mapped[str] = mapped_column(String(4))
    qty: Mapped[float] = mapped_column(Float)
    price: Mapped[float] = mapped_column(Float)
    fee: Mapped[float] = mapped_column(Float, default=0.0)
    mode: Mapped[str] = mapped_column(String(8))
    order_id: Mapped[str] = mapped_column(String(64), default="")
    note: Mapped[str] = mapped_column(Text, default="")
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow)


class BalanceSnapshot(Base):
    __tablename__ = "balance_snapshots"
    id: Mapped[int] = mapped_column(Integer, primary_key=True)
    account_id: Mapped[int] = mapped_column(ForeignKey("exchange_accounts.id", ondelete="CASCADE"), index=True)
    taken_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, index=True)
    total_usd: Mapped[float] = mapped_column(Float, default=0.0)
    balances: Mapped[list] = mapped_column(JSON, default=list)  # [{asset, free, locked, total, usd}]
    positions: Mapped[list] = mapped_column(JSON, default=list)  # [{symbol, side, size, entry, mark, upnl, leverage}]


class PositionEvent(Base):
    """A detected change between two snapshots (opened, increased, reduced, closed, flipped, balance change)."""

    __tablename__ = "position_events"
    id: Mapped[int] = mapped_column(Integer, primary_key=True)
    account_id: Mapped[int] = mapped_column(ForeignKey("exchange_accounts.id", ondelete="CASCADE"), index=True)
    kind: Mapped[str] = mapped_column(String(16))
    symbol: Mapped[str] = mapped_column(String(32))
    side: Mapped[str] = mapped_column(String(8), default="")
    size_before: Mapped[float] = mapped_column(Float, default=0.0)
    size_after: Mapped[float] = mapped_column(Float, default=0.0)
    price: Mapped[float] = mapped_column(Float, default=0.0)
    detail: Mapped[str] = mapped_column(Text, default="")
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utcnow, index=True)


def iso(dt: datetime | None) -> str | None:
    """ISO-8601 in UTC (SQLite returns naive datetimes)."""
    if dt is None:
        return None
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return dt.isoformat()
