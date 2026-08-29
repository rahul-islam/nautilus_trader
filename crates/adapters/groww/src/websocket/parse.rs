// -------------------------------------------------------------------------------------------------
//  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
//  https://nautechsystems.io
//
//  Licensed under the GNU Lesser General Public License Version 3.0 (the "License");
//  You may not use this file except in compliance with the License.
//  You may obtain a copy of the License at https://www.gnu.org/licenses/lgpl-3.0.en.html
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.
// -------------------------------------------------------------------------------------------------

//! Conversion of decoded feed payloads into Nautilus domain types.
//!
//! The feed publishes three market data shapes. The depth feed maps onto [`OrderBookDepth10`]
//! (padded from the venue's five levels) and yields a [`QuoteTick`] from its top of book. The
//! live price feed carries the last traded price and *cumulative* session volume but no trade
//! size, so trade ticks are synthesized by differencing consecutive volumes; the first update
//! after (re)subscribing therefore establishes a baseline and produces no tick.

use nautilus_core::UnixNanos;
use nautilus_model::{
    data::{BookOrder, DEPTH10_LEN, OrderBookDepth10, QuoteTick, TradeTick},
    enums::{AggressorSide, OrderSide},
    identifiers::{InstrumentId, TradeId},
    instruments::{Instrument, InstrumentAny},
    types::{Price, Quantity},
};

use crate::{
    common::parse::parse_millis_f64,
    websocket::{
        error::{Error, Result},
        proto::market_data::{StocksLivePriceProto, StocksMarketDepthProto},
    },
};

/// Size precision used for equity quantities, which trade in whole units.
const SIZE_PRECISION: u8 = 0;

/// Builds a [`QuoteTick`] from the top level of a depth message.
///
/// Returns `None` when either side of the book is empty, which is normal outside continuous
/// trading; a one-sided quote would invent a zero price on the other side.
///
/// # Errors
///
/// Returns an error if the timestamp cannot be converted.
pub fn parse_quote_tick(
    depth: &StocksMarketDepthProto,
    instrument: &InstrumentAny,
    ts_init: UnixNanos,
) -> Result<Option<QuoteTick>> {
    let (Some(bid), Some(ask)) = (best_level(depth, true), best_level(depth, false)) else {
        return Ok(None);
    };
    let ts_event = parse_millis_f64(depth.ts_in_millis)?;

    let quote = QuoteTick::new(
        instrument.id(),
        Price::new(bid.0, instrument.price_precision()),
        Price::new(ask.0, instrument.price_precision()),
        Quantity::new(bid.1, SIZE_PRECISION),
        Quantity::new(ask.1, SIZE_PRECISION),
        ts_event,
        ts_init,
    );
    Ok(Some(quote))
}

/// Returns the best `(price, qty)` on one side of a depth message.
///
/// The venue keys book levels by index, with level 0 the best price; empty levels are published
/// as zero prices and skipped.
fn best_level(depth: &StocksMarketDepthProto, bid: bool) -> Option<(f64, f64)> {
    let book = if bid {
        &depth.buy_book
    } else {
        &depth.sell_book
    };
    book.iter()
        .filter(|(_, level)| level.price > 0.0 && level.qty > 0.0)
        .min_by_key(|(index, _)| **index)
        .map(|(_, level)| (level.price, level.qty))
}

/// Builds an [`OrderBookDepth10`] from a depth message, padding the venue's five levels.
///
/// # Errors
///
/// Returns an error if the timestamp cannot be converted.
pub fn parse_depth10(
    depth: &StocksMarketDepthProto,
    instrument: &InstrumentAny,
    sequence: u64,
    ts_init: UnixNanos,
) -> Result<OrderBookDepth10> {
    let ts_event = parse_millis_f64(depth.ts_in_millis)?;
    let precision = instrument.price_precision();

    let mut bids = [null_order(OrderSide::Buy); DEPTH10_LEN];
    let mut asks = [null_order(OrderSide::Sell); DEPTH10_LEN];
    let mut bid_counts = [0u32; DEPTH10_LEN];
    let mut ask_counts = [0u32; DEPTH10_LEN];

    for (index, level) in &depth.buy_book {
        let slot = usize::try_from(*index).unwrap_or(DEPTH10_LEN);
        if slot < DEPTH10_LEN && level.price > 0.0 {
            bids[slot] = BookOrder::new(
                OrderSide::Buy,
                Price::new(level.price, precision),
                Quantity::new(level.qty, SIZE_PRECISION),
                0,
            );
            bid_counts[slot] = 1;
        }
    }
    for (index, level) in &depth.sell_book {
        let slot = usize::try_from(*index).unwrap_or(DEPTH10_LEN);
        if slot < DEPTH10_LEN && level.price > 0.0 {
            asks[slot] = BookOrder::new(
                OrderSide::Sell,
                Price::new(level.price, precision),
                Quantity::new(level.qty, SIZE_PRECISION),
                0,
            );
            ask_counts[slot] = 1;
        }
    }

    Ok(OrderBookDepth10::new(
        instrument.id(),
        bids,
        asks,
        bid_counts,
        ask_counts,
        0,
        sequence,
        ts_event,
        ts_init,
    ))
}

fn null_order(side: OrderSide) -> BookOrder {
    BookOrder::new(side, Price::zero(0), Quantity::zero(SIZE_PRECISION), 0)
}

/// Synthesizes trade ticks from the live price feed by differencing session volume.
///
/// Holds the last seen cumulative volume per instrument; the caller keeps one tracker per
/// subscription. A volume decrease means the session counter reset (a new session or a feed
/// correction) and re-baselines without emitting.
#[derive(Debug, Default)]
pub struct TradeTickSynthesizer {
    last_volume: ahash::AHashMap<InstrumentId, f64>,
    counter: u64,
}

impl TradeTickSynthesizer {
    /// Creates a new empty [`TradeTickSynthesizer`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Processes a live price update, returning a tick when new volume traded at the LTP.
    ///
    /// The venue publishes no per-trade size or aggressor, so the tick's size is the volume
    /// traded since the previous update (possibly aggregating several trades) and the aggressor
    /// is [`AggressorSide::NoAggressor`].
    ///
    /// # Errors
    ///
    /// Returns an error if the timestamp cannot be converted.
    pub fn process(
        &mut self,
        price: &StocksLivePriceProto,
        instrument: &InstrumentAny,
        ts_init: UnixNanos,
    ) -> Result<Option<TradeTick>> {
        if price.ltp <= 0.0 || !price.ltp.is_finite() {
            return Ok(None);
        }
        let instrument_id = instrument.id();
        let volume = price.volume;
        let previous = self.last_volume.insert(instrument_id, volume);

        let Some(previous) = previous else {
            return Ok(None); // First update establishes the baseline.
        };
        let traded = volume - previous;
        if traded <= 0.0 {
            return Ok(None); // No new volume, or a session counter reset.
        }

        let ts_event = parse_millis_f64(price.ts_in_millis)?;
        self.counter += 1;
        let trade = TradeTick::new(
            instrument_id,
            Price::new(price.ltp, instrument.price_precision()),
            Quantity::new(traded, SIZE_PRECISION),
            AggressorSide::NoAggressor,
            // The feed carries no venue trade id; a locally monotone id keeps ticks unique
            // within this session while remaining recognizably synthetic.
            TradeId::new(format!("G-{}-{}", ts_event.as_u64(), self.counter)),
            ts_event,
            ts_init,
        );
        Ok(Some(trade))
    }

    /// Clears the volume baseline, forcing the next update per instrument to re-baseline.
    pub fn reset(&mut self) {
        self.last_volume.clear();
    }
}

/// Validates that a decoded market data payload belongs to `instrument`.
///
/// # Errors
///
/// Returns an error if the exchange token disagrees with the subscribed instrument.
pub fn ensure_symbol(expected: &str, actual: &str) -> Result<()> {
    if expected == actual {
        Ok(())
    } else {
        Err(Error::decode(format!(
            "feed message for `{actual}` arrived on subscription for `{expected}`"
        )))
    }
}

#[cfg(test)]
mod tests {
    use nautilus_model::instruments::{Equity, InstrumentAny};
    use rstest::rstest;
    use ustr::Ustr;

    use super::*;
    use crate::websocket::proto::market_data::BookProto;

    fn instrument() -> InstrumentAny {
        InstrumentAny::Equity(
            Equity::builder()
                .instrument_id("RELIANCE.NSE".parse().unwrap())
                .raw_symbol("RELIANCE".into())
                .currency(nautilus_model::types::Currency::INR())
                .price_precision(1)
                .price_increment(Price::new(0.1, 1))
                .maybe_isin(Some(Ustr::from("INE002A01018")))
                .ts_event(UnixNanos::default())
                .ts_init(UnixNanos::default())
                .build()
                .unwrap(),
        )
    }

    fn depth() -> StocksMarketDepthProto {
        let mut depth = StocksMarketDepthProto {
            ts_in_millis: 1_787_912_999_000.0,
            ..Default::default()
        };
        depth.buy_book.insert(
            0,
            BookProto {
                price: 1287.0,
                qty: 50.0,
            },
        );
        depth.buy_book.insert(
            1,
            BookProto {
                price: 1286.9,
                qty: 120.0,
            },
        );
        depth.sell_book.insert(
            0,
            BookProto {
                price: 1287.2,
                qty: 30.0,
            },
        );
        depth
    }

    #[rstest]
    fn test_quote_from_depth_top_of_book() {
        let quote = parse_quote_tick(&depth(), &instrument(), UnixNanos::default())
            .unwrap()
            .unwrap();
        assert_eq!(quote.bid_price, Price::new(1287.0, 1));
        assert_eq!(quote.ask_price, Price::new(1287.2, 1));
        assert_eq!(quote.bid_size, Quantity::new(50.0, 0));
        assert_eq!(quote.ask_size, Quantity::new(30.0, 0));
        assert_eq!(quote.ts_event.as_u64(), 1_787_912_999_000_000_000);
    }

    #[rstest]
    fn test_quote_requires_both_sides() {
        let mut one_sided = depth();
        one_sided.sell_book.clear();
        let quote = parse_quote_tick(&one_sided, &instrument(), UnixNanos::default()).unwrap();
        assert!(quote.is_none());
    }

    #[rstest]
    fn test_depth10_pads_missing_levels() {
        let parsed = parse_depth10(&depth(), &instrument(), 7, UnixNanos::default()).unwrap();
        assert_eq!(parsed.bids[0].price, Price::new(1287.0, 1));
        assert_eq!(parsed.bids[1].price, Price::new(1286.9, 1));
        assert_eq!(parsed.bid_counts[0], 1);
        assert_eq!(parsed.bid_counts[2], 0);
        assert_eq!(parsed.ask_counts[1], 0);
        assert_eq!(parsed.sequence, 7);
    }

    #[rstest]
    fn test_trade_synthesis_differences_volume() {
        let mut synthesizer = TradeTickSynthesizer::new();
        let instrument = instrument();
        let mut price = StocksLivePriceProto {
            ts_in_millis: 1_787_912_999_000.0,
            ltp: 1287.0,
            volume: 1_000.0,
            ..Default::default()
        };

        // Baseline: no tick.
        assert!(
            synthesizer
                .process(&price, &instrument, UnixNanos::default())
                .unwrap()
                .is_none()
        );

        // 250 shares traded since the last update.
        price.volume = 1_250.0;
        price.ltp = 1_287.5;
        let trade = synthesizer
            .process(&price, &instrument, UnixNanos::default())
            .unwrap()
            .unwrap();
        assert_eq!(trade.price, Price::new(1287.5, 1));
        assert_eq!(trade.size, Quantity::new(250.0, 0));

        // Unchanged volume: no tick.
        assert!(
            synthesizer
                .process(&price, &instrument, UnixNanos::default())
                .unwrap()
                .is_none()
        );

        // Session reset re-baselines without emitting.
        price.volume = 10.0;
        assert!(
            synthesizer
                .process(&price, &instrument, UnixNanos::default())
                .unwrap()
                .is_none()
        );
    }
}
