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

//! Enumerations modeling the Groww venue domain.

use nautilus_model::enums::{OrderSide, OrderStatus, OrderType, TimeInForce};
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumIter, EnumString};

/// Exchange an instrument is listed and traded on.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    EnumIter,
    AsRefStr,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
pub enum GrowwExchange {
    /// National Stock Exchange of India.
    Nse,
    /// BSE (formerly Bombay Stock Exchange).
    Bse,
    /// Multi Commodity Exchange of India.
    Mcx,
}

/// Trading segment an instrument belongs to.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    EnumIter,
    AsRefStr,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
pub enum GrowwSegment {
    /// Cash (equity delivery and intraday) segment.
    Cash,
    /// Futures and options segment.
    Fno,
    /// Currency derivatives segment.
    Currency,
    /// Commodity derivatives segment.
    Commodity,
}

/// Product type, which determines the settlement and margin treatment of a position.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    EnumIter,
    AsRefStr,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
pub enum GrowwProduct {
    /// Cash and carry: delivery-settled equity held in the demat account.
    Cnc,
    /// Margin intraday square-off: auto-squared by the venue before the close.
    Mis,
    /// Normal: carry-forward derivatives position.
    Nrml,
    /// Margin trading facility: broker-funded delivery position.
    Mtf,
}

/// Order type as understood by the venue.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    EnumIter,
    AsRefStr,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
pub enum GrowwOrderType {
    /// Market order.
    Market,
    /// Limit order.
    Limit,
    /// Stop-loss limit order, triggered at `trigger_price` and placed at `price`.
    #[serde(rename = "SL")]
    #[strum(serialize = "SL")]
    StopLoss,
    /// Stop-loss market order, triggered at `trigger_price`.
    #[serde(rename = "SL_M")]
    #[strum(serialize = "SL_M")]
    StopLossMarket,
}

/// Order side as understood by the venue.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    EnumIter,
    AsRefStr,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
pub enum GrowwTransactionType {
    /// Buy.
    Buy,
    /// Sell.
    Sell,
}

/// Time in force accepted by the venue.
///
/// Groww accepts only `DAY` on the REST order endpoints today. The remaining values are modeled
/// because the order-update feed reports them, and rejecting an unmodeled value on ingest would
/// drop otherwise valid events.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    EnumIter,
    AsRefStr,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
pub enum GrowwValidity {
    /// Good for the trading day.
    Day,
    /// Immediate or cancel.
    Ioc,
    /// Good until cancelled.
    Gtc,
    /// Good until a specified date.
    Gtd,
    /// Good until the end of session.
    Eos,
}

/// Lifecycle status of an order at the venue.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    EnumIter,
    AsRefStr,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
pub enum GrowwOrderStatus {
    /// Accepted by Groww but not yet acknowledged by the exchange.
    New,
    /// Acknowledged by the exchange.
    Acked,
    /// Stop order resting until its trigger price is reached.
    TriggerPending,
    /// Approved for routing.
    Approved,
    /// Rejected by the exchange or by risk checks.
    Rejected,
    /// Failed before reaching the exchange.
    Failed,
    /// Fully filled.
    Executed,
    /// Filled and awaiting delivery settlement.
    DeliveryAwaited,
    /// Cancelled.
    Cancelled,
    /// Cancellation requested but not yet confirmed.
    CancellationRequested,
    /// Modification requested but not yet confirmed.
    ModificationRequested,
    /// Terminal completed state.
    Completed,
}

impl GrowwOrderStatus {
    /// Returns whether the status is terminal, meaning no further transitions are expected.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Rejected | Self::Failed | Self::Cancelled | Self::Completed
        )
    }
}

/// Instrument type as published in the instrument master.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    EnumIter,
    AsRefStr,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
pub enum GrowwInstrumentType {
    /// Equity cash instrument.
    #[serde(rename = "EQ")]
    #[strum(serialize = "EQ")]
    Equity,
    /// Futures contract.
    #[serde(rename = "FUT")]
    #[strum(serialize = "FUT")]
    Future,
    /// Call option contract.
    #[serde(rename = "CE")]
    #[strum(serialize = "CE")]
    Call,
    /// Put option contract.
    #[serde(rename = "PE")]
    #[strum(serialize = "PE")]
    Put,
    /// Index. Not directly tradable, but quoted on the feed and used as a derivative underlying.
    #[serde(rename = "IDX")]
    #[strum(serialize = "IDX")]
    Index,
}

impl From<GrowwTransactionType> for OrderSide {
    fn from(value: GrowwTransactionType) -> Self {
        match value {
            GrowwTransactionType::Buy => Self::Buy,
            GrowwTransactionType::Sell => Self::Sell,
        }
    }
}

impl From<GrowwOrderType> for OrderType {
    fn from(value: GrowwOrderType) -> Self {
        match value {
            GrowwOrderType::Market => Self::Market,
            GrowwOrderType::Limit => Self::Limit,
            GrowwOrderType::StopLoss => Self::StopLimit,
            GrowwOrderType::StopLossMarket => Self::StopMarket,
        }
    }
}

impl From<GrowwValidity> for TimeInForce {
    fn from(value: GrowwValidity) -> Self {
        match value {
            GrowwValidity::Day => Self::Day,
            GrowwValidity::Ioc => Self::Ioc,
            GrowwValidity::Gtc => Self::Gtc,
            GrowwValidity::Gtd => Self::Gtd,
            // End-of-session expires with the trading day, which is what `Day` means here.
            GrowwValidity::Eos => Self::Day,
        }
    }
}

impl From<GrowwOrderStatus> for OrderStatus {
    fn from(value: GrowwOrderStatus) -> Self {
        match value {
            GrowwOrderStatus::New => Self::Submitted,
            GrowwOrderStatus::Acked | GrowwOrderStatus::Approved => Self::Accepted,
            GrowwOrderStatus::TriggerPending => Self::Accepted,
            GrowwOrderStatus::Rejected | GrowwOrderStatus::Failed => Self::Rejected,
            GrowwOrderStatus::Executed
            | GrowwOrderStatus::DeliveryAwaited
            | GrowwOrderStatus::Completed => Self::Filled,
            GrowwOrderStatus::Cancelled => Self::Canceled,
            GrowwOrderStatus::CancellationRequested => Self::PendingCancel,
            GrowwOrderStatus::ModificationRequested => Self::PendingUpdate,
        }
    }
}
