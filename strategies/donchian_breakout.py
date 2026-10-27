class DonchianBreakout(Strategy):
    """唐奇安通道突破 + ATR 波動調整倉位：突破 N 根高點做多、跌破 M 根低點出場。"""

    params = {"entry": 55, "exit": 20, "atr_period": 20, "risk_per_trade": 0.02, "atr_stop": 2.0}

    def init(self, data):
        self.upper = ta.highest(data.high, self.p.entry)
        self.lower = ta.lowest(data.low, self.p.exit)
        self.atr = ta.atr(data.high, data.low, data.close, self.p.atr_period)

    def next(self, i, data):
        if i < 1 or self.upper[i - 1] is None or self.lower[i - 1] is None or self.atr[i] is None:
            return None
        price = data.close[i]
        if self.position == 0 and price > self.upper[i - 1]:
            # size so that an `atr_stop` x ATR move costs `risk_per_trade` of equity
            stop_distance = self.p.atr_stop * self.atr[i] / price
            return min(1.0, self.p.risk_per_trade / stop_distance) if stop_distance > 0 else 1.0
        if self.position > 0 and price < self.lower[i - 1]:
            return 0.0
        return None
