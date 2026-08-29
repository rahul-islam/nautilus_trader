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

//! Feed subjects and decoded message types.
//!
//! Groww addresses market data by exchange token: a subject is a fixed prefix chosen by segment,
//! exchange, and feed kind, followed by the instrument's `exchange_token` from the instrument
//! master. Order and position updates are addressed instead by the account's `subscriptionId`
//! returned with the feed credentials.

use prost::Message as _;

use crate::{
    common::enums::{GrowwExchange, GrowwSegment},
    websocket::{
        error::{Error, Result},
        proto::{market_data::StocksSocketResponseProtoDto, orders::OrderDetailsBroadCastDto},
    },
};

/// Kind of market data carried on a subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FeedKind {
    /// Last traded price and session statistics.
    LivePrice,
    /// Extended last traded price detail.
    LivePriceDetailed,
    /// Five-level depth book.
    MarketDepth,
    /// Index level.
    IndexValue,
}

/// Returns the subject prefix for `segment`, `exchange`, and `kind`.
///
/// # Errors
///
/// Returns an error if the venue publishes no subject for the combination, which is the case for
/// index feeds outside the cash segment and for any feed on an exchange without a documented
/// prefix.
pub fn subject_prefix(
    segment: GrowwSegment,
    exchange: GrowwExchange,
    kind: FeedKind,
) -> Result<&'static str> {
    use FeedKind::{IndexValue, LivePrice, LivePriceDetailed, MarketDepth};
    use GrowwExchange::{Bse, Nse};
    use GrowwSegment::{Cash, Fno};

    let prefix = match (segment, exchange, kind) {
        (Cash, Nse, LivePrice) => "/ld/eq/nse/price.",
        (Cash, Bse, LivePrice) => "/ld/eq/bse/price.",
        (Cash, Nse, LivePriceDetailed) => "/ld/eq/nse/price_detailed.",
        (Cash, Bse, LivePriceDetailed) => "/ld/eq/bse/price_detailed.",
        (Cash, Nse, MarketDepth) => "/ld/eq/nse/book.",
        (Cash, Bse, MarketDepth) => "/ld/eq/bse/book.",
        (Fno, Nse, LivePrice) => "/ld/fo/nse/price.",
        (Fno, Bse, LivePrice) => "/ld/fo/bse/price.",
        (Fno, Nse, LivePriceDetailed) => "/ld/fo/nse/price_detailed.",
        (Fno, Bse, LivePriceDetailed) => "/ld/fo/bse/price_detailed.",
        (Fno, Nse, MarketDepth) => "/ld/fo/nse/book.",
        (Fno, Bse, MarketDepth) => "/ld/fo/bse/book.",
        (_, Nse, IndexValue) => "/ld/indices/nse/price.",
        (_, Bse, IndexValue) => "/ld/indices/bse/price.",
        _ => {
            return Err(Error::protocol(format!(
                "no Groww feed subject for segment {segment}, exchange {exchange}, kind {kind:?}"
            )));
        }
    };
    Ok(prefix)
}

/// Returns the market data subject for an instrument's `exchange_token`.
///
/// # Errors
///
/// Returns an error if the venue publishes no subject for the combination.
pub fn market_data_subject(
    segment: GrowwSegment,
    exchange: GrowwExchange,
    kind: FeedKind,
    exchange_token: &str,
) -> Result<String> {
    Ok(format!(
        "{}{exchange_token}",
        subject_prefix(segment, exchange, kind)?
    ))
}

/// Returns the equity order update subject for an account.
#[must_use]
pub fn equity_order_updates_subject(subscription_id: &str) -> String {
    format!("stocks/order/updates.apex.{subscription_id}")
}

/// Returns the derivatives order update subject for an account.
#[must_use]
pub fn derivatives_order_updates_subject(subscription_id: &str) -> String {
    format!("stocks_fo/order/updates.apex.{subscription_id}")
}

/// Returns the derivatives position update subject for an account.
#[must_use]
pub fn derivatives_position_updates_subject(subscription_id: &str) -> String {
    format!("stocks_fo/position/updates.apex.{subscription_id}")
}

/// A decoded feed payload together with the subject it arrived on.
#[derive(Debug, Clone)]
pub enum FeedMessage {
    /// Market data for a single instrument.
    MarketData(Box<StocksSocketResponseProtoDto>),
    /// An order update for the account.
    OrderUpdate(Box<OrderDetailsBroadCastDto>),
}

/// Classifies a subject and decodes its protobuf payload.
///
/// The payload type is determined by the subject, since the venue publishes no discriminator
/// inside the message itself.
///
/// # Errors
///
/// Returns an error if the subject is unrecognized or the payload fails to decode.
pub fn decode_message(subject: &str, payload: &[u8]) -> Result<FeedMessage> {
    if subject.starts_with("/ld/") {
        let message = StocksSocketResponseProtoDto::decode(payload)
            .map_err(|e| Error::decode(format!("market data payload on `{subject}`: {e}")))?;
        return Ok(FeedMessage::MarketData(Box::new(message)));
    }

    if subject.starts_with("stocks/order/updates") || subject.starts_with("stocks_fo/order/updates")
    {
        let message = OrderDetailsBroadCastDto::decode(payload)
            .map_err(|e| Error::decode(format!("order update payload on `{subject}`: {e}")))?;
        return Ok(FeedMessage::OrderUpdate(Box::new(message)));
    }

    Err(Error::protocol(format!("unrecognized subject `{subject}`")))
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case(
        GrowwSegment::Cash,
        GrowwExchange::Nse,
        FeedKind::LivePrice,
        "/ld/eq/nse/price.2885"
    )]
    #[case(
        GrowwSegment::Cash,
        GrowwExchange::Bse,
        FeedKind::MarketDepth,
        "/ld/eq/bse/book.2885"
    )]
    #[case(
        GrowwSegment::Fno,
        GrowwExchange::Nse,
        FeedKind::LivePrice,
        "/ld/fo/nse/price.2885"
    )]
    #[case(
        GrowwSegment::Fno,
        GrowwExchange::Nse,
        FeedKind::MarketDepth,
        "/ld/fo/nse/book.2885"
    )]
    fn test_market_data_subjects(
        #[case] segment: GrowwSegment,
        #[case] exchange: GrowwExchange,
        #[case] kind: FeedKind,
        #[case] expected: &str,
    ) {
        assert_eq!(
            market_data_subject(segment, exchange, kind, "2885").unwrap(),
            expected
        );
    }

    #[rstest]
    fn test_account_subjects() {
        assert_eq!(
            equity_order_updates_subject("abc="),
            "stocks/order/updates.apex.abc="
        );
        assert_eq!(
            derivatives_order_updates_subject("abc="),
            "stocks_fo/order/updates.apex.abc="
        );
        assert_eq!(
            derivatives_position_updates_subject("abc="),
            "stocks_fo/position/updates.apex.abc="
        );
    }

    #[rstest]
    fn test_unsupported_subject_combination_errors() {
        assert!(
            subject_prefix(
                GrowwSegment::Commodity,
                GrowwExchange::Mcx,
                FeedKind::LivePrice
            )
            .is_err()
        );
    }

    #[rstest]
    fn test_decode_market_data_round_trip() {
        use crate::websocket::proto::market_data::{
            StocksLivePriceProto, StocksSocketResponseProtoDto,
        };

        let original = StocksSocketResponseProtoDto {
            symbol: "RELIANCE".to_string(),
            segment: 0,
            exchange: 1,
            stock_live_price: Some(StocksLivePriceProto {
                ts_in_millis: 1_787_912_999_000.0,
                ltp: 1287.0,
                volume: 1000.0,
                ..Default::default()
            }),
            stocks_market_depth: None,
            stocks_live_indices: None,
        };

        let encoded = original.encode_to_vec();
        let FeedMessage::MarketData(decoded) =
            decode_message("/ld/eq/nse/price.2885", &encoded).unwrap()
        else {
            panic!("expected market data");
        };
        assert_eq!(decoded.symbol, "RELIANCE");
        assert_eq!(decoded.stock_live_price.unwrap().ltp, 1287.0);
    }

    #[rstest]
    fn test_decode_rejects_unknown_subject() {
        assert!(decode_message("something/else", &[]).is_err());
    }
}
