#!/usr/bin/env python3
# -------------------------------------------------------------------------------------------------
#  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
#  https://nautechsystems.io
#
#  Licensed under the GNU Lesser General Public License Version 3.0 (the "License");
#  You may not use this file except in compliance with the License.
#  You may obtain a copy of the License at https://www.gnu.org/licenses/lgpl-3.0.en.html
#
#  Unless required by applicable law or agreed to in writing, software
#  distributed under the License is distributed on an "AS IS" BASIS,
#  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
#  See the License for the specific language governing permissions and
#  limitations under the License.
# -------------------------------------------------------------------------------------------------
"""
Post-crash rebound day-trading strategy for ADANIENT.NSE, backtested on Groww bars.

Rules (frozen 2026-08-29 after in-sample research on 2020-03..2024-12; the raw
effect replicated out-of-sample on 2025-01..2026-08):

- Signal: previous session's open-to-close return <= -4%.
- Entry:  market BUY at the close of the 09:45 IST bar the next session.
- Stop:   2.5% below entry (stop-market).
- Exit:   15:15 IST market close-out when the stop has not fired.
- Size:   risk 4% of equity against the stop, capped at 2x equity notional
          (MIS margin), whole shares.

Costs are Groww MIS brokerage and statutory charges via GrowwEquityFeeModel.
Expect few trades: the signal fires roughly 10-20 times per year.
"""

import csv
import json
import os
import sys
from decimal import Decimal
from pathlib import Path

import pandas as pd

from nautilus_trader.backtest import BacktestEngine
from nautilus_trader.config import BacktestEngineConfig
from nautilus_trader.config import StrategyConfig
from nautilus_trader.model import AccountType
from nautilus_trader.model import Bar
from nautilus_trader.model import BarType
from nautilus_trader.model import Currency
from nautilus_trader.model import Equity
from nautilus_trader.model import InstrumentId
from nautilus_trader.model import Money
from nautilus_trader.model import OmsType
from nautilus_trader.model import OrderSide
from nautilus_trader.model import Price
from nautilus_trader.model import Quantity
from nautilus_trader.model import Symbol
from nautilus_trader.model import TimeInForce
from nautilus_trader.model import TraderId
from nautilus_trader.model import Venue
from nautilus_trader.trading import Strategy


sys.path.insert(0, str(Path(__file__).resolve().parent))
from groww_fees import GrowwEquityFeeModel  # noqa: E402


DATA_DIR = Path(os.environ.get("GROWW_DATA_DIR", Path.home() / "groww_data" / "bars"))
SYMBOL = "ADANIENT"
IST = "Asia/Kolkata"


class ReboundConfig(StrategyConfig):
    _CUSTOM_FIELDS = ("instrument_id", "bar_type", "threshold_pct", "stop_pct", "risk_pct", "lev_cap")

    def __new__(cls, *args: object, **kwargs: object):
        for f in cls._CUSTOM_FIELDS:
            kwargs.pop(f, None)
        return super().__new__(cls, *args, **kwargs)

    def __init__(
        self,
        instrument_id: InstrumentId,
        bar_type: BarType,
        threshold_pct: float = -4.0,
        stop_pct: float = 2.5,
        risk_pct: float = 0.04,
        lev_cap: float = 2.0,
        **_kwargs: object,
    ) -> None:
        super().__init__()
        self.instrument_id = instrument_id
        self.bar_type = bar_type
        self.threshold_pct = threshold_pct
        self.stop_pct = stop_pct
        self.risk_pct = risk_pct
        self.lev_cap = lev_cap


class PostCrashRebound(Strategy):
    """
    Long-only post-crash intraday rebound with a protective stop and EOD flat.
    """

    def __init__(self, config: ReboundConfig) -> None:
        super().__init__(config)
        self._cur_date = None
        self._day_open = None
        self._last_close = None
        self._prev_ret = None
        self._armed = False       # signal active for today
        self._entry_px = None
        self._stop_px = None
        self._qty = 0

    def on_start(self) -> None:
        self.subscribe_bars(self.config.bar_type)

    def _equity(self) -> float:
        account = self.portfolio.account(self.config.instrument_id.venue)
        if account is None:
            return 0.0
        balance = account.balance_total(Currency.from_str("INR"))
        return float(balance.as_double()) if balance is not None else 0.0

    def on_bar(self, bar: Bar) -> None:
        ts = pd.Timestamp(bar.ts_event, unit="ns", tz="UTC").tz_convert(IST)
        date, hhmm = ts.date(), ts.hour * 60 + ts.minute

        if date != self._cur_date:
            # New session: yesterday's open->close return decides today's signal.
            if self._day_open is not None and self._last_close is not None:
                self._prev_ret = (self._last_close / self._day_open - 1.0) * 100.0
            self._cur_date = date
            self._day_open = float(bar.open)
            self._armed = (
                self._prev_ret is not None and self._prev_ret <= self.config.threshold_pct
            )
        self._last_close = float(bar.close)

        in_position = self._qty > 0 and not self.portfolio.is_net_flat(self.config.instrument_id)

        # Entry at the 09:45 bar close on signal days.
        if self._armed and hhmm == 9 * 60 + 45 and not in_position:
            self._armed = False
            equity = self._equity()
            px = float(bar.close)
            stop_dist = px * self.config.stop_pct / 100.0
            qty = int(min(
                equity * self.config.risk_pct / stop_dist,
                self.config.lev_cap * equity / px,
            ))
            if qty < 1:
                return
            self._qty = qty
            self._entry_px = px
            self._stop_px = px * (1.0 - self.config.stop_pct / 100.0)
            instrument = self.cache.instrument(self.config.instrument_id)
            self.submit_order(
                self.order_factory.market(
                    self.config.instrument_id,
                    OrderSide.BUY,
                    instrument.make_qty(qty),
                    time_in_force=TimeInForce.DAY,
                ),
            )
            return

        if not in_position:
            return

        # Stop: bar traded through the level -> exit immediately.
        if float(bar.low) <= self._stop_px or hhmm >= 15 * 60 + 15:
            instrument = self.cache.instrument(self.config.instrument_id)
            self.submit_order(
                self.order_factory.market(
                    self.config.instrument_id,
                    OrderSide.SELL,
                    instrument.make_qty(self._qty),
                    time_in_force=TimeInForce.DAY,
                    reduce_only=True,
                ),
            )
            self._qty = 0


def load_instrument(instrument_id: InstrumentId) -> Equity:
    records = json.loads((DATA_DIR / "instruments.json").read_text())
    for r in records:
        if r["instrument_id"] == str(instrument_id):
            return Equity(
                instrument_id=instrument_id,
                raw_symbol=Symbol(r["raw_symbol"]),
                currency=Currency.from_str("INR"),
                price_precision=r["price_precision"],
                price_increment=Price.from_str(r["price_increment"]),
                lot_size=Quantity.from_int(1),
                isin=r.get("isin"),
                ts_event=0,
                ts_init=0,
            )
    raise KeyError(instrument_id)


def load_bars(bar_type: BarType, instrument: Equity, path: Path) -> list[Bar]:
    precision = instrument.price_precision
    bars = []
    with path.open() as f:
        reader = csv.reader(f)
        next(reader)
        for row in reader:
            ts = int(pd.Timestamp(row[0]).value)
            bars.append(Bar(
                bar_type=bar_type,
                open=Price(float(row[1]), precision=precision),
                high=Price(float(row[2]), precision=precision),
                low=Price(float(row[3]), precision=precision),
                close=Price(float(row[4]), precision=precision),
                volume=Quantity(float(row[5]), precision=0),
                ts_event=ts,
                ts_init=ts,
            ))
    return bars


if __name__ == "__main__":
    engine = BacktestEngine(BacktestEngineConfig(trader_id=TraderId.from_str("BACKTESTER-001")))
    NSE = Venue("NSE")
    INR = Currency.from_str("INR")
    engine.add_venue(
        venue=NSE,
        oms_type=OmsType.NETTING,
        account_type=AccountType.MARGIN,
        base_currency=INR,
        starting_balances=[Money(10_000, INR)],
        default_leverage=Decimal(5),   # NSE MIS intraday margin
        fee_model=GrowwEquityFeeModel(product="MIS", exchange="NSE"),
    )
    instrument_id = InstrumentId.from_str(f"{SYMBOL}.NSE")
    instrument = load_instrument(instrument_id)
    engine.add_instrument(instrument)
    bar_type = BarType.from_str(f"{instrument_id}-5-MINUTE-LAST-EXTERNAL")
    bars = load_bars(bar_type, instrument, DATA_DIR / f"{SYMBOL}.NSE_5minute.csv")
    print(f"Loaded {len(bars):,} bars {bars[0].ts_event} .. {bars[-1].ts_event}")
    engine.add_data(bars)
    engine.add_strategy(PostCrashRebound(ReboundConfig(
        instrument_id=instrument_id,
        bar_type=bar_type,
    )))
    engine.run()

    account = engine.generate_account_report(NSE)
    fills = engine.generate_order_fills_report()
    positions = engine.generate_positions_report()
    pnl = positions["realized_pnl"].str.replace(" INR", "").astype(float)
    comm = positions["commissions"].map(lambda c: sum(float(x.replace(" INR", "")) for x in c))
    print("\n=== ENGINE RESULT (full 2020-03 .. 2026-08, Groww MIS fees) ===")
    print(f"trades (round trips): {len(positions)}")
    print(f"final balance       : {account.iloc[-1]['total']} INR (start 10,000)")
    print(f"total realized pnl  : {pnl.sum():+,.0f} INR | fees: {comm.sum():,.0f} INR")
    print(f"win rate            : {(pnl > 0).mean() * 100:.1f}%")
    print(f"best / worst trade  : {pnl.max():+,.0f} / {pnl.min():+,.0f} INR")
    engine.dispose()
