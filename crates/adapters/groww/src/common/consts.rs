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

//! Constants for the Groww integration.

use std::sync::LazyLock;

use nautilus_model::identifiers::{ClientId, Venue};
use ustr::Ustr;

/// Client identifier for the Groww adapter.
pub const GROWW: &str = "GROWW";

/// Exchange identifier for the National Stock Exchange of India.
pub const NSE: &str = "NSE";

/// Exchange identifier for the BSE (formerly Bombay Stock Exchange).
pub const BSE: &str = "BSE";

/// Exchange identifier for the Multi Commodity Exchange of India.
pub const MCX: &str = "MCX";

/// Static client ID instance.
pub static GROWW_CLIENT_ID: LazyLock<ClientId> = LazyLock::new(|| ClientId::new(GROWW));

/// Static venue instance for the National Stock Exchange of India.
pub static NSE_VENUE: LazyLock<Venue> = LazyLock::new(|| Venue::new(Ustr::from(NSE)));

/// Static venue instance for the BSE.
pub static BSE_VENUE: LazyLock<Venue> = LazyLock::new(|| Venue::new(Ustr::from(BSE)));

/// Static venue instance for the Multi Commodity Exchange of India.
pub static MCX_VENUE: LazyLock<Venue> = LazyLock::new(|| Venue::new(Ustr::from(MCX)));

/// Groww routes orders to several Indian exchanges, so the adapter models each exchange as its
/// own venue rather than a single broker venue. The same ticker listed on NSE and BSE has a
/// different exchange token and tick size, and is therefore a distinct instrument.
pub const SUPPORTED_EXCHANGES: [&str; 3] = [NSE, BSE, MCX];

/// Header name carrying the Groww API version.
pub const HEADER_API_VERSION: &str = "x-api-version";

/// Header name carrying a per-request correlation identifier.
pub const HEADER_REQUEST_ID: &str = "x-request-id";

/// Header name identifying the calling client.
pub const HEADER_CLIENT_ID: &str = "x-client-id";

/// Header name identifying the calling client platform.
pub const HEADER_CLIENT_PLATFORM: &str = "x-client-platform";

/// Header name carrying the calling client platform version.
pub const HEADER_CLIENT_PLATFORM_VERSION: &str = "x-client-platform-version";

/// Value sent for [`HEADER_API_VERSION`].
pub const API_VERSION: &str = "1.0";

/// Value sent for [`HEADER_CLIENT_ID`].
pub const CLIENT_ID_VALUE: &str = "growwapi";

/// Value sent for [`HEADER_CLIENT_PLATFORM`].
pub const CLIENT_PLATFORM: &str = "growwapi-rust-client";

/// Maximum number of concurrent feed subscriptions accepted by the venue.
pub const MAX_FEED_SUBSCRIPTIONS: usize = 1_000;

/// Query parameter key for the trading segment.
pub const QUERY_KEY_SEGMENT: &str = "segment";

/// Query parameter key for a result page index.
pub const QUERY_KEY_PAGE: &str = "page";

/// Query parameter key for a result page size.
pub const QUERY_KEY_PAGE_SIZE: &str = "page_size";

/// Maximum page size accepted by the paginated order and trade endpoints.
pub const MAX_PAGE_SIZE: u32 = 100;

/// Query parameter key for the exchange.
pub const QUERY_KEY_EXCHANGE: &str = "exchange";

/// Query parameter key for a venue trading symbol.
pub const QUERY_KEY_TRADING_SYMBOL: &str = "trading_symbol";

/// Query parameter key for a Groww symbol, the venue's cross-exchange instrument key.
pub const QUERY_KEY_GROWW_SYMBOL: &str = "groww_symbol";

/// Query parameter key for a comma-separated list of `EXCHANGE_SYMBOL` keys.
pub const QUERY_KEY_EXCHANGE_SYMBOLS: &str = "exchange_symbols";

/// Query parameter key for a range start, as naive Indian local time.
pub const QUERY_KEY_START_TIME: &str = "start_time";

/// Query parameter key for a range end, as naive Indian local time.
pub const QUERY_KEY_END_TIME: &str = "end_time";

/// Query parameter key for the historical candle interval.
pub const QUERY_KEY_CANDLE_INTERVAL: &str = "candle_interval";
