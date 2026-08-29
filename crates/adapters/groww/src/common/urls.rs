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

//! URLs for the Groww REST API, instrument master, and streaming feed.

/// Base URL for the Groww REST API.
pub const GROWW_REST_URL: &str = "https://api.groww.in/v1";

/// URL of the access-token endpoint.
///
/// Groww caps this endpoint at 150 requests per 24 hours, so a token is reused for its lifetime
/// rather than minted per request.
pub const GROWW_ACCESS_TOKEN_URL: &str = "https://api.groww.in/v1/token/api/access";

/// URL of the socket-token endpoint.
///
/// The trailing slash is deliberately absent: the slashed form answers with a 307 redirect that
/// drops the POST body on clients which do not re-send it.
pub const GROWW_SOCKET_TOKEN_URL: &str = "https://api.groww.in/v1/api/apex/v1/socket/token/create";

/// URL of the public instrument master CSV.
///
/// Served without authentication and refreshed by the venue daily.
pub const GROWW_INSTRUMENTS_CSV_URL: &str =
    "https://growwapi-assets.groww.in/instruments/instrument.csv";

/// URL of the streaming feed, a NATS server reached over WebSocket.
pub const GROWW_WS_URL: &str = "wss://socket-api.groww.in";

/// Returns the base REST URL, preferring `base_url` when provided.
#[must_use]
pub fn get_http_base_url(base_url: Option<&str>) -> String {
    base_url.unwrap_or(GROWW_REST_URL).to_string()
}

/// Returns the streaming feed URL, preferring `base_url` when provided.
#[must_use]
pub fn get_ws_url(base_url: Option<&str>) -> String {
    base_url.unwrap_or(GROWW_WS_URL).to_string()
}
