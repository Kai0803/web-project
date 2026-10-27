"""Minimal Gemini API client (REST generateContent) and the platform's AI helpers."""
from __future__ import annotations

import json
import re

import httpx

API_ROOT = "https://generativelanguage.googleapis.com/v1beta"


class GeminiError(Exception):
    pass


class GeminiClient:
    def __init__(self, api_key: str, model: str, transport: httpx.AsyncBaseTransport | None = None, language: str = "繁體中文"):
        if not api_key:
            raise GeminiError("尚未設定 Gemini API Key")
        self.api_key = api_key
        self.model = model
        self.language = language
        self._transport = transport

    async def generate(self, prompt: str, system: str = "", json_mode: bool = False, temperature: float = 0.4) -> str:
        body: dict = {
            "contents": [{"role": "user", "parts": [{"text": prompt}]}],
            "generationConfig": {"temperature": temperature},
        }
        sys_text = (system + "\n" if system else "") + f"Always answer in {self.language}."
        body["systemInstruction"] = {"parts": [{"text": sys_text}]}
        if json_mode:
            body["generationConfig"]["responseMimeType"] = "application/json"
        async with httpx.AsyncClient(timeout=90, transport=self._transport) as client:
            try:
                resp = await client.post(
                    f"{API_ROOT}/models/{self.model}:generateContent",
                    headers={"x-goog-api-key": self.api_key, "Content-Type": "application/json"},
                    json=body,
                )
            except httpx.HTTPError as e:
                raise GeminiError(f"Gemini 連線失敗: {e!r}") from e
        if resp.status_code >= 400:
            raise GeminiError(f"Gemini HTTP {resp.status_code}: {resp.text[:400]}")
        data = resp.json()
        try:
            parts = data["candidates"][0]["content"]["parts"]
        except (KeyError, IndexError) as e:
            raise GeminiError(f"Gemini 回應沒有內容: {json.dumps(data)[:400]}") from e
        return "".join(p.get("text", "") for p in parts if not p.get("thought"))

    async def generate_json(self, prompt: str, system: str = "") -> dict:
        text = await self.generate(prompt, system, json_mode=True, temperature=0.2)
        match = re.search(r"\{.*\}", text, re.S)
        try:
            return json.loads(match.group(0) if match else text)
        except json.JSONDecodeError as e:
            raise GeminiError(f"Gemini 回傳的不是 JSON: {text[:300]}") from e


QUANT_SYSTEM = (
    "You are a senior crypto quantitative trader and risk manager reviewing a user's personal trading system. "
    "Be concrete and critical; point out overfitting, look-ahead bias, fee/slippage sensitivity and risk of ruin. "
    "Never promise profits. Use Markdown."
)

STRATEGY_API_DOC = """Strategy API: class X(Strategy) with `params` dict, `init(self, data)` to precompute indicators
(data.open/high/low/close/volume/ts are lists) and `next(self, i, data)` returning target exposure in [-1, 1]
or None to hold. Helpers: ta.sma, ta.ema, ta.rsi, ta.macd, ta.bollinger, ta.atr, ta.highest, ta.lowest,
ta.crossover(a, b, i), ta.crossunder(a, b, i). self.p.<name> reads params; self.position / self.entry_price are available."""


async def review_strategy(client: GeminiClient, code: str, note: str = "") -> str:
    prompt = (
        f"{STRATEGY_API_DOC}\n\nReview this strategy. List weaknesses, then propose concrete improvements, "
        f"and finish with a complete improved version of the code in a ```python block using the same API.\n"
        f"User note: {note or '-'}\n\n```python\n{code}\n```"
    )
    return await client.generate(prompt, QUANT_SYSTEM)


async def analyze_backtest(client: GeminiClient, code: str, cfg: dict, metrics: dict, trades: list[dict]) -> str:
    sample = trades[-40:]
    prompt = (
        f"{STRATEGY_API_DOC}\n\nAnalyze this backtest. Explain what the metrics say, whether the edge looks real or "
        f"overfit, how to validate it (walk-forward / out-of-sample / parameter sensitivity), and specific parameter or "
        f"logic changes to try next (with reasoning). If useful, include an improved code version.\n\n"
        f"Config: {json.dumps(cfg)}\nMetrics: {json.dumps(metrics)}\nLast fills ({len(sample)} of {len(trades)}): "
        f"{json.dumps(sample)}\n\n```python\n{code}\n```"
    )
    return await client.generate(prompt, QUANT_SYSTEM)


async def guard_trade(client: GeminiClient, context: dict) -> dict:
    """Ask Gemini to veto or approve an order a bot is about to send. Returns {approve, reason}."""
    prompt = (
        "A trading bot is about to change its position. Approve unless there is a clear, specific risk reason "
        "(e.g. abnormal volatility, oversized exposure, contradicting obvious data). Reply as JSON "
        '{"approve": true|false, "reason": "<one or two sentences>"}.\n\n' + json.dumps(context)
    )
    result = await client.generate_json(prompt, QUANT_SYSTEM)
    return {"approve": bool(result.get("approve", True)), "reason": str(result.get("reason", ""))}


async def review_bot(client: GeminiClient, bot: dict, trades: list[dict], logs: list[str]) -> str:
    prompt = (
        "Review this live/paper trading bot's recent behaviour and results. Compare to what its backtest would "
        "imply, flag execution problems, and suggest improvements or whether to pause it.\n\n"
        f"Bot: {json.dumps(bot)}\nRecent trades: {json.dumps(trades[-50:])}\nRecent logs:\n" + "\n".join(logs[-60:])
    )
    return await client.generate(prompt, QUANT_SYSTEM)


async def analyze_portfolio(client: GeminiClient, summary: dict) -> str:
    prompt = (
        "Analyze this multi-exchange crypto portfolio: concentration, exchange counterparty risk, leverage and "
        "liquidation risk, stablecoin ratio, and recent changes. Give a prioritized list of concrete rebalancing / "
        "risk-management suggestions.\n\n" + json.dumps(summary, ensure_ascii=False)
    )
    return await client.generate(prompt, QUANT_SYSTEM)
