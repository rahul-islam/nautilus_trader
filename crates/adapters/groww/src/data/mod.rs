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

//! Live market data client for Groww.
//!
//! Market data arrives on the streaming feed as depth and last-traded-price messages addressed by
//! exchange token. The client resolves tokens through the instrument provider, converts payloads
//! into Nautilus data events, and serves historical bar requests from the REST candle endpoint.
//!
//! The venue publishes no bar stream, so live bar subscriptions are not supported; subscribe to
//! quotes or trades and let the engine aggregate `INTERNAL` bars instead. Trade ticks are
//! synthesized from cumulative session volume, as documented on
//! [`crate::websocket::parse::TradeTickSynthesizer`].

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use ahash::AHashMap;
use nautilus_common::{
    clients::DataClient,
    live::{runner::get_data_event_sender, runtime::get_runtime},
    messages::{
        DataEvent,
        data::{
            BarsResponse, DataResponse, InstrumentResponse, InstrumentsResponse, RequestBars,
            RequestInstrument, RequestInstruments, SubscribeBookDepth10, SubscribeQuotes,
            SubscribeTrades, UnsubscribeBookDepth10, UnsubscribeQuotes, UnsubscribeTrades,
        },
    },
};
use nautilus_core::{
    datetime::datetime_to_unix_nanos,
    time::{AtomicTime, get_atomic_clock_realtime},
};
use nautilus_model::{
    data::{Bar, BarType, Data},
    enums::BarAggregation,
    identifiers::{ClientId, InstrumentId, Venue},
    instruments::{Instrument, InstrumentAny},
};
use tokio::{sync::RwLock, task::JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::{
    common::{consts::GROWW_CLIENT_ID, credential::GrowwCredential, parse::india_tz},
    config::GrowwDataClientConfig,
    http::client::GrowwHttpClient,
    provider::{GrowwInstrumentProvider, InstrumentFilter, InstrumentMetadata},
    websocket::{
        client::{FeedEvent, GrowwFeedClient},
        messages::{FeedKind, FeedMessage, market_data_subject},
        parse::{TradeTickSynthesizer, parse_depth10, parse_quote_tick},
    },
};

/// What a market data subject was subscribed for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubjectRole {
    /// Depth subject serving quote ticks.
    Quotes,
    /// Depth subject serving ten-level book snapshots.
    Depth,
    /// Live price subject serving synthesized trade ticks.
    Trades,
}

/// Routing entry for one subscribed subject.
#[derive(Debug, Clone)]
struct SubjectEntry {
    instrument_id: InstrumentId,
    roles: Vec<SubjectRole>,
}

/// Live market data client for Groww.
#[derive(Debug)]
pub struct GrowwDataClient {
    client_id: ClientId,
    config: GrowwDataClientConfig,
    http: Arc<GrowwHttpClient>,
    provider: GrowwInstrumentProvider,
    feed: Option<GrowwFeedClient>,
    routing: Arc<RwLock<AHashMap<String, SubjectEntry>>>,
    is_connected: Arc<AtomicBool>,
    cancellation_token: CancellationToken,
    tasks: Vec<JoinHandle<()>>,
    data_sender: tokio::sync::mpsc::UnboundedSender<DataEvent>,
    clock: &'static AtomicTime,
}

impl GrowwDataClient {
    /// Creates a new [`GrowwDataClient`].
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP client cannot be created.
    pub fn new(client_id: Option<ClientId>, config: GrowwDataClientConfig) -> anyhow::Result<Self> {
        let credential =
            GrowwCredential::resolve(config.api_key.clone(), config.api_secret.clone());
        let http = match credential {
            Some(credential) => GrowwHttpClient::with_credentials(
                credential,
                config.base_url_http.as_deref(),
                config.http_timeout_secs,
                None,
                None,
            )?,
            None => GrowwHttpClient::new(
                config.base_url_http.as_deref(),
                config.http_timeout_secs,
                None,
                None,
            )?,
        };
        let http = Arc::new(http);
        let provider = GrowwInstrumentProvider::new(Arc::clone(&http));

        Ok(Self {
            client_id: client_id.unwrap_or(*GROWW_CLIENT_ID),
            config,
            http,
            provider,
            feed: None,
            routing: Arc::new(RwLock::new(AHashMap::new())),
            is_connected: Arc::new(AtomicBool::new(false)),
            cancellation_token: CancellationToken::new(),
            tasks: Vec::new(),
            data_sender: get_data_event_sender(),
            clock: get_atomic_clock_realtime(),
        })
    }

    /// Returns the instrument provider.
    #[must_use]
    pub const fn provider(&self) -> &GrowwInstrumentProvider {
        &self.provider
    }

    fn instrument_filter(&self) -> InstrumentFilter {
        InstrumentFilter {
            exchanges: self.config.exchanges.clone(),
            segments: self.config.segments.clone(),
            instrument_types: self.config.instrument_types.clone(),
        }
    }

    /// Resolves the instrument and its venue metadata, or explains what is missing.
    fn resolve(
        &self,
        instrument_id: InstrumentId,
    ) -> anyhow::Result<(InstrumentAny, InstrumentMetadata)> {
        let instrument = self
            .provider
            .get(&instrument_id)
            .ok_or_else(|| anyhow::anyhow!("unknown instrument {instrument_id}"))?;
        let metadata = self
            .provider
            .metadata(&instrument_id)
            .ok_or_else(|| anyhow::anyhow!("missing venue metadata for {instrument_id}"))?;
        Ok((instrument, metadata))
    }

    /// Returns the market data subject serving `role` for an instrument.
    fn subject_for(metadata: &InstrumentMetadata, role: SubjectRole) -> anyhow::Result<String> {
        let kind = match role {
            SubjectRole::Quotes | SubjectRole::Depth => FeedKind::MarketDepth,
            SubjectRole::Trades => FeedKind::LivePrice,
        };
        market_data_subject(
            metadata.segment,
            metadata.exchange,
            kind,
            &metadata.exchange_token,
        )
        .map_err(|e| anyhow::anyhow!("{e}"))
    }

    /// Registers `role` for an instrument and subscribes the subject when new.
    fn add_subscription(
        &self,
        instrument_id: InstrumentId,
        role: SubjectRole,
    ) -> anyhow::Result<()> {
        let (_, metadata) = self.resolve(instrument_id)?;
        let subject = Self::subject_for(&metadata, role)?;
        let feed = self
            .feed
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("data client not connected"))?;

        let routing = Arc::clone(&self.routing);
        let feed_tx = feed.sender();
        get_runtime().spawn(async move {
            let mut routing = routing.write().await;
            let entry = routing
                .entry(subject.clone())
                .or_insert_with(|| SubjectEntry {
                    instrument_id,
                    roles: Vec::new(),
                });
            let is_new_role = !entry.roles.contains(&role);
            if is_new_role {
                entry.roles.push(role);
            }
            let needs_subscribe = entry.roles.len() == 1 && is_new_role;
            drop(routing);

            if needs_subscribe && let Err(e) = feed_tx.subscribe(subject.clone()).await {
                tracing::error!("Failed to subscribe `{subject}`: {e}");
            }
        });
        Ok(())
    }

    /// Removes `role` for an instrument and unsubscribes the subject when unused.
    fn remove_subscription(
        &self,
        instrument_id: InstrumentId,
        role: SubjectRole,
    ) -> anyhow::Result<()> {
        let (_, metadata) = self.resolve(instrument_id)?;
        let subject = Self::subject_for(&metadata, role)?;
        let feed = self
            .feed
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("data client not connected"))?;

        let routing = Arc::clone(&self.routing);
        let feed_tx = feed.sender();
        get_runtime().spawn(async move {
            let mut routing = routing.write().await;
            let mut unsubscribe = false;
            if let Some(entry) = routing.get_mut(&subject) {
                entry.roles.retain(|r| *r != role);
                if entry.roles.is_empty() {
                    routing.remove(&subject);
                    unsubscribe = true;
                }
            }
            drop(routing);

            if unsubscribe && let Err(e) = feed_tx.unsubscribe(subject.clone()).await {
                tracing::error!("Failed to unsubscribe `{subject}`: {e}");
            }
        });
        Ok(())
    }

    /// Spawns the task that turns feed events into data events.
    fn spawn_dispatcher(&mut self, mut events: tokio::sync::mpsc::UnboundedReceiver<FeedEvent>) {
        let routing = Arc::clone(&self.routing);
        let provider = self.provider.clone();
        let data_sender = self.data_sender.clone();
        let cancellation_token = self.cancellation_token.clone();
        let is_connected = Arc::clone(&self.is_connected);
        let clock = self.clock;

        let task = get_runtime().spawn(async move {
            let mut synthesizer = TradeTickSynthesizer::new();
            let mut depth_sequence: u64 = 0;

            loop {
                tokio::select! {
                    () = cancellation_token.cancelled() => break,
                    event = events.recv() => {
                        let Some(event) = event else { break };
                        match event {
                            FeedEvent::Connected => {
                                is_connected.store(true, Ordering::Release);
                            }
                            FeedEvent::Reconnected => {
                                // Volume baselines are stale after a gap; re-baseline rather
                                // than emitting one oversized synthetic trade.
                                synthesizer.reset();
                            }
                            FeedEvent::Error(message) => {
                                tracing::warn!("Groww feed error: {message}");
                            }
                            FeedEvent::Message { subject, message } => {
                                let entry = routing.read().await.get(&subject).cloned();
                                let Some(entry) = entry else { continue };
                                let Some(instrument) = provider.get(&entry.instrument_id) else {
                                    continue;
                                };
                                dispatch_message(
                                    &message,
                                    &entry,
                                    &instrument,
                                    &mut synthesizer,
                                    &mut depth_sequence,
                                    &data_sender,
                                    clock,
                                );
                            }
                        }
                    }
                }
            }
            tracing::debug!("Groww data dispatcher stopped");
        });
        self.tasks.push(task);
    }
}

/// Converts one feed message into data events according to the subject's roles.
fn dispatch_message(
    message: &FeedMessage,
    entry: &SubjectEntry,
    instrument: &InstrumentAny,
    synthesizer: &mut TradeTickSynthesizer,
    depth_sequence: &mut u64,
    data_sender: &tokio::sync::mpsc::UnboundedSender<DataEvent>,
    clock: &'static AtomicTime,
) {
    let FeedMessage::MarketData(payload) = message else {
        return; // Order updates belong to the execution client.
    };
    let ts_init = clock.get_time_ns();

    if let Some(depth) = &payload.stocks_market_depth {
        for role in &entry.roles {
            match role {
                SubjectRole::Quotes => match parse_quote_tick(depth, instrument, ts_init) {
                    Ok(Some(quote)) => {
                        let _ = data_sender.send(DataEvent::Data(Data::Quote(quote)));
                    }
                    Ok(None) => {}
                    Err(e) => tracing::warn!("Failed to parse quote: {e}"),
                },
                SubjectRole::Depth => {
                    *depth_sequence += 1;
                    match parse_depth10(depth, instrument, *depth_sequence, ts_init) {
                        Ok(depth10) => {
                            let _ =
                                data_sender.send(DataEvent::Data(Data::Depth10(Box::new(depth10))));
                        }
                        Err(e) => tracing::warn!("Failed to parse depth: {e}"),
                    }
                }
                SubjectRole::Trades => {}
            }
        }
    }

    if let Some(live_price) = &payload.stock_live_price
        && entry.roles.contains(&SubjectRole::Trades)
    {
        match synthesizer.process(live_price, instrument, ts_init) {
            Ok(Some(trade)) => {
                let _ = data_sender.send(DataEvent::Data(Data::Trade(trade)));
            }
            Ok(None) => {}
            Err(e) => tracing::warn!("Failed to synthesize trade: {e}"),
        }
    }
}

/// Maps a Nautilus bar specification onto a venue candle interval.
///
/// # Errors
///
/// Returns an error for aggregations or steps the venue does not serve.
fn candle_interval_for(bar_type: &BarType) -> anyhow::Result<(&'static str, i64)> {
    let spec = bar_type.spec();
    let step = spec.step.get();
    let interval = match (spec.aggregation, step) {
        (BarAggregation::Minute, 1) => ("1minute", 30),
        (BarAggregation::Minute, 2) => ("2minute", 30),
        (BarAggregation::Minute, 3) => ("3minute", 30),
        (BarAggregation::Minute, 5) => ("5minute", 30),
        (BarAggregation::Minute, 10) => ("10minute", 90),
        (BarAggregation::Minute, 15) => ("15minute", 90),
        (BarAggregation::Minute, 30) => ("30minute", 90),
        (BarAggregation::Hour, 1) => ("1hour", 180),
        (BarAggregation::Hour, 4) => ("4hour", 180),
        (BarAggregation::Day, 1) => ("1day", 180),
        (BarAggregation::Week, 1) => ("1week", 180),
        (BarAggregation::Month, 1) => ("1month", 180),
        (aggregation, step) => anyhow::bail!(
            "Groww serves no candles for {step}-{aggregation} bars; supported steps are \
             1/2/3/5/10/15/30 minute, 1/4 hour, and 1 day/week/month"
        ),
    };
    Ok(interval)
}

/// Returns the seconds each candle spans, used to stamp bars at close time.
const fn interval_close_shift_secs(interval: &str) -> i64 {
    match interval.as_bytes() {
        b"1minute" => 60,
        b"2minute" => 120,
        b"3minute" => 180,
        b"5minute" => 300,
        b"10minute" => 600,
        b"15minute" => 900,
        b"30minute" => 1_800,
        b"1hour" => 3_600,
        b"4hour" => 14_400,
        _ => 0, // Daily and coarser bars close with the session.
    }
}

#[async_trait::async_trait(?Send)]
impl DataClient for GrowwDataClient {
    fn client_id(&self) -> ClientId {
        self.client_id
    }

    fn venue(&self) -> Option<Venue> {
        // The client spans NSE, BSE, and MCX; instruments carry their own venue.
        None
    }

    fn start(&mut self) -> anyhow::Result<()> {
        tracing::info!("Starting Groww data client {}", self.client_id);
        Ok(())
    }

    fn stop(&mut self) -> anyhow::Result<()> {
        tracing::info!("Stopping Groww data client {}", self.client_id);
        self.cancellation_token.cancel();
        if let Some(feed) = &self.feed {
            feed.request_disconnect();
        }
        self.is_connected.store(false, Ordering::Release);
        Ok(())
    }

    fn reset(&mut self) -> anyhow::Result<()> {
        self.cancellation_token = CancellationToken::new();
        self.is_connected.store(false, Ordering::Release);
        Ok(())
    }

    fn dispose(&mut self) -> anyhow::Result<()> {
        self.stop()
    }

    fn is_connected(&self) -> bool {
        self.is_connected.load(Ordering::Acquire)
    }

    fn is_disconnected(&self) -> bool {
        !self.is_connected()
    }

    async fn connect(&mut self) -> anyhow::Result<()> {
        if self.is_connected() {
            return Ok(());
        }

        let filter = self.instrument_filter();
        let instruments = self
            .provider
            .load(&filter)
            .await
            .map_err(|e| anyhow::anyhow!("failed to load instruments: {e}"))?;
        for instrument in instruments {
            let _ = self.data_sender.send(DataEvent::Instrument(instrument));
        }

        let mut feed =
            GrowwFeedClient::new(Arc::clone(&self.http), self.config.base_url_ws.as_deref());
        let events = feed
            .connect()
            .await
            .map_err(|e| anyhow::anyhow!("failed to connect feed: {e}"))?;
        self.feed = Some(feed);
        self.spawn_dispatcher(events);

        // The handshake completes asynchronously; wait briefly so strategies starting
        // immediately after connect do not race the first subscriptions.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        while !self.is_connected() {
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("timed out waiting for Groww feed handshake");
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        Ok(())
    }

    async fn disconnect(&mut self) -> anyhow::Result<()> {
        self.stop()
    }

    fn subscribe_quotes(&mut self, cmd: SubscribeQuotes) -> anyhow::Result<()> {
        self.add_subscription(cmd.instrument_id, SubjectRole::Quotes)
    }

    fn unsubscribe_quotes(&mut self, cmd: &UnsubscribeQuotes) -> anyhow::Result<()> {
        self.remove_subscription(cmd.instrument_id, SubjectRole::Quotes)
    }

    fn subscribe_trades(&mut self, cmd: SubscribeTrades) -> anyhow::Result<()> {
        self.add_subscription(cmd.instrument_id, SubjectRole::Trades)
    }

    fn unsubscribe_trades(&mut self, cmd: &UnsubscribeTrades) -> anyhow::Result<()> {
        self.remove_subscription(cmd.instrument_id, SubjectRole::Trades)
    }

    fn subscribe_book_depth10(&mut self, cmd: SubscribeBookDepth10) -> anyhow::Result<()> {
        self.add_subscription(cmd.instrument_id, SubjectRole::Depth)
    }

    fn unsubscribe_book_depth10(&mut self, cmd: &UnsubscribeBookDepth10) -> anyhow::Result<()> {
        self.remove_subscription(cmd.instrument_id, SubjectRole::Depth)
    }

    fn request_instruments(&self, request: RequestInstruments) -> anyhow::Result<()> {
        let provider = self.provider.clone();
        let filter = self.instrument_filter();
        let sender = self.data_sender.clone();
        let clock = self.clock;
        let client_id = self.client_id;

        get_runtime().spawn(async move {
            let instruments = match provider.load(&filter).await {
                Ok(instruments) => instruments,
                Err(e) => {
                    tracing::error!("Instrument request failed: {e}");
                    return;
                }
            };
            let venue = request.venue.unwrap_or(*crate::common::consts::NSE_VENUE);
            let response = DataResponse::Instruments(InstrumentsResponse::new(
                request.request_id,
                client_id,
                venue,
                instruments,
                datetime_to_unix_nanos(request.start),
                datetime_to_unix_nanos(request.end),
                clock.get_time_ns(),
                request.params,
            ));
            let _ = sender.send(DataEvent::Response(response));
        });
        Ok(())
    }

    fn request_instrument(&self, request: RequestInstrument) -> anyhow::Result<()> {
        let instrument = self
            .provider
            .get(&request.instrument_id)
            .ok_or_else(|| anyhow::anyhow!("unknown instrument {}", request.instrument_id))?;
        let response = DataResponse::Instrument(Box::new(InstrumentResponse::new(
            request.request_id,
            self.client_id,
            instrument.id(),
            instrument,
            datetime_to_unix_nanos(request.start),
            datetime_to_unix_nanos(request.end),
            self.clock.get_time_ns(),
            request.params,
        )));
        let _ = self.data_sender.send(DataEvent::Response(response));
        Ok(())
    }

    fn request_bars(&self, request: RequestBars) -> anyhow::Result<()> {
        let bar_type = request.bar_type;
        let instrument_id = bar_type.instrument_id();
        let (instrument, metadata) = self.resolve(instrument_id)?;
        let (interval, max_days) = candle_interval_for(&bar_type)?;

        let http = Arc::clone(&self.http);
        let sender = self.data_sender.clone();
        let clock = self.clock;
        let client_id = self.client_id;

        get_runtime().spawn(async move {
            match fetch_bars(
                &http,
                &metadata,
                &instrument,
                bar_type,
                interval,
                max_days,
                datetime_to_unix_nanos(request.start).map(|n| n.as_u64()),
                datetime_to_unix_nanos(request.end).map(|n| n.as_u64()),
                request.limit.map(std::num::NonZeroUsize::get),
                clock,
            )
            .await
            {
                Ok(bars) => {
                    let response = DataResponse::Bars(BarsResponse::new(
                        request.request_id,
                        client_id,
                        bar_type,
                        bars,
                        datetime_to_unix_nanos(request.start),
                        datetime_to_unix_nanos(request.end),
                        clock.get_time_ns(),
                        request.params,
                    ));
                    let _ = sender.send(DataEvent::Response(response));
                }
                Err(e) => tracing::error!("Bar request failed: {e}"),
            }
        });
        Ok(())
    }
}

/// Fetches candles in venue-sized chunks and converts them to close-stamped bars.
#[allow(clippy::too_many_arguments)]
async fn fetch_bars(
    http: &GrowwHttpClient,
    metadata: &InstrumentMetadata,
    instrument: &InstrumentAny,
    bar_type: BarType,
    interval: &str,
    max_days: i64,
    start_ns: Option<u64>,
    end_ns: Option<u64>,
    limit: Option<usize>,
    clock: &'static AtomicTime,
) -> anyhow::Result<Vec<Bar>> {
    use jiff::ToSpan;

    let tz = india_tz().map_err(|e| anyhow::anyhow!("{e}"))?;
    let now = jiff::Zoned::now().with_time_zone(tz.clone());

    let to_zoned = |ns: u64| -> anyhow::Result<jiff::Zoned> {
        let ts = jiff::Timestamp::from_nanosecond(i128::from(ns))
            .map_err(|e| anyhow::anyhow!("invalid request time: {e}"))?;
        Ok(ts.to_zoned(tz.clone()))
    };
    let end = end_ns.map_or_else(|| Ok(now.clone()), to_zoned)?;
    let start = start_ns.map_or_else(
        || end.checked_sub(max_days.days()).map_err(Into::into),
        to_zoned,
    )?;

    let shift = interval_close_shift_secs(interval);
    let mut bars = Vec::new();
    let mut chunk_start = start;

    while chunk_start < end {
        let chunk_end = chunk_start.checked_add(max_days.days())?.min(end.clone());

        let candles = http
            .http_get_candles(
                metadata.exchange,
                metadata.segment,
                &metadata.groww_symbol,
                &chunk_start.strftime("%Y-%m-%d %H:%M:%S").to_string(),
                &chunk_end.strftime("%Y-%m-%d %H:%M:%S").to_string(),
                interval,
            )
            .await
            .map_err(|e| anyhow::anyhow!("candle request failed: {e}"))?;

        let ts_init = clock.get_time_ns();
        for candle in candles {
            let Some((open, high, low, close, volume)) = candle.ohlcv() else {
                continue; // Pre-open auction rows carry no OHLC.
            };
            let civil: jiff::civil::DateTime = candle
                .timestamp()
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid candle timestamp: {e}"))?;
            let close_civil = if shift > 0 {
                civil.checked_add(shift.seconds())?
            } else {
                civil.with().hour(15).minute(30).second(0).build()?
            };
            let zoned = close_civil
                .to_zoned(tz.clone())
                .map_err(|e| anyhow::anyhow!("unresolvable candle timestamp: {e}"))?;
            let ts_event = nautilus_core::UnixNanos::from(
                u64::try_from(zoned.timestamp().as_nanosecond())
                    .map_err(|_| anyhow::anyhow!("candle precedes the UNIX epoch"))?,
            );

            let precision = instrument.price_precision();
            bars.push(Bar::new(
                bar_type,
                nautilus_model::types::Price::new(open, precision),
                nautilus_model::types::Price::new(high, precision),
                nautilus_model::types::Price::new(low, precision),
                nautilus_model::types::Price::new(close, precision),
                nautilus_model::types::Quantity::new(volume, 0),
                ts_event,
                ts_init,
            ));
        }
        chunk_start = chunk_end;
    }

    bars.sort_by_key(|bar| bar.ts_event);
    bars.dedup_by_key(|bar| bar.ts_event);
    if let Some(limit) = limit
        && bars.len() > limit
    {
        bars.drain(..bars.len() - limit);
    }
    Ok(bars)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("RELIANCE.NSE-5-MINUTE-LAST-EXTERNAL", "5minute", 30)]
    #[case("RELIANCE.NSE-1-DAY-LAST-EXTERNAL", "1day", 180)]
    #[case("RELIANCE.NSE-15-MINUTE-LAST-EXTERNAL", "15minute", 90)]
    fn test_candle_interval_mapping(
        #[case] bar_type: &str,
        #[case] interval: &str,
        #[case] max_days: i64,
    ) {
        let bar_type: BarType = bar_type.parse().unwrap();
        let (mapped, days) = candle_interval_for(&bar_type).unwrap();
        assert_eq!(mapped, interval);
        assert_eq!(days, max_days);
    }

    #[rstest]
    fn test_unsupported_interval_is_refused() {
        let bar_type: BarType = "RELIANCE.NSE-2-HOUR-LAST-EXTERNAL".parse().unwrap();
        assert!(candle_interval_for(&bar_type).is_err());
    }
}
