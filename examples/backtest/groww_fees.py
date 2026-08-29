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
Fee model for Indian equity trades through Groww.

Implements the brokerage and statutory charges Groww publishes for NSE/BSE cash
equities, so a backtest carries realistic Indian trading costs instead of zero
commission. Rates as of August 2026:

- Brokerage: lower of Rs 20 or 0.1% per executed order, minimum Rs 5 (both sides).
- STT: delivery 0.1% on both sides; intraday 0.025% on the sell side only.
- Stamp duty (buy side only): delivery 0.015%; intraday 0.003%.
- Exchange transaction charges: NSE 0.00297%, BSE 0.00375% (both sides).
- Investor Protection Fund (NSE): 0.0001% (both sides).
- SEBI turnover fee: 0.0001% (both sides).
- GST: 18% on brokerage, exchange, IPF, and SEBI charges (not on STT/stamp duty).
- DP charge: ~Rs 20 + GST on each delivery sell (per scrip per day).

Approximations, stated rather than hidden:

- Charges are computed per fill. Brokerage caps/minimums and the DP charge are
  defined per order (or per scrip per day); with bar data fills are one-per-order
  so this matches, but tick-level partial fills would overstate flat components.
- Stamp duty varies by state for offline brokers; the uniform 2020 Stamp Act
  rates used here are what discount brokers apply.
"""

from decimal import ROUND_HALF_UP, Decimal

from nautilus_trader.execution import FeeModel
from nautilus_trader.model import Currency, Money


_INR = Currency.from_str("INR")
_PCT = Decimal("0.01")


class GrowwEquityFeeModel(FeeModel):
    """
    Groww brokerage plus Indian statutory charges for cash equities.

    Parameters
    ----------
    product : str
        ``"CNC"`` for delivery or ``"MIS"`` for intraday; decides STT and stamp
        duty treatment and whether DP charges apply on sells.
    exchange : str
        ``"NSE"`` or ``"BSE"``; decides the exchange transaction rate.

    """

    def __init__(self, product: str = "MIS", exchange: str = "NSE") -> None:
        if product not in ("CNC", "MIS"):
            raise ValueError(f"product must be 'CNC' or 'MIS', was {product!r}")
        if exchange not in ("NSE", "BSE"):
            raise ValueError(f"exchange must be 'NSE' or 'BSE', was {exchange!r}")
        self._product = product
        self._exchange = exchange

    def get_commission(self, order, fill_quantity, fill_px, instrument) -> Money:
        side = getattr(order, "order_side", None)
        if side is None:
            side = order.side  # Property, not a method, on pyo3 order types.
        is_buy = "BUY" in str(side).upper()
        notional = Decimal(str(fill_quantity)) * Decimal(str(fill_px))

        brokerage = max(Decimal(5), min(Decimal(20), notional * Decimal("0.1") * _PCT))

        if self._product == "CNC":
            stt = notional * Decimal("0.1") * _PCT  # Both sides for delivery.
            stamp = notional * Decimal("0.015") * _PCT if is_buy else Decimal(0)
        else:
            stt = Decimal(0) if is_buy else notional * Decimal("0.025") * _PCT
            stamp = notional * Decimal("0.003") * _PCT if is_buy else Decimal(0)

        exchange_rate = Decimal("0.00297") if self._exchange == "NSE" else Decimal("0.00375")
        exchange_txn = notional * exchange_rate * _PCT
        ipf = notional * Decimal("0.0001") * _PCT if self._exchange == "NSE" else Decimal(0)
        sebi = notional * Decimal("0.0001") * _PCT

        dp = Decimal(20) if (self._product == "CNC" and not is_buy) else Decimal(0)

        gst = Decimal("0.18") * (brokerage + exchange_txn + ipf + sebi + dp)

        total = brokerage + stt + stamp + exchange_txn + ipf + sebi + dp + gst
        return Money(total.quantize(Decimal("0.01"), rounding=ROUND_HALF_UP), _INR)


if __name__ == "__main__":
    # Self-check against hand-computed references for a 10 x 1300.00 fill.
    class _Order:
        def __init__(self, side: str) -> None:
            self.order_side = side

    checks = [
        # (product, side, expected) — worked by hand from the published rates.
        # notional = 13,000
        # MIS buy : brokerage 13.00, stamp 0.39, exch 0.3861, ipf 0.013, sebi 0.013,
        #           gst 0.18*13.4121 = 2.414178 -> total 16.216
        ("MIS", "BUY", Decimal("16.22")),
        # MIS sell: + stt 3.25, no stamp -> 13 + 3.25 + 0.3861 + 0.013 + 0.013 + 2.414178
        ("MIS", "SELL", Decimal("19.08")),
        # CNC buy : stt 13.00, stamp 1.95 -> 13 + 13 + 1.95 + 0.3861 + 0.013 + 0.013 + 2.414178
        ("CNC", "BUY", Decimal("30.78")),
        # CNC sell: stt 13.00, dp 20 -> 13 + 13 + 0.3861 + 0.013 + 0.013 + 20 + 0.18*33.4121
        ("CNC", "SELL", Decimal("52.43")),
    ]
    for product, side, expected in checks:
        model = GrowwEquityFeeModel(product=product)
        fee = model.get_commission(_Order(side), Decimal(10), Decimal("1300.0"), None)
        status = "OK " if Decimal(str(fee.as_decimal())) == expected else "FAIL"
        print(f"{status} {product} {side:4}: {fee} (expected {expected} INR)")
