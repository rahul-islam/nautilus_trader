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

//! Configuration for the Groww data and execution clients.

use nautilus_model::identifiers::AccountId;
use serde::{Deserialize, Serialize};

use crate::common::enums::{GrowwExchange, GrowwInstrumentType, GrowwSegment};

/// Configuration for [`crate::data::GrowwDataClient`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(module = "nautilus_trader.adapters.groww", from_py_object)
)]
#[cfg_attr(
    feature = "python",
    pyo3_stub_gen::derive::gen_stub_pyclass(module = "nautilus_trader.adapters.groww")
)]
pub struct GrowwDataClientConfig {
    /// API key. Falls back to `GROWW_API_KEY` when unset.
    pub api_key: Option<String>,
    /// API secret. Falls back to `GROWW_API_SECRET` when unset.
    pub api_secret: Option<String>,
    /// Base REST URL override, for testing against a mock venue.
    pub base_url_http: Option<String>,
    /// Feed URL override, for testing against a mock venue.
    pub base_url_ws: Option<String>,
    /// HTTP request timeout (seconds).
    pub http_timeout_secs: u64,
    /// Exchanges to load from the instrument master. Empty loads every exchange.
    pub exchanges: Vec<GrowwExchange>,
    /// Segments to load from the instrument master. Empty loads every segment.
    pub segments: Vec<GrowwSegment>,
    /// Instrument types to load from the instrument master. Empty loads every type.
    pub instrument_types: Vec<GrowwInstrumentType>,
}

impl Default for GrowwDataClientConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            api_secret: None,
            base_url_http: None,
            base_url_ws: None,
            http_timeout_secs: 60,
            // The full master exceeds 135,000 rows, most of them options; loading the cash
            // segment by default keeps startup fast for the common equity case.
            exchanges: vec![GrowwExchange::Nse, GrowwExchange::Bse],
            segments: vec![GrowwSegment::Cash],
            instrument_types: vec![],
        }
    }
}

/// Configuration for the Groww execution client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(module = "nautilus_trader.adapters.groww", from_py_object)
)]
#[cfg_attr(
    feature = "python",
    pyo3_stub_gen::derive::gen_stub_pyclass(module = "nautilus_trader.adapters.groww")
)]
pub struct GrowwExecClientConfig {
    /// API key. Falls back to `GROWW_API_KEY` when unset.
    pub api_key: Option<String>,
    /// API secret. Falls back to `GROWW_API_SECRET` when unset.
    pub api_secret: Option<String>,
    /// Base REST URL override, for testing against a mock venue.
    pub base_url_http: Option<String>,
    /// Feed URL override, for testing against a mock venue.
    pub base_url_ws: Option<String>,
    /// HTTP request timeout (seconds).
    pub http_timeout_secs: u64,
    /// Account identifier, defaulting to `GROWW-001`.
    pub account_id: Option<AccountId>,
    /// Product type sent with orders: `CNC` for delivery or `MIS` for intraday.
    pub product: crate::common::enums::GrowwProduct,
    /// Exchanges to load from the instrument master. Empty loads every exchange.
    pub exchanges: Vec<GrowwExchange>,
    /// Segments to load from the instrument master. Empty loads every segment.
    pub segments: Vec<GrowwSegment>,
}

impl Default for GrowwExecClientConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            api_secret: None,
            base_url_http: None,
            base_url_ws: None,
            http_timeout_secs: 60,
            account_id: None,
            product: crate::common::enums::GrowwProduct::Cnc,
            exchanges: vec![GrowwExchange::Nse, GrowwExchange::Bse],
            segments: vec![GrowwSegment::Cash],
        }
    }
}
