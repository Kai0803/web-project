class RsiReversion(Strategy):
    """RSI 均值回歸：RSI 低於超賣區且價格在長期均線之上時買進，RSI 回到中性區賣出。"""

    params = {"rsi_period": 14, "oversold": 30, "exit_level": 55, "trend_period": 200}

    def init(self, data):
        self.rsi = ta.rsi(data.close, self.p.rsi_period)
        self.trend = ta.sma(data.close, self.p.trend_period)

    def next(self, i, data):
        r, t = self.rsi[i], self.trend[i]
        if r is None or t is None:
            return None
        if self.position == 0 and r < self.p.oversold and data.close[i] > t:
            return 1.0
        if self.position > 0 and r > self.p.exit_level:
            return 0.0
        return None
