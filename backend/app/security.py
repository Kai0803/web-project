"""Encryption of exchange credentials at rest (Fernet / AES-128-CBC + HMAC)."""
from __future__ import annotations

from cryptography.fernet import Fernet

from . import config

_fernet: Fernet | None = None


def _get_fernet() -> Fernet:
    global _fernet
    if _fernet is None:
        key = config.MASTER_KEY
        if not key:
            if config.MASTER_KEY_FILE.exists():
                key = config.MASTER_KEY_FILE.read_text().strip()
            else:
                key = Fernet.generate_key().decode()
                config.MASTER_KEY_FILE.write_text(key)
                config.MASTER_KEY_FILE.chmod(0o600)
        _fernet = Fernet(key.encode())
    return _fernet


def encrypt(value: str) -> str:
    if not value:
        return ""
    return _get_fernet().encrypt(value.encode()).decode()


def decrypt(value: str) -> str:
    if not value:
        return ""
    return _get_fernet().decrypt(value.encode()).decode()


def mask(value: str) -> str:
    if len(value) <= 8:
        return "*" * len(value)
    return f"{value[:4]}…{value[-4:]}"
