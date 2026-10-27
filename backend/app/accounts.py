"""Helpers to build exchange clients from stored (encrypted) accounts."""
from __future__ import annotations

from .exchanges import ExchangeClient, make_client
from .models import ExchangeAccount
from .security import decrypt


def client_for_account(account: ExchangeAccount, **kwargs) -> ExchangeClient:
    return make_client(
        account.exchange,
        api_key=decrypt(account.api_key_enc),
        api_secret=decrypt(account.api_secret_enc),
        passphrase=decrypt(account.passphrase_enc),
        testnet=account.testnet,
        **kwargs,
    )
