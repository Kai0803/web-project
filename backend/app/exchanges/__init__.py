from .base import TIMEFRAMES, Balance, Candle, ExchangeClient, ExchangeError, OrderResult, Position
from .binance import BinanceClient
from .bitget import BitgetClient
from .bybit import BybitClient
from .okx import OKXClient
from .pionex import PionexClient

EXCHANGES: dict[str, type[ExchangeClient]] = {
    "binance": BinanceClient,
    "bybit": BybitClient,
    "okx": OKXClient,
    "bitget": BitgetClient,
    "pionex": PionexClient,
}

EXCHANGE_INFO = {
    "binance": {"label": "Binance", "api": "Spot /api/v3 · USDⓈ-M Futures /fapi", "passphrase": False, "swap": True},
    "bybit": {"label": "Bybit", "api": "V5 Unified (/v5)", "passphrase": False, "swap": True},
    "okx": {"label": "OKX", "api": "V5 (/api/v5)", "passphrase": True, "swap": True},
    "bitget": {"label": "Bitget", "api": "UTA V3 (/api/v3)", "passphrase": True, "swap": True},
    "pionex": {"label": "Pionex", "api": "Open API v1 (/api/v1)", "passphrase": False, "swap": False},
}


def make_client(exchange: str, **kwargs) -> ExchangeClient:
    try:
        cls = EXCHANGES[exchange]
    except KeyError:
        raise ExchangeError(f"Unsupported exchange {exchange!r}")
    return cls(**kwargs)


__all__ = [
    "TIMEFRAMES", "Balance", "Candle", "ExchangeClient", "ExchangeError", "OrderResult", "Position",
    "EXCHANGES", "EXCHANGE_INFO", "make_client",
]
