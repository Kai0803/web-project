"""FastAPI entry point: `python -m app.main` from the backend/ directory."""
from __future__ import annotations

import asyncio
import logging
from contextlib import asynccontextmanager

from fastapi import FastAPI
from fastapi.responses import FileResponse
from fastapi.staticfiles import StaticFiles

from . import config
from .backtest.strategy import StrategyLoadError, load_strategy_class
from .db import SessionLocal, init_db
from .models import Strategy
from .portfolio.service import snapshot_loop
from .routers import backtest, system, trading
from .trading.runner import manager

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s: %(message)s")
FRONTEND_DIR = config.ROOT_DIR / "frontend"


def seed_strategies() -> None:
    """Import strategies/*.py that are not in the DB yet (by file name)."""
    if not config.STRATEGY_DIR.exists():
        return
    with SessionLocal() as db:
        for path in sorted(config.STRATEGY_DIR.glob("*.py")):
            name = path.stem
            if db.query(Strategy).filter(Strategy.name == name).first():
                continue
            code = path.read_text(encoding="utf-8")
            try:
                cls = load_strategy_class(code)
            except StrategyLoadError as e:
                logging.warning("skip strategy %s: %s", path.name, e)
                continue
            doc = (cls.__doc__ or "").strip().splitlines()
            db.add(Strategy(name=name, description=doc[0] if doc else "", code=code, params=dict(cls.params)))
        db.commit()


@asynccontextmanager
async def lifespan(app: FastAPI):
    init_db()
    seed_strategies()
    manager.resume_running()
    snapshot_task = asyncio.create_task(snapshot_loop())
    yield
    snapshot_task.cancel()
    await manager.stop_all()


app = FastAPI(title="Personal Trading Platform", lifespan=lifespan)
app.include_router(system.router)
app.include_router(backtest.router)
app.include_router(trading.router)

if FRONTEND_DIR.exists():
    app.mount("/static", StaticFiles(directory=FRONTEND_DIR), name="static")

    @app.get("/", include_in_schema=False)
    def index():
        return FileResponse(FRONTEND_DIR / "index.html")


if __name__ == "__main__":
    import uvicorn

    uvicorn.run("app.main:app", host=config.HOST, port=config.PORT)
