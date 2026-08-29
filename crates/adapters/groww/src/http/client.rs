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

//! HTTP client for the Groww REST API.
//!
//! Groww authenticates REST calls with a bearer access token minted from the API key and secret.
//! The token endpoint is capped at 150 requests per 24 hours, so [`GrowwHttpClient`] mints a token
//! once and reuses it, refreshing only when the venue rejects it as expired.

use std::{
    collections::HashMap,
    num::NonZeroU32,
    sync::{Arc, LazyLock},
};

use nautilus_core::{UnixNanos, consts::NAUTILUS_USER_AGENT, time::get_atomic_clock_realtime};
use nautilus_network::{
    http::{HttpClient, HttpClientError, HttpResponse, Method},
    ratelimiter::quota::Quota,
    retry::{RetryConfig, RetryManager},
};
use serde::{Serialize, de::DeserializeOwned};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::{
    common::{
        consts::{
            API_VERSION, CLIENT_ID_VALUE, CLIENT_PLATFORM, HEADER_API_VERSION, HEADER_CLIENT_ID,
            HEADER_CLIENT_PLATFORM, HEADER_CLIENT_PLATFORM_VERSION, HEADER_REQUEST_ID,
            MAX_PAGE_SIZE, QUERY_KEY_CANDLE_INTERVAL, QUERY_KEY_END_TIME, QUERY_KEY_EXCHANGE,
            QUERY_KEY_EXCHANGE_SYMBOLS, QUERY_KEY_GROWW_SYMBOL, QUERY_KEY_PAGE,
            QUERY_KEY_PAGE_SIZE, QUERY_KEY_SEGMENT, QUERY_KEY_START_TIME, QUERY_KEY_TRADING_SYMBOL,
        },
        credential::{GrowwCredential, NKeyPair},
        enums::{GrowwExchange, GrowwSegment},
        parse::parse_naive_india,
        urls,
    },
    http::{
        error::{Error, Result},
        models::{
            AccessToken, Candle, CandlesPayload, GrowwResponse, GrowwStatus, Holding,
            HoldingsPayload, Ohlc, Order, OrderAck, OrderListPayload, Position, PositionsPayload,
            Quote, SocketToken, Trade, TradeListPayload, UserDetail, UserMargin,
        },
        query::{
            AccessTokenRequest, CancelOrderRequest, CreateOrderRequest, ModifyOrderRequest,
            SocketTokenRequest,
        },
    },
};

/// Rate limit key for order placement, modification, and cancellation.
pub const RATE_KEY_ORDERS: &str = "orders";
/// Rate limit key for live market data endpoints.
pub const RATE_KEY_LIVE_DATA: &str = "live_data";
/// Rate limit key for non-trading endpoints.
pub const RATE_KEY_NON_TRADING: &str = "non_trading";
/// Rate limit key for authentication endpoints.
pub const RATE_KEY_AUTH: &str = "auth";

fn nonzero(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).expect("rate limit constants are non-zero")
}

/// Returns the venue's documented rate limits, keyed by endpoint category.
///
/// Groww publishes both a per-second and a per-minute ceiling for each category, and the
/// per-minute ceiling is the tighter of the two (250 orders per minute against 10 per second).
/// Each quota therefore replenishes at the per-minute rate while allowing a burst up to the
/// per-second ceiling, so a short burst is permitted without exceeding the sustained limit.
pub static GROWW_RATE_QUOTAS: LazyLock<Vec<(String, Quota)>> = LazyLock::new(|| {
    vec![
        (
            RATE_KEY_ORDERS.to_string(),
            Quota::per_minute(nonzero(250)).allow_burst(nonzero(10)),
        ),
        (
            RATE_KEY_LIVE_DATA.to_string(),
            Quota::per_minute(nonzero(300)).allow_burst(nonzero(10)),
        ),
        (
            RATE_KEY_NON_TRADING.to_string(),
            Quota::per_minute(nonzero(500)).allow_burst(nonzero(20)),
        ),
        (
            RATE_KEY_AUTH.to_string(),
            Quota::per_minute(nonzero(30)).allow_burst(nonzero(5)),
        ),
    ]
});

/// Default quota applied to any request that does not carry a category key.
pub static GROWW_DEFAULT_QUOTA: LazyLock<Quota> =
    LazyLock::new(|| Quota::per_minute(nonzero(500)).allow_burst(nonzero(20)));

/// Returns the default retry configuration for the Groww HTTP client.
#[must_use]
pub fn default_retry_config() -> RetryConfig {
    RetryConfig {
        max_retries: 3,
        initial_delay_ms: 100,
        max_delay_ms: 5_000,
        backoff_factor: 2.0,
        jitter_ms: 250,
        operation_timeout_ms: Some(60_000),
        immediate_first: false,
        max_elapsed_ms: Some(180_000),
    }
}

/// Cached access token together with the instant it stops being usable.
#[derive(Debug, Default)]
struct TokenState {
    token: Option<String>,
    /// Expiry reported by the venue, when it supplied one.
    expires_at: Option<UnixNanos>,
}

/// Refresh margin applied to a token's expiry.
///
/// Groww expires tokens on a daily boundary rather than after a fixed lifetime, so a token can be
/// close to expiry when first minted. Refreshing early avoids sending a request that is certain to
/// be rejected, at the cost of one extra call against a 150-per-day endpoint.
const TOKEN_REFRESH_MARGIN_NANOS: u64 = 60 * 1_000_000_000;

/// HTTP client for the Groww REST API.
#[derive(Debug)]
pub struct GrowwHttpClient {
    client: HttpClient,
    credential: Option<GrowwCredential>,
    base_url: String,
    retry_manager: RetryManager<Error>,
    cancellation_token: CancellationToken,
    token: Arc<Mutex<TokenState>>,
}

impl GrowwHttpClient {
    /// Creates a new [`GrowwHttpClient`] without credentials, for public endpoints only.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying HTTP client cannot be created.
    pub fn new(
        base_url: Option<&str>,
        timeout_secs: u64,
        proxy_url: Option<String>,
        retry_config: Option<RetryConfig>,
    ) -> std::result::Result<Self, HttpClientError> {
        Self::build(None, base_url, timeout_secs, proxy_url, retry_config)
    }

    /// Creates a new [`GrowwHttpClient`] with credentials for authenticated requests.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying HTTP client cannot be created.
    pub fn with_credentials(
        credential: GrowwCredential,
        base_url: Option<&str>,
        timeout_secs: u64,
        proxy_url: Option<String>,
        retry_config: Option<RetryConfig>,
    ) -> std::result::Result<Self, HttpClientError> {
        Self::build(
            Some(credential),
            base_url,
            timeout_secs,
            proxy_url,
            retry_config,
        )
    }

    fn build(
        credential: Option<GrowwCredential>,
        base_url: Option<&str>,
        timeout_secs: u64,
        proxy_url: Option<String>,
        retry_config: Option<RetryConfig>,
    ) -> std::result::Result<Self, HttpClientError> {
        Ok(Self {
            client: HttpClient::builder()
                .headers(Self::default_headers())
                .keyed_quotas(GROWW_RATE_QUOTAS.clone())
                .default_quota(*GROWW_DEFAULT_QUOTA)
                .timeout_secs(timeout_secs)
                .maybe_proxy_url(proxy_url)
                .build()?,
            credential,
            base_url: urls::get_http_base_url(base_url),
            retry_manager: RetryManager::new(retry_config.unwrap_or_else(default_retry_config)),
            cancellation_token: CancellationToken::new(),
            token: Arc::new(Mutex::new(TokenState::default())),
        })
    }

    fn default_headers() -> HashMap<String, String> {
        HashMap::from([
            ("user-agent".to_string(), NAUTILUS_USER_AGENT.to_string()),
            ("content-type".to_string(), "application/json".to_string()),
            (HEADER_API_VERSION.to_string(), API_VERSION.to_string()),
            (HEADER_CLIENT_ID.to_string(), CLIENT_ID_VALUE.to_string()),
            (
                HEADER_CLIENT_PLATFORM.to_string(),
                CLIENT_PLATFORM.to_string(),
            ),
            (
                HEADER_CLIENT_PLATFORM_VERSION.to_string(),
                env!("CARGO_PKG_VERSION").to_string(),
            ),
        ])
    }

    /// Returns the base URL in use.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Returns whether the client holds credentials.
    #[must_use]
    pub const fn has_credentials(&self) -> bool {
        self.credential.is_some()
    }

    /// Returns the cancellation token used to abort in-flight retries.
    #[must_use]
    pub const fn cancellation_token(&self) -> &CancellationToken {
        &self.cancellation_token
    }

    /// Cancels all in-flight requests and stops further retries.
    pub fn cancel_all_requests(&self) {
        self.cancellation_token.cancel();
    }

    fn credential(&self) -> Result<&GrowwCredential> {
        crate::common::credential::require_credential(self.credential.as_ref())
    }

    /// Returns a bearer access token, minting one if the cache is empty.
    ///
    /// # Errors
    ///
    /// Returns an error if credentials are absent or the venue rejects them.
    pub async fn access_token(&self) -> Result<String> {
        let mut state = self.token.lock().await;
        let now = get_atomic_clock_realtime().get_time_ns();

        if let Some(token) = &state.token {
            let still_valid = state.expires_at.is_none_or(|expiry| {
                expiry.as_u64().saturating_sub(TOKEN_REFRESH_MARGIN_NANOS) > now.as_u64()
            });
            if still_valid {
                return Ok(token.clone());
            }
        }

        let issued = self.mint_access_token().await?;
        state.token = Some(issued.token.clone());
        state.expires_at =
            issued
                .expiry
                .as_deref()
                .and_then(|expiry| match parse_naive_india(expiry) {
                    Ok(ts) => Some(ts),
                    Err(e) => {
                        // An unparsable expiry is not fatal: the token still works until the venue
                        // rejects it, and treating it as non-expiring simply defers the refresh.
                        tracing::warn!("Ignoring unparsable Groww token expiry `{expiry}`: {e}");
                        None
                    }
                });
        Ok(issued.token)
    }

    /// Discards the cached access token so the next request mints a fresh one.
    pub async fn invalidate_access_token(&self) {
        let mut state = self.token.lock().await;
        state.token = None;
        state.expires_at = None;
    }

    /// Mints a new access token from the API key and secret.
    ///
    /// # Errors
    ///
    /// Returns an error if credentials are absent or the venue rejects them.
    async fn mint_access_token(&self) -> Result<AccessToken> {
        let credential = self.credential()?;
        let timestamp = get_atomic_clock_realtime().get_time_ns().as_i64() / 1_000_000_000;
        let body = AccessTokenRequest::approval(credential.checksum(timestamp), timestamp);

        // The token endpoint authorizes with the API key itself rather than a bearer token.
        let mut headers = HashMap::new();
        headers.insert(
            "authorization".to_string(),
            format!("Bearer {}", credential.api_key()),
        );
        headers.insert(HEADER_REQUEST_ID.to_string(), new_request_id());

        let response = self
            .client
            .request(
                Method::POST,
                urls::GROWW_ACCESS_TOKEN_URL.to_string(),
                None,
                Some(headers),
                Some(serde_json::to_vec(&body)?),
                None,
                Some(vec![RATE_KEY_AUTH.to_string()]),
            )
            .await?;

        // This endpoint answers with the token object directly rather than the shared envelope.
        let status = response.status.as_u16();
        if !(200..300).contains(&status) {
            return Err(Error::auth(format!(
                "token request failed with HTTP {status}: {}",
                String::from_utf8_lossy(&response.body)
            )));
        }

        serde_json::from_slice(&response.body).map_err(|e| {
            Error::decode(format!(
                "failed to decode access token: {e}: {}",
                String::from_utf8_lossy(&response.body)
            ))
        })
    }

    /// Sends an authenticated request and decodes the response envelope.
    async fn send<T, B>(
        &self,
        method: Method,
        path: &str,
        params: Option<&HashMap<String, Vec<String>>>,
        body: Option<&B>,
        rate_key: &str,
    ) -> Result<T>
    where
        T: DeserializeOwned,
        B: Serialize + ?Sized,
    {
        let url = format!("{}{path}", self.base_url);
        let body_bytes = body.map(serde_json::to_vec).transpose()?;

        let operation = || {
            let url = url.clone();
            let method = method.clone();
            let body_bytes = body_bytes.clone();
            let keys = vec![rate_key.to_string()];
            async move {
                let token = self.access_token().await?;
                let mut headers = HashMap::new();
                headers.insert("authorization".to_string(), format!("Bearer {token}"));
                headers.insert(HEADER_REQUEST_ID.to_string(), new_request_id());

                let response = self
                    .client
                    .request(
                        method,
                        url,
                        params,
                        Some(headers),
                        body_bytes,
                        None,
                        Some(keys),
                    )
                    .await?;
                deserialize_envelope::<T>(&response)
            }
        };

        self.retry_manager
            .execute_with_retry_with_cancel(
                path,
                operation,
                Error::is_retryable,
                |e| Error::transport(e.to_string()),
                &self.cancellation_token,
            )
            .await
    }

    /// Requests the authenticated user's account detail.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the response cannot be decoded.
    pub async fn http_get_user_detail(&self) -> Result<UserDetail> {
        self.send::<UserDetail, ()>(
            Method::GET,
            "/user/detail",
            None,
            None,
            RATE_KEY_NON_TRADING,
        )
        .await
    }

    /// Requests the account's margin and balance detail.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the response cannot be decoded.
    pub async fn http_get_margin(&self) -> Result<UserMargin> {
        self.send::<UserMargin, ()>(
            Method::GET,
            "/margins/detail/user",
            None,
            None,
            RATE_KEY_NON_TRADING,
        )
        .await
    }

    /// Requests the account's demat holdings.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the response cannot be decoded.
    pub async fn http_get_holdings(&self) -> Result<Vec<Holding>> {
        let payload: HoldingsPayload = self
            .send::<HoldingsPayload, ()>(
                Method::GET,
                "/holdings/user",
                None,
                None,
                RATE_KEY_NON_TRADING,
            )
            .await?;
        Ok(payload.holdings)
    }

    /// Requests the account's open positions.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the response cannot be decoded.
    pub async fn http_get_positions(&self) -> Result<Vec<Position>> {
        let payload: PositionsPayload = self
            .send::<PositionsPayload, ()>(
                Method::GET,
                "/positions/user",
                None,
                None,
                RATE_KEY_NON_TRADING,
            )
            .await?;
        Ok(payload.positions)
    }

    /// Requests a page of orders for `segment`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the response cannot be decoded.
    pub async fn http_get_order_list(
        &self,
        segment: GrowwSegment,
        page: u32,
        page_size: u32,
    ) -> Result<Vec<Order>> {
        let params = HashMap::from([
            (QUERY_KEY_SEGMENT.to_string(), vec![segment.to_string()]),
            (QUERY_KEY_PAGE.to_string(), vec![page.to_string()]),
            (
                QUERY_KEY_PAGE_SIZE.to_string(),
                vec![page_size.min(MAX_PAGE_SIZE).to_string()],
            ),
        ]);
        let payload: OrderListPayload = self
            .send::<OrderListPayload, ()>(
                Method::GET,
                "/order/list",
                Some(&params),
                None,
                RATE_KEY_NON_TRADING,
            )
            .await?;
        Ok(payload.order_list)
    }

    /// Requests the full detail of a single order.
    ///
    /// The lighter `/order/status` endpoint answers with only five fields; this endpoint returns
    /// the complete order object needed to build a status report.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the response cannot be decoded.
    pub async fn http_get_order_detail(
        &self,
        groww_order_id: &str,
        segment: GrowwSegment,
    ) -> Result<Order> {
        let params = HashMap::from([(QUERY_KEY_SEGMENT.to_string(), vec![segment.to_string()])]);
        self.send::<Order, ()>(
            Method::GET,
            &format!("/order/detail/{groww_order_id}"),
            Some(&params),
            None,
            RATE_KEY_NON_TRADING,
        )
        .await
    }

    /// Requests the abbreviated status of a single order.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the response cannot be decoded.
    pub async fn http_get_order_status(
        &self,
        groww_order_id: &str,
        segment: GrowwSegment,
    ) -> Result<Order> {
        let params = HashMap::from([(QUERY_KEY_SEGMENT.to_string(), vec![segment.to_string()])]);
        self.send::<Order, ()>(
            Method::GET,
            &format!("/order/status/{groww_order_id}"),
            Some(&params),
            None,
            RATE_KEY_NON_TRADING,
        )
        .await
    }

    /// Requests the status of an order by the client-supplied reference.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the response cannot be decoded.
    pub async fn http_get_order_status_by_reference(
        &self,
        order_reference_id: &str,
        segment: GrowwSegment,
    ) -> Result<Order> {
        let params = HashMap::from([(QUERY_KEY_SEGMENT.to_string(), vec![segment.to_string()])]);
        self.send::<Order, ()>(
            Method::GET,
            &format!("/order/status/reference/{order_reference_id}"),
            Some(&params),
            None,
            RATE_KEY_NON_TRADING,
        )
        .await
    }

    /// Requests the trades booked against an order.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the response cannot be decoded.
    pub async fn http_get_order_trades(
        &self,
        groww_order_id: &str,
        segment: GrowwSegment,
        page: u32,
        page_size: u32,
    ) -> Result<Vec<Trade>> {
        let params = HashMap::from([
            (QUERY_KEY_SEGMENT.to_string(), vec![segment.to_string()]),
            (QUERY_KEY_PAGE.to_string(), vec![page.to_string()]),
            (
                QUERY_KEY_PAGE_SIZE.to_string(),
                vec![page_size.min(MAX_PAGE_SIZE).to_string()],
            ),
        ]);
        let payload: TradeListPayload = self
            .send::<TradeListPayload, ()>(
                Method::GET,
                &format!("/order/trades/{groww_order_id}"),
                Some(&params),
                None,
                RATE_KEY_NON_TRADING,
            )
            .await?;
        Ok(payload.trade_list)
    }

    /// Places an order.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the venue rejects the order.
    pub async fn http_place_order(&self, request: &CreateOrderRequest) -> Result<OrderAck> {
        self.send(
            Method::POST,
            "/order/create",
            None,
            Some(request),
            RATE_KEY_ORDERS,
        )
        .await
    }

    /// Modifies a working order.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the venue rejects the modification.
    pub async fn http_modify_order(&self, request: &ModifyOrderRequest) -> Result<OrderAck> {
        self.send(
            Method::POST,
            "/order/modify",
            None,
            Some(request),
            RATE_KEY_ORDERS,
        )
        .await
    }

    /// Cancels a working order.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the venue rejects the cancellation.
    pub async fn http_cancel_order(&self, request: &CancelOrderRequest) -> Result<OrderAck> {
        self.send(
            Method::POST,
            "/order/cancel",
            None,
            Some(request),
            RATE_KEY_ORDERS,
        )
        .await
    }

    /// Requests feed credentials for `key_pair`.
    ///
    /// Only the public half of `key_pair` is sent; the seed stays in this process.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the response cannot be decoded.
    pub async fn http_create_socket_token(&self, key_pair: &NKeyPair) -> Result<SocketToken> {
        let request = SocketTokenRequest {
            socket_key: key_pair.public_key(),
        };
        let token = self.access_token().await?;
        let mut headers = HashMap::new();
        headers.insert("authorization".to_string(), format!("Bearer {token}"));
        headers.insert(HEADER_REQUEST_ID.to_string(), new_request_id());

        let response = self
            .client
            .request(
                Method::POST,
                urls::GROWW_SOCKET_TOKEN_URL.to_string(),
                None,
                Some(headers),
                Some(serde_json::to_vec(&request)?),
                None,
                Some(vec![RATE_KEY_NON_TRADING.to_string()]),
            )
            .await?;

        // Like the access-token endpoint, this answers with the object directly rather than the
        // shared envelope.
        let status = response.status.as_u16();
        if !(200..300).contains(&status) {
            return Err(Error::Http {
                status,
                message: String::from_utf8_lossy(&response.body).into_owned(),
            });
        }
        serde_json::from_slice(&response.body).map_err(|e| {
            Error::decode(format!(
                "failed to decode socket token: {e}: {}",
                String::from_utf8_lossy(&response.body)
            ))
        })
    }

    /// Requests the last traded price for a set of `EXCHANGE_SYMBOL` keys within one segment.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the response cannot be decoded.
    pub async fn http_get_ltp(
        &self,
        segment: GrowwSegment,
        exchange_symbols: &[String],
    ) -> Result<HashMap<String, f64>> {
        let params = HashMap::from([
            (QUERY_KEY_SEGMENT.to_string(), vec![segment.to_string()]),
            (
                QUERY_KEY_EXCHANGE_SYMBOLS.to_string(),
                vec![exchange_symbols.join(",")],
            ),
        ]);
        self.send::<HashMap<String, f64>, ()>(
            Method::GET,
            "/live-data/ltp",
            Some(&params),
            None,
            RATE_KEY_LIVE_DATA,
        )
        .await
    }

    /// Requests the session OHLC for a set of `EXCHANGE_SYMBOL` keys within one segment.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the response cannot be decoded.
    pub async fn http_get_ohlc(
        &self,
        segment: GrowwSegment,
        exchange_symbols: &[String],
    ) -> Result<HashMap<String, Ohlc>> {
        let params = HashMap::from([
            (QUERY_KEY_SEGMENT.to_string(), vec![segment.to_string()]),
            (
                QUERY_KEY_EXCHANGE_SYMBOLS.to_string(),
                vec![exchange_symbols.join(",")],
            ),
        ]);
        self.send::<HashMap<String, Ohlc>, ()>(
            Method::GET,
            "/live-data/ohlc",
            Some(&params),
            None,
            RATE_KEY_LIVE_DATA,
        )
        .await
    }

    /// Requests the full quote for a single instrument, including five-level depth.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the response cannot be decoded.
    pub async fn http_get_quote(
        &self,
        exchange: GrowwExchange,
        segment: GrowwSegment,
        trading_symbol: &str,
    ) -> Result<Quote> {
        let params = HashMap::from([
            (QUERY_KEY_EXCHANGE.to_string(), vec![exchange.to_string()]),
            (QUERY_KEY_SEGMENT.to_string(), vec![segment.to_string()]),
            (
                QUERY_KEY_TRADING_SYMBOL.to_string(),
                vec![trading_symbol.to_string()],
            ),
        ]);
        self.send::<Quote, ()>(
            Method::GET,
            "/live-data/quote",
            Some(&params),
            None,
            RATE_KEY_LIVE_DATA,
        )
        .await
    }

    /// Requests historical candles for `groww_symbol` between two naive Indian local times.
    ///
    /// `start_time` and `end_time` use the venue's `yyyy-MM-dd HH:mm:ss` form and are interpreted
    /// in the Indian market time zone.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the response cannot be decoded.
    pub async fn http_get_candles(
        &self,
        exchange: GrowwExchange,
        segment: GrowwSegment,
        groww_symbol: &str,
        start_time: &str,
        end_time: &str,
        candle_interval: &str,
    ) -> Result<Vec<Candle>> {
        let params = HashMap::from([
            (QUERY_KEY_EXCHANGE.to_string(), vec![exchange.to_string()]),
            (QUERY_KEY_SEGMENT.to_string(), vec![segment.to_string()]),
            (
                QUERY_KEY_GROWW_SYMBOL.to_string(),
                vec![groww_symbol.to_string()],
            ),
            (
                QUERY_KEY_START_TIME.to_string(),
                vec![start_time.to_string()],
            ),
            (QUERY_KEY_END_TIME.to_string(), vec![end_time.to_string()]),
            (
                QUERY_KEY_CANDLE_INTERVAL.to_string(),
                vec![candle_interval.to_string()],
            ),
        ]);
        let payload: CandlesPayload = self
            .send::<CandlesPayload, ()>(
                Method::GET,
                "/historical/candles",
                Some(&params),
                None,
                RATE_KEY_LIVE_DATA,
            )
            .await?;
        Ok(payload.candles)
    }

    /// Downloads the public instrument master CSV.
    ///
    /// The endpoint is unauthenticated and returns CSV rather than the usual JSON envelope.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the response is not successful.
    pub async fn http_get_instruments_csv(&self) -> Result<String> {
        let response = self
            .client
            .request(
                Method::GET,
                urls::GROWW_INSTRUMENTS_CSV_URL.to_string(),
                None,
                None,
                None,
                None,
                Some(vec![RATE_KEY_NON_TRADING.to_string()]),
            )
            .await?;

        if !(200..300).contains(&response.status.as_u16()) {
            return Err(Error::Http {
                status: response.status.as_u16(),
                message: String::from_utf8_lossy(&response.body).into_owned(),
            });
        }

        String::from_utf8(response.body.to_vec())
            .map_err(|e| Error::decode(format!("instrument master is not valid UTF-8: {e}")))
    }
}

/// Returns a fresh correlation identifier for the `x-request-id` header.
fn new_request_id() -> String {
    nautilus_core::UUID4::new().to_string()
}

/// Decodes a Groww response envelope, mapping a failure envelope onto a typed error.
///
/// The venue answers some failures with a 200 status and a `FAILURE` envelope, so the status code
/// alone is not sufficient to detect an error.
fn deserialize_envelope<T: DeserializeOwned>(response: &HttpResponse) -> Result<T> {
    let status = response.status.as_u16();
    let body = &response.body;

    let envelope: GrowwResponse<T> = serde_json::from_slice(body).map_err(|e| {
        if (200..300).contains(&status) {
            Error::decode(format!(
                "failed to decode response: {e}: {}",
                String::from_utf8_lossy(body)
            ))
        } else {
            Error::Http {
                status,
                message: String::from_utf8_lossy(body).into_owned(),
            }
        }
    })?;

    match envelope.status {
        GrowwStatus::Success => envelope.payload.ok_or_else(|| {
            Error::decode("response reported success but carried no payload".to_string())
        }),
        GrowwStatus::Failure => {
            let detail = envelope.error;
            let message = detail
                .as_ref()
                .and_then(|e| e.message.clone())
                .unwrap_or_else(|| "unspecified venue error".to_string());
            let code = detail.as_ref().and_then(|e| e.code.clone());

            match code.as_deref() {
                Some("401" | "403") => Err(Error::auth(message)),
                Some("429") => Err(Error::RateLimit {
                    retry_after_ms: None,
                }),
                Some("400") => Err(Error::bad_request(message)),
                _ if (500..600).contains(&status) => Err(Error::Http { status, message }),
                _ => Err(Error::venue(message)),
            }
        }
    }
}
