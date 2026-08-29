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
EMA cross backtest on NSE equity bars downloaded from Groww.

Expects the data produced by the `groww-download-bars` Rust example:

    cargo run --release -p nautilus-groww --features nautilus-live/node \
        --example groww-download-bars

which writes per-symbol CSVs and an `instruments.json` under `~/groww_data/bars`
(override with `GROWW_DATA_DIR`). Bars are stamped at close time in UTC, so the
engine never sees a bar before the interval it summarizes has finished.
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
from nautilus_trader.model import AccountType
from nautilus_trader.model import Bar
from nautilus_trader.model import BarType
from nautilus_trader.model import Currency
from nautilus_trader.model import Equity
from nautilus_trader.model import InstrumentId
from nautilus_trader.model import Money
from nautilus_trader.model import OmsType
from nautilus_trader.model import Price
from nautilus_trader.model import Quantity
from nautilus_trader.model import Symbol
from nautilus_trader.model import TraderId
from nautilus_trader.model import Venue


sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "docs" / "tutorials"))

from ema_cross import EMACross  # noqa: E402
from ema_cross import EMACrossConfig  # noqa: E402
from groww_fees import GrowwEquityFeeModel  # noqa: E402


DATA_DIR = Path(os.environ.get("GROWW_DATA_DIR", Path.home() / "groww_data" / "bars"))
SYMBOL = os.environ.get("GROWW_SYMBOL", "RELIANCE")
INTERVAL = os.environ.get("GROWW_INTERVAL", "5minute")
INTERVAL_TO_BAR_SPEC = {
    "1minute": "1-MINUTE",
    "5minute": "5-MINUTE",
    "15minute": "15-MINUTE",
    "1hour": "1-HOUR",
    "1day": "1-DAY",
}


def load_instrument(instrument_id: InstrumentId) -> Equity:
    """
    Build the Equity from the metadata written by the downloader.
    """
    records = json.loads((DATA_DIR / "instruments.json").read_text())
    for record in records:
        if record["instrument_id"] == str(instrument_id):
            return Equity(
                instrument_id=instrument_id,
                raw_symbol=Symbol(record["raw_symbol"]),
                currency=Currency.from_str(record["currency"]),
                price_precision=record["price_precision"],
                price_increment=Price.from_str(record["price_increment"]),
                lot_size=Quantity.from_int(record["lot_size"]),
                isin=record.get("isin"),
                ts_event=0,
                ts_init=0,
            )
    raise KeyError(f"{instrument_id} not found in {DATA_DIR / 'instruments.json'}")


def load_bars(bar_type: BarType, instrument: Equity, path: Path) -> list[Bar]:
    """
    Build Bars from a downloader CSV, which stamps rows at close time in UTC.
    """
    precision = instrument.price_precision
    bars: list[Bar] = []
    with path.open() as f:
        reader = csv.reader(f)
        header = next(reader)
        if header != ["timestamp_utc", "open", "high", "low", "close", "volume"]:
            raise ValueError(f"Unexpected CSV header: {header}")
        for row in reader:
            ts_ns = int(pd.Timestamp(row[0]).value)
            bars.append(
                Bar(
                    bar_type=bar_type,
                    open=Price(float(row[1]), precision=precision),
                    high=Price(float(row[2]), precision=precision),
                    low=Price(float(row[3]), precision=precision),
                    close=Price(float(row[4]), precision=precision),
                    volume=Quantity(float(row[5]), precision=0),
                    ts_event=ts_ns,
                    ts_init=ts_ns,
                ),
            )
    return bars


if __name__ == "__main__":
    engine = BacktestEngine(
        BacktestEngineConfig(trader_id=TraderId.from_str("BACKTESTER-001")),
    )

    NSE = Venue("NSE")
    INR = Currency.from_str("INR")
    # A margin account so the demonstration strategy can hold both directions;
    # a delivery (CNC) cash account cannot carry an overnight short.
    # MIS (intraday) fee treatment matches this strategy's rapid in-and-out churn;
    # switch to product="CNC" for delivery-style costing with STT on both sides.
    engine.add_venue(
        venue=NSE,
        oms_type=OmsType.NETTING,
        account_type=AccountType.MARGIN,
        base_currency=INR,
        starting_balances=[Money(1_000_000, INR)],
        fee_model=GrowwEquityFeeModel(product="MIS", exchange="NSE"),
    )

    instrument_id = InstrumentId.from_str(f"{SYMBOL}.NSE")
    instrument = load_instrument(instrument_id)
    engine.add_instrument(instrument)

    spec = INTERVAL_TO_BAR_SPEC[INTERVAL]
    bar_type = BarType.from_str(f"{instrument_id}-{spec}-LAST-EXTERNAL")
    csv_path = DATA_DIR / f"{SYMBOL}.NSE_{INTERVAL}.csv"
    bars = load_bars(bar_type, instrument, csv_path)
    print(f"Loaded {len(bars):,} bars from {csv_path}")
    engine.add_data(bars)

    strategy = EMACross(
        EMACrossConfig(
            instrument_id=instrument_id,
            bar_type=bar_type,
            trade_size=Decimal(10),
            fast_ema_period=10,
            slow_ema_period=20,
        ),
    )
    engine.add_strategy(strategy)
    engine.run()

    with pd.option_context(
        "display.max_rows",
        100,
        "display.max_columns",
        None,
        "display.width",
        300,
    ):
        print(engine.generate_account_report(NSE))
        print(engine.generate_order_fills_report())
        print(engine.generate_positions_report())

    engine.reset()
    engine.dispose()
