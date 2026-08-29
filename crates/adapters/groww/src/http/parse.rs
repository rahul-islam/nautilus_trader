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

//! Conversion of Groww REST payloads into Nautilus reports.

use nautilus_core::UnixNanos;
use nautilus_model::{
    enums::{LiquiditySide, OrderStatus, OrderType, PositionSideSpecified, TimeInForce},
    identifiers::{AccountId, ClientOrderId, InstrumentId, PositionId, TradeId, VenueOrderId},
    instruments::{Instrument, InstrumentAny},
    reports::{FillReport, OrderStatusReport, PositionStatusReport},
    types::{Currency, Money, Price, Quantity},
};
use rust_decimal::prelude::FromPrimitive;

use crate::{
    common::{
        enums::{GrowwOrderStatus, GrowwTransactionType},
        parse::parse_naive_india,
    },
    http::{
        error::{Error, Result},
        models::{Order, Position, Trade},
    },
};

/// Size precision used for equity quantities, which trade in whole units.
const SIZE_PRECISION: u8 = 0;

/// Maps a venue order onto a Nautilus [`OrderStatusReport`].
///
/// The venue has no explicit partially-filled status; an order reported working with a non-zero
/// filled quantity is mapped to [`OrderStatus::PartiallyFilled`].
///
/// # Errors
///
/// Returns an error if a required field is missing or cannot be converted.
pub fn parse_order_status_report(
    order: &Order,
    account_id: AccountId,
    instrument: &InstrumentAny,
    client_order_id: Option<ClientOrderId>,
    ts_init: UnixNanos,
) -> Result<OrderStatusReport> {
    let venue_order_id = VenueOrderId::new(order.groww_order_id.as_str());
    let side = order
        .transaction_type
        .ok_or_else(|| Error::decode(format!("order {venue_order_id} missing side")))?;
    let order_type: OrderType = order
        .order_type
        .ok_or_else(|| Error::decode(format!("order {venue_order_id} missing type")))?
        .into();
    let time_in_force: TimeInForce = order.validity.map_or(TimeInForce::Day, Into::into);

    let mut status: OrderStatus = order.order_status.into();
    if order.filled_quantity > 0.0
        && order.filled_quantity < order.quantity
        && matches!(status, OrderStatus::Accepted)
    {
        status = OrderStatus::PartiallyFilled;
    }

    let ts_accepted = order
        .created_at
        .as_deref()
        .map(parse_naive_india)
        .transpose()?
        .unwrap_or(ts_init);
    let ts_last = order
        .exchange_time
        .as_deref()
        .map(parse_naive_india)
        .transpose()?
        .unwrap_or(ts_accepted);

    let mut report = OrderStatusReport::new(
        account_id,
        instrument.id(),
        client_order_id,
        venue_order_id,
        side.into(),
        order_type,
        time_in_force,
        status,
        Quantity::new(order.quantity, SIZE_PRECISION),
        Quantity::new(order.filled_quantity, SIZE_PRECISION),
        ts_accepted,
        ts_last,
        ts_init,
        None,
    );

    if order.price > 0.0 {
        report.price = Some(Price::new(order.price, instrument.price_precision()));
    }
    if let Some(trigger) = order.trigger_price.filter(|p| *p > 0.0) {
        report.trigger_price = Some(Price::new(trigger, instrument.price_precision()));
    }
    if let Some(avg) = order.average_fill_price.filter(|p| *p > 0.0)
        && let Some(avg) = rust_decimal::Decimal::from_f64(avg)
    {
        report.avg_px = Some(avg);
    }
    if let Some(remark) = &order.remark
        && !remark.is_empty()
    {
        report.cancel_reason = matches!(
            order.order_status,
            GrowwOrderStatus::Cancelled | GrowwOrderStatus::CancellationRequested
        )
        .then(|| remark.clone());
    }

    Ok(report)
}

/// Maps a venue trade onto a Nautilus [`FillReport`].
///
/// The venue reports no per-trade commission; fees are levied on the contract note instead, so
/// the commission is zero INR and cost modeling belongs to the strategy or a fee model.
///
/// # Errors
///
/// Returns an error if a required field is missing or cannot be converted.
pub fn parse_fill_report(
    trade: &Trade,
    account_id: AccountId,
    instrument: &InstrumentAny,
    client_order_id: Option<ClientOrderId>,
    ts_init: UnixNanos,
) -> Result<FillReport> {
    let side: GrowwTransactionType = trade
        .transaction_type
        .ok_or_else(|| Error::decode(format!("trade {} missing side", trade.groww_trade_id)))?;
    let ts_event = trade
        .trade_date_time
        .as_deref()
        .or(trade.created_at.as_deref())
        .map(parse_naive_india)
        .transpose()?
        .unwrap_or(ts_init);

    Ok(FillReport::new(
        account_id,
        instrument.id(),
        VenueOrderId::new(trade.groww_order_id.as_str()),
        TradeId::new(trade.groww_trade_id.as_str()),
        side.into(),
        Quantity::new(trade.quantity, SIZE_PRECISION),
        Price::new(trade.price, instrument.price_precision()),
        Money::new(0.0, Currency::INR()),
        // The venue does not report maker/taker; equity fills at a quoted price are takers in
        // practice and the engine treats this as informational.
        LiquiditySide::Taker,
        client_order_id,
        None,
        ts_event,
        ts_init,
        None,
    ))
}

/// Maps a venue position onto a Nautilus [`PositionStatusReport`].
///
/// # Errors
///
/// Returns an error if the net quantity cannot be represented.
pub fn parse_position_status_report(
    position: &Position,
    account_id: AccountId,
    instrument_id: InstrumentId,
    ts_init: UnixNanos,
) -> Result<PositionStatusReport> {
    let net = position.credit_quantity + position.carry_forward_credit_quantity
        - position.debit_quantity
        - position.carry_forward_debit_quantity;
    let side = if net > 0.0 {
        PositionSideSpecified::Long
    } else if net < 0.0 {
        PositionSideSpecified::Short
    } else {
        PositionSideSpecified::Flat
    };

    let venue_position_id = position.symbol_isin.as_deref().map(PositionId::new);

    Ok(PositionStatusReport::new(
        account_id,
        instrument_id,
        side,
        Quantity::new(net.abs(), SIZE_PRECISION),
        ts_init,
        ts_init,
        None,
        venue_position_id,
        None,
    ))
}

#[cfg(test)]
mod tests {
    use nautilus_model::instruments::Equity;
    use rstest::rstest;

    use super::*;
    use crate::common::enums::{GrowwOrderType, GrowwValidity};

    fn instrument() -> InstrumentAny {
        InstrumentAny::Equity(
            Equity::builder()
                .instrument_id("RELIANCE.NSE".parse().unwrap())
                .raw_symbol("RELIANCE".into())
                .currency(Currency::INR())
                .price_precision(1)
                .price_increment(Price::new(0.1, 1))
                .ts_event(UnixNanos::default())
                .ts_init(UnixNanos::default())
                .build()
                .unwrap(),
        )
    }

    fn order() -> Order {
        Order {
            groww_order_id: "GMK912345".to_string(),
            order_reference_id: Some("NT0011223344556677".to_string()),
            trading_symbol: Some("RELIANCE".to_string()),
            order_status: GrowwOrderStatus::Acked,
            quantity: 10.0,
            filled_quantity: 4.0,
            remaining_quantity: 6.0,
            price: 1287.0,
            trigger_price: None,
            average_fill_price: Some(1286.9),
            order_type: Some(GrowwOrderType::Limit),
            segment: None,
            exchange: None,
            product: None,
            transaction_type: Some(GrowwTransactionType::Buy),
            validity: Some(GrowwValidity::Day),
            remark: None,
            exchange_order_id: None,
            deliverable_quantity: None,
            amo_status: None,
            created_at: Some("2026-08-28T10:15:00".to_string()),
            exchange_time: None,
            trade_date: None,
        }
    }

    #[rstest]
    fn test_working_order_with_partial_fill_maps_to_partially_filled() {
        let report = parse_order_status_report(
            &order(),
            AccountId::new("GROWW-001"),
            &instrument(),
            None,
            UnixNanos::default(),
        )
        .unwrap();
        assert_eq!(report.order_status, OrderStatus::PartiallyFilled);
        assert_eq!(report.quantity, Quantity::new(10.0, 0));
        assert_eq!(report.filled_qty, Quantity::new(4.0, 0));
        assert_eq!(report.price, Some(Price::new(1287.0, 1)));
        // 10:15 IST == 04:45 UTC.
        assert_eq!(report.ts_accepted.as_u64(), 1_787_892_300_000_000_000);
    }

    #[rstest]
    fn test_position_nets_carry_forward() {
        let position = Position {
            trading_symbol: "RELIANCE".to_string(),
            exchange: None,
            product: Some(crate::common::enums::GrowwProduct::Cnc),
            credit_quantity: 10.0,
            credit_price: 0.0,
            debit_quantity: 2.0,
            debit_price: 0.0,
            carry_forward_credit_quantity: 5.0,
            carry_forward_credit_price: 0.0,
            carry_forward_debit_quantity: 0.0,
            carry_forward_debit_price: 0.0,
            quantity: 13.0,
            symbol_isin: None,
            net_carry_forward_quantity: 0.0,
            net_price: 0.0,
            net_carry_forward_price: 0.0,
            realised_pnl: 0.0,
        };
        let report = parse_position_status_report(
            &position,
            AccountId::new("GROWW-001"),
            "RELIANCE.NSE".parse().unwrap(),
            UnixNanos::default(),
        )
        .unwrap();
        assert_eq!(report.quantity, Quantity::new(13.0, 0));
        assert_eq!(report.position_side, PositionSideSpecified::Long);
    }
}
