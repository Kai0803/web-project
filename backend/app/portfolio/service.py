"""Multi-exchange balance/position snapshots and change history."""
from __future__ import annotations

import asyncio
import logging
from collections import defaultdict
from datetime import datetime, timezone

from sqlalchemy.orm import Session

from .. import config
from ..accounts import client_for_account
from ..db import SessionLocal
from ..exchanges import Balance, ExchangeClient, ExchangeError, Position
from ..models import BalanceSnapshot, ExchangeAccount, PositionEvent, iso

log = logging.getLogger(__name__)

STABLES = {"USDT", "USDC", "USD", "FDUSD", "BUSD", "DAI", "TUSD", "USDE", "PYUSD"}
BALANCE_CHANGE_THRESHOLD = 0.01  # record balance moves above 1%


async def value_balances(client: ExchangeClient, balances: list[Balance]) -> list[dict]:
    """Attach a USD value to each balance (exchange-reported, stablecoin = 1, else spot price vs USDT)."""
    prices: dict[str, float] = {}
    out = []
    for b in balances:
        usd = b.usd
        asset = b.asset.upper()
        if usd is None:
            if asset in STABLES:
                usd = b.total
            else:
                if asset not in prices:
                    try:
                        prices[asset] = await client.fetch_price(f"{asset}/USDT", "spot")
                    except ExchangeError:
                        prices[asset] = 0.0
                usd = b.total * prices[asset]
        out.append({**b.to_dict(), "usd": round(usd, 4)})
    return out


def diff_snapshots(prev: BalanceSnapshot | None, cur: BalanceSnapshot) -> list[PositionEvent]:
    events: list[PositionEvent] = []
    if prev is None:
        return events
    old_pos = {(p["symbol"], p["side"]): p for p in prev.positions or []}
    new_pos = {(p["symbol"], p["side"]): p for p in cur.positions or []}
    for key in old_pos.keys() | new_pos.keys():
        symbol, side = key
        before = old_pos.get(key, {}).get("size", 0.0)
        after = new_pos.get(key, {}).get("size", 0.0)
        if abs(after - before) <= max(before, after) * 1e-6:
            continue
        if before == 0:
            kind = "open"
        elif after == 0:
            kind = "close"
        elif after > before:
            kind = "increase"
        else:
            kind = "reduce"
        ref = new_pos.get(key) or old_pos.get(key) or {}
        detail = ""
        if kind == "close":
            detail = f"平倉前未實現損益 {old_pos[key].get('unrealized_pnl', 0):.2f}"
        events.append(
            PositionEvent(
                account_id=cur.account_id,
                kind=kind,
                symbol=symbol,
                side=side,
                size_before=before,
                size_after=after,
                price=ref.get("mark_price") or ref.get("entry_price") or 0.0,
                detail=detail,
                created_at=cur.taken_at,
            )
        )
    old_bal = {(b["asset"], b.get("wallet", "")): b for b in prev.balances or []}
    new_bal = {(b["asset"], b.get("wallet", "")): b for b in cur.balances or []}
    for key in old_bal.keys() | new_bal.keys():
        before = old_bal.get(key, {}).get("total", 0.0)
        after = new_bal.get(key, {}).get("total", 0.0)
        base = max(abs(before), abs(after))
        if base == 0 or abs(after - before) / base < BALANCE_CHANGE_THRESHOLD:
            continue
        usd_after = new_bal.get(key, {}).get("usd", 0.0)
        events.append(
            PositionEvent(
                account_id=cur.account_id,
                kind="balance",
                symbol=key[0],
                side=key[1],
                size_before=before,
                size_after=after,
                detail=f"{after - before:+.8g} (現值約 ${usd_after:,.2f})",
                created_at=cur.taken_at,
            )
        )
    return events


async def snapshot_account(db: Session, account: ExchangeAccount, client: ExchangeClient | None = None) -> BalanceSnapshot:
    own = client is None
    client = client or client_for_account(account)
    try:
        balances = await client.fetch_balances()
        valued = await value_balances(client, balances)
        positions: list[Position] = []
        if client.supports_swap:
            try:
                positions = await client.fetch_positions()
            except ExchangeError as e:
                log.warning("positions failed for %s: %s", account.name, e)
    finally:
        if own:
            await client.close()
    upnl = sum(p.unrealized_pnl for p in positions)
    snap = BalanceSnapshot(
        account_id=account.id,
        taken_at=datetime.now(timezone.utc),
        total_usd=round(sum(b["usd"] for b in valued) + upnl, 2),
        balances=valued,
        positions=[p.to_dict() for p in positions],
    )
    prev = (
        db.query(BalanceSnapshot)
        .filter(BalanceSnapshot.account_id == account.id)
        .order_by(BalanceSnapshot.taken_at.desc())
        .first()
    )
    db.add(snap)
    for ev in diff_snapshots(prev, snap):
        db.add(ev)
    db.commit()
    return snap


async def snapshot_all(db: Session) -> dict:
    results = {}
    for account in db.query(ExchangeAccount).filter(ExchangeAccount.enabled.is_(True)).all():
        try:
            snap = await snapshot_account(db, account)
            results[account.name] = {"ok": True, "total_usd": snap.total_usd}
        except Exception as e:  # noqa: BLE001
            db.rollback()
            results[account.name] = {"ok": False, "error": str(e)}
    return results


def latest_snapshots(db: Session) -> dict[int, BalanceSnapshot]:
    out = {}
    for account in db.query(ExchangeAccount).all():
        snap = (
            db.query(BalanceSnapshot)
            .filter(BalanceSnapshot.account_id == account.id)
            .order_by(BalanceSnapshot.taken_at.desc())
            .first()
        )
        if snap:
            out[account.id] = snap
    return out


def overview(db: Session) -> dict:
    accounts = {a.id: a for a in db.query(ExchangeAccount).all()}
    snaps = latest_snapshots(db)
    by_asset: dict[str, dict] = defaultdict(lambda: {"total": 0.0, "usd": 0.0, "exchanges": {}})
    by_exchange = []
    positions = []
    for acc_id, snap in snaps.items():
        acc = accounts[acc_id]
        by_exchange.append(
            {"account_id": acc_id, "name": acc.name, "exchange": acc.exchange, "total_usd": snap.total_usd, "taken_at": iso(snap.taken_at)}
        )
        for b in snap.balances:
            entry = by_asset[b["asset"]]
            entry["total"] += b["total"]
            entry["usd"] += b["usd"]
            entry["exchanges"][acc.name] = entry["exchanges"].get(acc.name, 0.0) + b["total"]
        for p in snap.positions:
            positions.append({**p, "account": acc.name, "exchange": acc.exchange})
    total = sum(e["total_usd"] for e in by_exchange)
    assets = sorted(
        ({"asset": k, **v, "pct": round(v["usd"] / total * 100, 2) if total else 0.0} for k, v in by_asset.items()),
        key=lambda a: -a["usd"],
    )
    return {"total_usd": round(total, 2), "by_exchange": by_exchange, "assets": assets, "positions": positions}


def equity_history(db: Session, days: int = 90) -> list[dict]:
    """Total USD across accounts over time (each account carries its last known value forward)."""
    since = datetime.now(timezone.utc).timestamp() - days * 86400
    snaps = (
        db.query(BalanceSnapshot)
        .filter(BalanceSnapshot.taken_at >= datetime.fromtimestamp(since, timezone.utc))
        .order_by(BalanceSnapshot.taken_at)
        .all()
    )
    last: dict[int, float] = {}
    series = []
    for s in snaps:
        last[s.account_id] = s.total_usd
        series.append({"t": iso(s.taken_at), "total_usd": round(sum(last.values()), 2), "by_account": dict(last)})
    return series


async def snapshot_loop() -> None:
    if config.SNAPSHOT_INTERVAL <= 0:
        return
    while True:
        await asyncio.sleep(config.SNAPSHOT_INTERVAL)
        try:
            with SessionLocal() as db:
                await snapshot_all(db)
        except Exception:  # noqa: BLE001
            log.exception("portfolio snapshot failed")
