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
Stream Groww market data with the built-in DataTester actor.

Running this example connects to Groww and subscribes quotes, trades, and depth for
RELIANCE on the NSE, logging all received data. No orders are placed.

Reads GROWW_API_KEY and GROWW_API_SECRET from the environment (or a `.env` file loaded
by your shell). Data flows only during NSE market hours (09:15-15:30 IST, Mon-Fri).

"""

from __future__ import annotations

from nautilus_trader.adapters.groww import GROWW
from nautilus_trader.adapters.groww import GrowwDataClientConfig
from nautilus_trader.adapters.groww import GrowwDataClientFactory
from nautilus_trader.common import Environment
from nautilus_trader.live import LiveNode
from nautilus_trader.model import BarType
from nautilus_trader.model import ClientId
from nautilus_trader.model import InstrumentId
from nautilus_trader.model import TraderId
from nautilus_trader.testkit import DataTesterConfig


TRADER_ID = TraderId.from_str("TESTER-001")
INSTRUMENT_ID = InstrumentId.from_str("RELIANCE.NSE")
BAR_TYPE = BarType.from_str(f"{INSTRUMENT_ID}-5-MINUTE-LAST-EXTERNAL")


def main() -> None:
    """
    Run the example.
    """
    node = (
        LiveNode.builder("GROWW-DATA-TESTER-001", TRADER_ID, Environment.LIVE)
        .add_data_client(
            None,
            GrowwDataClientFactory(),
            GrowwDataClientConfig(
                exchanges=["NSE"],
                segments=["CASH"],
                instrument_types=["EQ"],
            ),
        )
        .build()
    )
    node.add_builtin_actor(
        "DataTester",
        DataTesterConfig(
            client_id=ClientId.from_str(GROWW),
            instrument_ids=[INSTRUMENT_ID],
            bar_types=[BAR_TYPE],
            subscribe_quotes=True,
            subscribe_trades=True,
            subscribe_book_depth=True,
            request_bars=True,
            log_data=True,
        ),
    )

    node.run()


if __name__ == "__main__":
    main()
