from __future__ import annotations

from fastapi import APIRouter, Depends, HTTPException
from pydantic import BaseModel
from sqlalchemy.orm import Session

from .. import settings_store
from ..accounts import client_for_account
from ..ai.gemini import GeminiClient, GeminiError
from ..db import get_session
from ..exchanges import EXCHANGE_INFO, TIMEFRAMES, ExchangeError, make_client
from ..models import ExchangeAccount, iso
from ..security import encrypt, mask

router = APIRouter(prefix="/api")


def gemini_for(db: Session, feature: str) -> GeminiClient:
    """Return a Gemini client if AI is enabled for `feature`, else raise 400."""
    if not settings_store.ai_allowed(db, feature):
        raise HTTPException(400, "Gemini AI 未啟用（請到「設定」開啟總開關與此模組，並填入 API Key）")
    values = settings_store.get_all(db)
    return GeminiClient(values["gemini_api_key"], values["gemini_model"], language=values["ai_language"])


@router.get("/meta")
def meta(db: Session = Depends(get_session)):
    values = settings_store.get_all(db)
    return {
        "exchanges": EXCHANGE_INFO,
        "timeframes": TIMEFRAMES,
        "ai": {f: settings_store.ai_allowed(db, f) for f in ("backtest", "trading", "portfolio")},
        "ai_model": values["gemini_model"],
    }


# ---- settings -------------------------------------------------------------
class SettingsIn(BaseModel):
    ai_enabled: bool | None = None
    ai_backtest: bool | None = None
    ai_trading: bool | None = None
    ai_portfolio: bool | None = None
    gemini_model: str | None = None
    gemini_api_key: str | None = None
    ai_language: str | None = None


@router.get("/settings")
def get_settings(db: Session = Depends(get_session)):
    values = settings_store.get_all(db)
    values["gemini_api_key"] = mask(values["gemini_api_key"]) if values["gemini_api_key"] else ""
    for k in settings_store.BOOL_KEYS:
        values[k] = values[k] == "true"
    return values


@router.put("/settings")
def put_settings(body: SettingsIn, db: Session = Depends(get_session)):
    updates = body.model_dump(exclude_none=True)
    if updates.get("gemini_api_key", "x") == "" or "…" in updates.get("gemini_api_key", ""):
        updates.pop("gemini_api_key", None)  # unchanged masked value
    settings_store.set_many(db, updates)
    return get_settings(db)


@router.post("/ai/test")
async def ai_test(db: Session = Depends(get_session)):
    values = settings_store.get_all(db)
    try:
        client = GeminiClient(values["gemini_api_key"], values["gemini_model"], language=values["ai_language"])
        text = await client.generate("用一句話回覆：連線成功。")
    except GeminiError as e:
        raise HTTPException(400, str(e))
    return {"ok": True, "reply": text}


# ---- exchange accounts ----------------------------------------------------
class AccountIn(BaseModel):
    name: str
    exchange: str
    api_key: str = ""
    api_secret: str = ""
    passphrase: str = ""
    testnet: bool = False
    enabled: bool = True


def account_out(a: ExchangeAccount) -> dict:
    from ..security import decrypt

    return {
        "id": a.id,
        "name": a.name,
        "exchange": a.exchange,
        "api_key": mask(decrypt(a.api_key_enc)) if a.api_key_enc else "",
        "has_secret": bool(a.api_secret_enc),
        "has_passphrase": bool(a.passphrase_enc),
        "testnet": a.testnet,
        "enabled": a.enabled,
        "created_at": iso(a.created_at),
    }


@router.get("/accounts")
def list_accounts(db: Session = Depends(get_session)):
    return [account_out(a) for a in db.query(ExchangeAccount).order_by(ExchangeAccount.id).all()]


@router.post("/accounts")
def create_account(body: AccountIn, db: Session = Depends(get_session)):
    if body.exchange not in EXCHANGE_INFO:
        raise HTTPException(400, "不支援的交易所")
    if EXCHANGE_INFO[body.exchange]["passphrase"] and body.api_key and not body.passphrase:
        raise HTTPException(400, f"{body.exchange} 需要 API Passphrase")
    acc = ExchangeAccount(
        name=body.name,
        exchange=body.exchange,
        api_key_enc=encrypt(body.api_key),
        api_secret_enc=encrypt(body.api_secret),
        passphrase_enc=encrypt(body.passphrase),
        testnet=body.testnet,
        enabled=body.enabled,
    )
    db.add(acc)
    db.commit()
    return account_out(acc)


@router.put("/accounts/{account_id}")
def update_account(account_id: int, body: AccountIn, db: Session = Depends(get_session)):
    acc = db.get(ExchangeAccount, account_id)
    if not acc:
        raise HTTPException(404)
    acc.name, acc.testnet, acc.enabled = body.name, body.testnet, body.enabled
    # blank / masked fields keep the stored secret
    if body.api_key and "…" not in body.api_key:
        acc.api_key_enc = encrypt(body.api_key)
    if body.api_secret:
        acc.api_secret_enc = encrypt(body.api_secret)
    if body.passphrase:
        acc.passphrase_enc = encrypt(body.passphrase)
    db.commit()
    return account_out(acc)


@router.delete("/accounts/{account_id}")
def delete_account(account_id: int, db: Session = Depends(get_session)):
    acc = db.get(ExchangeAccount, account_id)
    if not acc:
        raise HTTPException(404)
    db.delete(acc)
    db.commit()
    return {"ok": True}


@router.post("/accounts/{account_id}/test")
async def test_account(account_id: int, db: Session = Depends(get_session)):
    acc = db.get(ExchangeAccount, account_id)
    if not acc:
        raise HTTPException(404)
    try:
        async with client_for_account(acc) as client:
            balances = await client.fetch_balances()
    except ExchangeError as e:
        raise HTTPException(400, str(e))
    return {"ok": True, "balances": [b.to_dict() for b in balances]}


# ---- market data ------------------------------------------------------------
@router.get("/market/candles")
async def market_candles(exchange: str, symbol: str, market: str = "spot", timeframe: str = "1h", limit: int = 300):
    try:
        async with make_client(exchange) as client:
            candles = await client.fetch_candles(symbol, market, timeframe, limit=min(limit, 1500))
    except ExchangeError as e:
        raise HTTPException(400, str(e))
    return [c.to_dict() for c in candles]


@router.get("/market/price")
async def market_price(exchange: str, symbol: str, market: str = "spot"):
    try:
        async with make_client(exchange) as client:
            return {"price": await client.fetch_price(symbol, market)}
    except ExchangeError as e:
        raise HTTPException(400, str(e))
