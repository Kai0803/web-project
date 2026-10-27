"""Settings persisted in the DB (Gemini key, AI switches)."""
from __future__ import annotations

from sqlalchemy.orm import Session

from . import config
from .models import Setting
from .security import decrypt, encrypt

DEFAULTS = {
    "ai_enabled": "false",  # master switch
    "ai_backtest": "true",  # suggestions in the backtest module
    "ai_trading": "true",  # bot reviews / AI trade guard
    "ai_portfolio": "true",  # portfolio risk analysis
    "gemini_model": config.GEMINI_MODEL,
    "gemini_api_key": "",
    "ai_language": "繁體中文",
}
SECRET_KEYS = {"gemini_api_key"}
BOOL_KEYS = {"ai_enabled", "ai_backtest", "ai_trading", "ai_portfolio"}


def get_all(db: Session) -> dict[str, str]:
    values = dict(DEFAULTS)
    for row in db.query(Setting).all():
        values[row.key] = decrypt(row.value) if row.key in SECRET_KEYS else row.value
    if not values["gemini_api_key"] and config.GEMINI_API_KEY:
        values["gemini_api_key"] = config.GEMINI_API_KEY
    return values


def get(db: Session, key: str) -> str:
    return get_all(db)[key]


def get_bool(db: Session, key: str) -> bool:
    return get(db, key).lower() == "true"


def set_many(db: Session, updates: dict[str, str | bool]) -> None:
    for key, value in updates.items():
        if key not in DEFAULTS:
            continue
        if key in BOOL_KEYS:
            value = "true" if value in (True, "true", "1", 1) else "false"
        value = str(value)
        stored = encrypt(value) if key in SECRET_KEYS else value
        row = db.get(Setting, key)
        if row:
            row.value = stored
        else:
            db.add(Setting(key=key, value=stored))
    db.commit()


def ai_allowed(db: Session, feature: str) -> bool:
    """feature: backtest / trading / portfolio"""
    values = get_all(db)
    return values["ai_enabled"] == "true" and values.get(f"ai_{feature}") == "true" and bool(values["gemini_api_key"])
