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

//! Response models for the Groww REST API.
//!
//! Every response shares an envelope carrying a `status` discriminator, with the useful content
//! under `payload` on success and under `error` on failure. [`GrowwResponse`] models that envelope
//! so a failure is surfaced as a typed error rather than a missing field.

use serde::{Deserialize, Serialize};

use crate::common::enums::{
    GrowwExchange, GrowwOrderStatus, GrowwOrderType, GrowwProduct, GrowwSegment,
    GrowwTransactionType, GrowwValidity,
};

/// Status discriminator returned in every response envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GrowwStatus {
    /// The request succeeded and `payload` is populated.
    Success,
    /// The request failed and `error` is populated.
    Failure,
}

/// Error detail returned inside a failed response envelope.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GrowwErrorDetail {
    /// Venue error code, reported as a string even when numeric.
    #[serde(default)]
    pub code: Option<String>,
    /// Human-readable error message.
    #[serde(default)]
    pub message: Option<String>,
    /// Additional context, when the venue supplies any.
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// Envelope wrapping every Groww REST response.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GrowwResponse<T> {
    /// Whether the request succeeded.
    pub status: GrowwStatus,
    /// Response content, present when `status` is `SUCCESS`.
    #[serde(default = "Option::default")]
    pub payload: Option<T>,
    /// Error detail, present when `status` is `FAILURE`.
    #[serde(default)]
    pub error: Option<GrowwErrorDetail>,
}

/// Account and entitlement detail for the authenticated user.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UserDetail {
    /// Vendor-scoped user identifier.
    pub vendor_user_id: String,
    /// Unique client code assigned by the broker.
    pub ucc: String,
    /// Whether NSE trading is enabled.
    #[serde(default)]
    pub nse_enabled: bool,
    /// Whether BSE trading is enabled.
    #[serde(default)]
    pub bse_enabled: bool,
    /// Whether the demat debit and pledge instruction mandate is active.
    #[serde(default)]
    pub ddpi_enabled: bool,
    /// Segments the account is permitted to trade.
    #[serde(default)]
    pub active_segments: Vec<String>,
}

/// Margin detail for the futures and options segment.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct FnoMarginDetails {
    /// Total margin consumed across the segment.
    #[serde(default)]
    pub net_fno_margin_used: f64,
    /// SPAN margin consumed.
    #[serde(default)]
    pub span_margin_used: f64,
    /// Exposure margin consumed.
    #[serde(default)]
    pub exposure_margin_used: f64,
    /// Balance available for futures.
    #[serde(default)]
    pub future_balance_available: f64,
    /// Balance available for buying options.
    #[serde(default)]
    pub option_buy_balance_available: f64,
    /// Balance available for writing options.
    #[serde(default)]
    pub option_sell_balance_available: f64,
}

/// Margin detail for the equity segment.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct EquityMarginDetails {
    /// Total margin consumed across the segment.
    #[serde(default)]
    pub net_equity_margin_used: f64,
    /// Margin consumed by delivery positions.
    #[serde(default)]
    pub cnc_margin_used: f64,
    /// Margin consumed by intraday positions.
    #[serde(default)]
    pub mis_margin_used: f64,
    /// Balance available for delivery orders.
    #[serde(default)]
    pub cnc_balance_available: f64,
    /// Balance available for intraday orders.
    #[serde(default)]
    pub mis_balance_available: f64,
}

/// Margin detail for the commodity segment.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct CommodityMarginDetails {
    /// SPAN margin consumed.
    #[serde(default)]
    pub commodity_span_margin: f64,
    /// Exposure margin consumed.
    #[serde(default)]
    pub commodity_exposure_margin: f64,
    /// Tender margin consumed near delivery.
    #[serde(default)]
    pub commodity_tender_margin: f64,
    /// Special margin levied by the exchange.
    #[serde(default)]
    pub commodity_special_margin: f64,
    /// Additional margin levied by the exchange.
    #[serde(default)]
    pub commodity_additional_margin: f64,
    /// Unrealised mark to market.
    #[serde(default)]
    pub commodity_unrealised_m2m: f64,
    /// Realised mark to market.
    #[serde(default)]
    pub commodity_realised_m2m: f64,
}

/// Account-level margin and balance detail.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UserMargin {
    /// Free cash available to trade.
    #[serde(default)]
    pub clear_cash: f64,
    /// Total margin consumed across all segments.
    #[serde(default)]
    pub net_margin_used: f64,
    /// Brokerage and statutory charges accrued.
    #[serde(default)]
    pub brokerage_and_charges: f64,
    /// Collateral value consumed as margin.
    #[serde(default)]
    pub collateral_used: f64,
    /// Collateral value available as margin.
    #[serde(default)]
    pub collateral_available: f64,
    /// Ad-hoc margin granted by the broker.
    #[serde(default)]
    pub adhoc_margin: f64,
    /// Futures and options segment detail.
    #[serde(default)]
    pub fno_margin_details: FnoMarginDetails,
    /// Equity segment detail.
    #[serde(default)]
    pub equity_margin_details: EquityMarginDetails,
    /// Commodity segment detail.
    #[serde(default)]
    pub commodity_margin_details: CommodityMarginDetails,
}

/// A single demat holding.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Holding {
    /// International Securities Identification Number.
    pub isin: String,
    /// Venue trading symbol.
    pub trading_symbol: String,
    /// Total quantity held.
    #[serde(default)]
    pub quantity: f64,
    /// Volume-weighted average acquisition price.
    #[serde(default)]
    pub average_price: f64,
    /// Quantity pledged for margin.
    #[serde(default)]
    pub pledge_quantity: f64,
    /// Quantity locked in the demat account.
    #[serde(default)]
    pub demat_locked_quantity: f64,
    /// Quantity locked by the broker.
    #[serde(default)]
    pub groww_locked_quantity: f64,
    /// Quantity re-pledged to the clearing corporation.
    #[serde(default)]
    pub repledge_quantity: f64,
    /// Quantity still in the T+1 settlement cycle.
    #[serde(default)]
    pub t1_quantity: f64,
    /// Quantity free to sell.
    #[serde(default)]
    pub demat_free_quantity: f64,
    /// Quantity credited by a corporate action.
    #[serde(default)]
    pub corporate_action_additional_quantity: f64,
    /// Quantity in an active demat transfer.
    #[serde(default)]
    pub active_demat_transfer_quantity: f64,
    /// Exchanges the holding may be sold on.
    #[serde(default)]
    pub tradable_exchanges: Vec<GrowwExchange>,
}

/// Wrapper for the holdings payload.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HoldingsPayload {
    /// Holdings in the demat account.
    #[serde(default)]
    pub holdings: Vec<Holding>,
}

/// An open position, as documented for `/positions/user`.
///
/// The documentation types every price as an integer without stating its unit, so prices here
/// are carried opaquely and never used for money arithmetic; position reports derive only from
/// the quantity fields.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Position {
    /// Venue trading symbol.
    pub trading_symbol: String,
    /// Exchange the position is held on.
    #[serde(default)]
    pub exchange: Option<GrowwExchange>,
    /// Product type governing settlement.
    #[serde(default)]
    pub product: Option<GrowwProduct>,
    /// Credit (buy) quantity for the day.
    #[serde(default)]
    pub credit_quantity: f64,
    /// Credit (buy) value for the day, in the venue's undocumented integer unit.
    #[serde(default)]
    pub credit_price: f64,
    /// Debit (sell) quantity for the day.
    #[serde(default)]
    pub debit_quantity: f64,
    /// Debit (sell) value for the day, in the venue's undocumented integer unit.
    #[serde(default)]
    pub debit_price: f64,
    /// Quantity carried forward from previous sessions.
    #[serde(default)]
    pub carry_forward_credit_quantity: f64,
    /// Value carried forward from previous sessions.
    #[serde(default)]
    pub carry_forward_credit_price: f64,
    /// Sell quantity carried forward from previous sessions.
    #[serde(default)]
    pub carry_forward_debit_quantity: f64,
    /// Sell value carried forward from previous sessions.
    #[serde(default)]
    pub carry_forward_debit_price: f64,
    /// Net quantity as reported by the venue.
    #[serde(default)]
    pub quantity: f64,
    /// International Securities Identification Number.
    #[serde(default)]
    pub symbol_isin: Option<String>,
    /// Net carry-forward quantity.
    #[serde(default)]
    pub net_carry_forward_quantity: f64,
    /// Net price, in the venue's undocumented integer unit.
    #[serde(default)]
    pub net_price: f64,
    /// Net carry-forward price, in the venue's undocumented integer unit.
    #[serde(default)]
    pub net_carry_forward_price: f64,
    /// Realised profit and loss, in the venue's undocumented integer unit.
    #[serde(default)]
    pub realised_pnl: f64,
}

/// Wrapper for the positions payload.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PositionsPayload {
    /// Open positions.
    #[serde(default)]
    pub positions: Vec<Position>,
}

/// An order as reported by the venue.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Order {
    /// Venue-assigned order identifier.
    pub groww_order_id: String,
    /// Client-supplied reference, when one was sent with the order.
    #[serde(default)]
    pub order_reference_id: Option<String>,
    /// Venue trading symbol.
    #[serde(default)]
    pub trading_symbol: Option<String>,
    /// Lifecycle status.
    pub order_status: GrowwOrderStatus,
    /// Quantity submitted.
    #[serde(default)]
    pub quantity: f64,
    /// Quantity filled so far.
    #[serde(default)]
    pub filled_quantity: f64,
    /// Quantity still working.
    #[serde(default)]
    pub remaining_quantity: f64,
    /// Limit price, zero for market orders.
    #[serde(default)]
    pub price: f64,
    /// Trigger price for stop orders.
    #[serde(default)]
    pub trigger_price: Option<f64>,
    /// Average price of the filled quantity.
    #[serde(default)]
    pub average_fill_price: Option<f64>,
    /// Order type.
    #[serde(default)]
    pub order_type: Option<GrowwOrderType>,
    /// Trading segment.
    #[serde(default)]
    pub segment: Option<GrowwSegment>,
    /// Exchange the order was routed to.
    #[serde(default)]
    pub exchange: Option<GrowwExchange>,
    /// Product type governing settlement.
    #[serde(default)]
    pub product: Option<GrowwProduct>,
    /// Side of the order.
    #[serde(default)]
    pub transaction_type: Option<GrowwTransactionType>,
    /// Time in force.
    #[serde(default)]
    pub validity: Option<GrowwValidity>,
    /// Free-text remark, carrying the rejection reason when an order is rejected.
    #[serde(default)]
    pub remark: Option<String>,
    /// Exchange-assigned order identifier, present on some detail responses.
    #[serde(default)]
    pub exchange_order_id: Option<String>,
    /// Quantity deliverable to the demat account.
    #[serde(default)]
    pub deliverable_quantity: Option<f64>,
    /// After-market-order status, when the order was queued outside market hours.
    #[serde(default)]
    pub amo_status: Option<String>,
    /// Local creation timestamp, in Indian market time.
    #[serde(default)]
    pub created_at: Option<String>,
    /// Local exchange timestamp, in Indian market time.
    #[serde(default)]
    pub exchange_time: Option<String>,
    /// Local trade date, in Indian market time.
    #[serde(default)]
    pub trade_date: Option<String>,
}

/// Wrapper for the order list payload.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OrderListPayload {
    /// Orders on the requested page.
    #[serde(default)]
    pub order_list: Vec<Order>,
}

/// Response returned when placing, modifying, or cancelling an order.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OrderAck {
    /// Venue-assigned order identifier.
    pub groww_order_id: String,
    /// Lifecycle status immediately after the command.
    #[serde(default)]
    pub order_status: Option<GrowwOrderStatus>,
    /// Client-supplied reference echoed back.
    #[serde(default)]
    pub order_reference_id: Option<String>,
    /// Free-text remark, carrying the rejection reason when a command is rejected.
    #[serde(default)]
    pub remark: Option<String>,
}

/// A single trade (fill) against an order.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Trade {
    /// Venue-assigned trade identifier.
    pub groww_trade_id: String,
    /// Venue-assigned order identifier this trade belongs to.
    pub groww_order_id: String,
    /// Exchange-assigned trade identifier.
    #[serde(default)]
    pub exchange_trade_id: Option<String>,
    /// Exchange-assigned order identifier.
    #[serde(default)]
    pub exchange_order_id: Option<String>,
    /// Venue trading symbol.
    #[serde(default)]
    pub trading_symbol: Option<String>,
    /// International Securities Identification Number.
    #[serde(default)]
    pub isin: Option<String>,
    /// Traded quantity.
    #[serde(default)]
    pub quantity: f64,
    /// Traded price.
    #[serde(default)]
    pub price: f64,
    /// Settlement status of the trade.
    #[serde(default)]
    pub trade_status: Option<String>,
    /// Exchange the trade occurred on.
    #[serde(default)]
    pub exchange: Option<GrowwExchange>,
    /// Trading segment.
    #[serde(default)]
    pub segment: Option<GrowwSegment>,
    /// Product type governing settlement.
    #[serde(default)]
    pub product: Option<GrowwProduct>,
    /// Side of the trade.
    #[serde(default)]
    pub transaction_type: Option<GrowwTransactionType>,
    /// Free-text remark.
    #[serde(default)]
    pub remark: Option<String>,
    /// Local creation timestamp, in Indian market time.
    #[serde(default)]
    pub created_at: Option<String>,
    /// Local trade date and time, in Indian market time.
    #[serde(default)]
    pub trade_date_time: Option<String>,
    /// Exchange settlement number.
    #[serde(default)]
    pub settlement_number: Option<String>,
}

/// Wrapper for the trade list payload.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TradeListPayload {
    /// Trades on the requested page.
    #[serde(default)]
    pub trade_list: Vec<Trade>,
}

/// Access token issued by the authentication endpoint.
///
/// This endpoint answers with the token object directly rather than the usual response envelope,
/// so it is decoded on its own rather than through [`GrowwResponse`].
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccessToken {
    /// Bearer token authorizing subsequent REST calls.
    pub token: String,
    /// Venue-side reference for the issued token.
    #[serde(rename = "tokenRefId", default)]
    pub token_ref_id: Option<String>,
    /// Local expiry timestamp in Indian market time, when the venue supplies one.
    ///
    /// Groww expires tokens daily rather than after a fixed lifetime.
    #[serde(default)]
    pub expiry: Option<String>,
    /// Session name the venue associates with the token.
    #[serde(rename = "sessionName", default)]
    pub session_name: Option<String>,
    /// Whether the token is currently active.
    #[serde(default)]
    pub active: Option<bool>,
}

/// Credentials authorizing a streaming feed connection.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SocketToken {
    /// JWT presented to the feed.
    pub token: String,
    /// Identifier scoping the account's order and position subjects.
    #[serde(rename = "subscriptionId")]
    pub subscription_id: String,
    /// Local expiry timestamp of the JWT.
    #[serde(default)]
    pub expiry: Option<String>,
}

/// One side of the five-level depth book carried in a quote.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct DepthLevel {
    /// Price at this level, zero when the level is empty.
    #[serde(default)]
    pub price: f64,
    /// Aggregate quantity at this level.
    #[serde(default)]
    pub quantity: f64,
    /// Number of orders resting at this level.
    #[serde(rename = "orderCount", default)]
    pub order_count: u32,
}

/// Five-level depth book carried in a quote.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct QuoteDepth {
    /// Bid levels, best first.
    #[serde(default)]
    pub buy: Vec<DepthLevel>,
    /// Ask levels, best first.
    #[serde(default)]
    pub sell: Vec<DepthLevel>,
}

/// Open, high, low, and close for the current session.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
pub struct Ohlc {
    /// Session open.
    #[serde(default)]
    pub open: f64,
    /// Session high.
    #[serde(default)]
    pub high: f64,
    /// Session low.
    #[serde(default)]
    pub low: f64,
    /// Previous session close.
    #[serde(default)]
    pub close: f64,
}

/// Full quote for a single instrument.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Quote {
    /// Last traded price.
    #[serde(default)]
    pub last_price: f64,
    /// Quantity of the last trade.
    #[serde(default)]
    pub last_trade_quantity: f64,
    /// Time of the last trade, in UNIX epoch seconds.
    #[serde(default)]
    pub last_trade_time: Option<i64>,
    /// Volume-weighted average price for the session.
    #[serde(default)]
    pub average_price: Option<f64>,
    /// Best bid price.
    #[serde(default)]
    pub bid_price: Option<f64>,
    /// Quantity at the best bid.
    #[serde(default)]
    pub bid_quantity: Option<f64>,
    /// Best ask price.
    #[serde(default)]
    pub offer_price: Option<f64>,
    /// Quantity at the best ask.
    #[serde(default)]
    pub offer_quantity: Option<f64>,
    /// Session open, high, low, and previous close.
    #[serde(default)]
    pub ohlc: Ohlc,
    /// Five-level depth book.
    #[serde(default)]
    pub depth: QuoteDepth,
    /// Total traded volume for the session.
    #[serde(default)]
    pub volume: Option<f64>,
    /// Total traded value for the session.
    #[serde(default)]
    pub total_buy_quantity: Option<f64>,
    /// Total resting sell quantity.
    #[serde(default)]
    pub total_sell_quantity: Option<f64>,
    /// Upper price band for the session.
    #[serde(default)]
    pub upper_circuit_limit: Option<f64>,
    /// Lower price band for the session.
    #[serde(default)]
    pub lower_circuit_limit: Option<f64>,
    /// Open interest, for derivatives.
    #[serde(default)]
    pub open_interest: Option<f64>,
    /// Implied volatility, for options.
    #[serde(default)]
    pub implied_volatility: Option<f64>,
}

/// Wrapper for the historical candle payload.
///
/// The venue returns each candle as a positional array rather than an object, in the order
/// timestamp, open, high, low, close, volume, open interest.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CandlesPayload {
    /// Candles covering the requested range.
    #[serde(default)]
    pub candles: Vec<Candle>,
}

/// A single historical candle.
///
/// Deserialized from the venue's positional array form. The timestamp is a naive local time in the
/// Indian market time zone. Every numeric field is optional because the venue publishes partial
/// rows: open interest is absent outside FNO, and OHLC fields are occasionally null even on
/// intraday equity candles. [`Self::ohlcv`] is the checked accessor for a complete bar.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Candle(
    /// Candle open time, as naive Indian local time.
    pub String,
    /// Open price.
    pub Option<f64>,
    /// High price.
    pub Option<f64>,
    /// Low price.
    pub Option<f64>,
    /// Close price.
    pub Option<f64>,
    /// Traded volume.
    pub Option<f64>,
    /// Open interest, when the venue reports it.
    pub Option<f64>,
);

impl Candle {
    /// Returns the candle open time as naive Indian local time.
    #[must_use]
    pub fn timestamp(&self) -> &str {
        &self.0
    }

    /// Returns `(open, high, low, close, volume)` when the row is complete.
    ///
    /// A missing volume is treated as zero, which the venue uses for no-trade intervals; a missing
    /// price makes the row unusable as a bar and returns `None`.
    #[must_use]
    pub fn ohlcv(&self) -> Option<(f64, f64, f64, f64, f64)> {
        match (self.1, self.2, self.3, self.4) {
            (Some(open), Some(high), Some(low), Some(close)) => {
                Some((open, high, low, close, self.5.unwrap_or(0.0)))
            }
            _ => None,
        }
    }

    /// Returns the open interest, when the venue reported one.
    #[must_use]
    pub const fn open_interest(&self) -> Option<f64> {
        self.6
    }
}
