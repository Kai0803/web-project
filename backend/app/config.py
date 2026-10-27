"""Runtime configuration, read from environment variables (or a .env file)."""
from __future__ import annotations

import os
from pathlib import Path

BASE_DIR = Path(__file__).resolve().parent.parent
ROOT_DIR = BASE_DIR.parent


def _load_dotenv() -> None:
    env_file = ROOT_DIR / ".env"
    if not env_file.exists():
        return
    for line in env_file.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        os.environ.setdefault(key.strip(), value.strip().strip('"').strip("'"))


_load_dotenv()

DATA_DIR = Path(os.environ.get("TP_DATA_DIR", ROOT_DIR / "data"))
DATA_DIR.mkdir(parents=True, exist_ok=True)

DATABASE_URL = os.environ.get("TP_DATABASE_URL", f"sqlite:///{DATA_DIR / 'trading.db'}")

# Fernet key used to encrypt exchange API secrets at rest. Generated on first run if absent.
MASTER_KEY = os.environ.get("TP_MASTER_KEY", "")
MASTER_KEY_FILE = DATA_DIR / ".master.key"

STRATEGY_DIR = Path(os.environ.get("TP_STRATEGY_DIR", ROOT_DIR / "strategies"))

# How often (seconds) the portfolio snapshotter polls connected exchanges. 0 disables it.
SNAPSHOT_INTERVAL = int(os.environ.get("TP_SNAPSHOT_INTERVAL", "900"))

# Gemini defaults; can be overridden from the Settings page (stored in DB).
GEMINI_API_KEY = os.environ.get("GEMINI_API_KEY", "")
GEMINI_MODEL = os.environ.get("GEMINI_MODEL", "gemini-flash-latest")

HOST = os.environ.get("TP_HOST", "127.0.0.1")
PORT = int(os.environ.get("TP_PORT", "8000"))
