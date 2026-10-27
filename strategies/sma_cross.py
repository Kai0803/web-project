class SmaCross(Strategy):
    """雙均線交叉：快線上穿慢線做多，下穿平倉（永續合約可設 allow_short 做空）。"""

    params = {"fast": 20, "slow": 60, "short": False}

    def init(self, data):
        self.fast = ta.sma(data.close, self.p.fast)
        self.slow = ta.sma(data.close, self.p.slow)

    def next(self, i, data):
        if ta.crossover(self.fast, self.slow, i):
            return 1.0
        if ta.crossunder(self.fast, self.slow, i):
            return -1.0 if self.p.short else 0.0
        return None
