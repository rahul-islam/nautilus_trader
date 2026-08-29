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

//! Request bodies and query parameters for the Groww REST API.

use serde::{Deserialize, Serialize};

use crate::common::enums::{
    GrowwExchange, GrowwOrderType, GrowwProduct, GrowwSegment, GrowwTransactionType, GrowwValidity,
};

/// Request body for `POST /order/create`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateOrderRequest {
    /// Venue trading symbol.
    pub trading_symbol: String,
    /// Quantity to trade, in units for equities and in contracts for derivatives.
    pub quantity: u64,
    /// Limit price. Sent as zero for market orders, which is what the venue expects.
    pub price: f64,
    /// Trigger price for stop orders.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_price: Option<f64>,
    /// Time in force.
    pub validity: GrowwValidity,
    /// Exchange to route to.
    pub exchange: GrowwExchange,
    /// Trading segment.
    pub segment: GrowwSegment,
    /// Product type governing settlement.
    pub product: GrowwProduct,
    /// Order type.
    pub order_type: GrowwOrderType,
    /// Side of the order.
    pub transaction_type: GrowwTransactionType,
    /// Client-supplied reference used to correlate the order back to a client order ID.
    ///
    /// The venue constrains this to 8-20 alphanumeric characters with at most two hyphens.
    pub order_reference_id: String,
}

/// Request body for `POST /order/modify`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModifyOrderRequest {
    /// Venue-assigned order identifier.
    pub groww_order_id: String,
    /// Trading segment, which the venue requires to locate the order.
    pub segment: GrowwSegment,
    /// Order type after the modification.
    pub order_type: GrowwOrderType,
    /// Quantity after the modification.
    pub quantity: u64,
    /// Limit price after the modification.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<f64>,
    /// Trigger price after the modification.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_price: Option<f64>,
}

/// Request body for `POST /order/cancel`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelOrderRequest {
    /// Venue-assigned order identifier.
    pub groww_order_id: String,
    /// Trading segment, which the venue requires to locate the order.
    pub segment: GrowwSegment,
}

/// Request body for `POST /api/apex/v1/socket/token/create`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocketTokenRequest {
    /// NKEY-encoded public key the feed will authenticate against.
    #[serde(rename = "socketKey")]
    pub socket_key: String,
}

/// Request body for `POST /token/api/access` using the API key and secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessTokenRequest {
    /// Discriminator selecting the approval (key and secret) flow.
    pub key_type: &'static str,
    /// SHA-256 checksum over the secret and timestamp.
    pub checksum: String,
    /// UNIX epoch seconds the checksum was computed against.
    pub timestamp: i64,
}

impl AccessTokenRequest {
    /// Creates a request for the approval flow.
    #[must_use]
    pub const fn approval(checksum: String, timestamp: i64) -> Self {
        Self {
            key_type: "approval",
            checksum,
            timestamp,
        }
    }
}
