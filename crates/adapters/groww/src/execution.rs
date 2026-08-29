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

//! Live execution client for Groww.
//!
//! Orders are commanded over REST and their lifecycle observed on the streaming order-update
//! feed, with REST reconciliation endpoints backing mass status requests.
//!
//! # Client order identity
//!
//! Groww's `order_reference_id` accepts 8-20 alphanumeric characters with at most two hyphens,
//! which cannot carry a Nautilus client order ID verbatim. Each submission therefore sends a
//! deterministic 18-character digest of the client order ID and keeps the pair in an in-session
//! map. Reports for orders submitted by other sessions carry no client order ID and reconcile by
//! venue order ID instead.
//!
//! # Fills
//!
//! The order-update feed reports integer prices whose scale is undocumented, so fills are never
//! priced from the feed. A fill-bearing update instead triggers a REST trade lookup, which
//! reports prices in rupees and carries the venue trade ID used for deduplication.

use std::sync::{Arc, Mutex};

use ahash::{AHashMap, AHashSet};
use async_trait::async_trait;
use nautilus_common::live::{runner::get_exec_event_sender, runtime::get_runtime};
use nautilus_core::{
    MUTEX_POISONED, Params, UnixNanos,
    time::{AtomicTime, get_atomic_clock_realtime},
};
use nautilus_live::{ExecutionClientCore, ExecutionEventEmitter};
use nautilus_model::{
    accounts::AccountAny,
    enums::{OmsType, OrderSide, OrderType, TimeInForce},
    identifiers::{AccountId, ClientId, ClientOrderId, InstrumentId, VenueOrderId},
    instruments::{Instrument, InstrumentAny},
    orders::{Order as OrderTrait, OrderAny},
    reports::{ExecutionMassStatus, FillReport, OrderStatusReport, PositionStatusReport},
    types::{AccountBalance, Currency, MarginBalance, Money},
};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::{
    common::{
        credential::GrowwCredential,
        enums::{
            GrowwOrderStatus, GrowwOrderType, GrowwSegment, GrowwTransactionType, GrowwValidity,
        },
    },
    config::GrowwExecClientConfig,
    http::{
        client::GrowwHttpClient,
        models::UserMargin,
        parse::{parse_fill_report, parse_order_status_report, parse_position_status_report},
        query::{CancelOrderRequest, CreateOrderRequest, ModifyOrderRequest},
    },
    provider::{GrowwInstrumentProvider, InstrumentFilter, InstrumentMetadata},
    websocket::{
        client::{FeedEvent, GrowwFeedClient},
        messages::{FeedMessage, derivatives_order_updates_subject, equity_order_updates_subject},
    },
};

/// Returns the venue `order_reference_id` for a client order ID.
///
/// A deterministic digest keeps resubmissions idempotent from the venue's point of view while
/// fitting the 8-20 alphanumeric constraint: `NT` followed by 16 hex characters of the ID's
/// SHA-256.
#[must_use]
pub fn order_reference_for(client_order_id: &ClientOrderId) -> String {
    let digest = Sha256::digest(client_order_id.as_str().as_bytes());
    let mut reference = String::with_capacity(18);
    reference.push_str("NT");
    for byte in &digest[..8] {
        use std::fmt::Write as _;
        let _ = write!(reference, "{byte:02x}");
    }
    reference
}

/// In-session mapping between Nautilus and venue order identity.
#[derive(Debug, Default)]
struct OrderIdMap {
    by_reference: AHashMap<String, ClientOrderId>,
    by_venue: AHashMap<VenueOrderId, ClientOrderId>,
    instrument_by_venue: AHashMap<VenueOrderId, InstrumentId>,
}

/// Live execution client for Groww.
#[derive(Debug)]
pub struct GrowwExecutionClient {
    core: ExecutionClientCore,
    config: GrowwExecClientConfig,
    clock: &'static AtomicTime,
    emitter: ExecutionEventEmitter,
    http: Arc<GrowwHttpClient>,
    provider: GrowwInstrumentProvider,
    feed: Option<GrowwFeedClient>,
    ids: Arc<Mutex<OrderIdMap>>,
    seen_trades: Arc<Mutex<AHashSet<String>>>,
    cancellation_token: CancellationToken,
}

impl GrowwExecutionClient {
    /// Creates a new [`GrowwExecutionClient`].
    ///
    /// # Errors
    ///
    /// Returns an error if credentials are missing or the HTTP client cannot be created.
    pub fn new(core: ExecutionClientCore, config: GrowwExecClientConfig) -> anyhow::Result<Self> {
        let credential =
            GrowwCredential::resolve(config.api_key.clone(), config.api_secret.clone())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Groww credentials not available; set GROWW_API_KEY and GROWW_API_SECRET \
                         or pass them in the config"
                    )
                })?;
        let http = Arc::new(GrowwHttpClient::with_credentials(
            credential,
            config.base_url_http.as_deref(),
            config.http_timeout_secs,
            None,
            None,
        )?);
        let provider = GrowwInstrumentProvider::new(Arc::clone(&http));

        let clock = get_atomic_clock_realtime();
        let emitter = ExecutionEventEmitter::new(
            clock,
            core.trader_id,
            core.account_id,
            core.account_type,
            Some(Currency::INR()),
        );

        Ok(Self {
            core,
            config,
            clock,
            emitter,
            http,
            provider,
            feed: None,
            ids: Arc::new(Mutex::new(OrderIdMap::default())),
            seen_trades: Arc::new(Mutex::new(AHashSet::new())),
            cancellation_token: CancellationToken::new(),
        })
    }

    fn segments(&self) -> Vec<GrowwSegment> {
        if self.config.segments.is_empty() {
            vec![GrowwSegment::Cash]
        } else {
            self.config.segments.clone()
        }
    }

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

    /// Resolves the instrument for a venue symbol and exchange from the provider cache.
    fn instrument_by_symbol(&self, trading_symbol: &str, exchange: &str) -> Option<InstrumentAny> {
        let instrument_id: InstrumentId = format!("{trading_symbol}.{exchange}").parse().ok()?;
        self.provider.get(&instrument_id)
    }

    fn client_order_id_for_reference(&self, reference: Option<&str>) -> Option<ClientOrderId> {
        reference.and_then(|reference| {
            self.ids
                .lock()
                .expect(MUTEX_POISONED)
                .by_reference
                .get(reference)
                .copied()
        })
    }

    /// Converts a Nautilus order into the venue's create request.
    fn build_create_request(
        &self,
        order: &OrderAny,
        metadata: &InstrumentMetadata,
    ) -> anyhow::Result<CreateOrderRequest> {
        let order_type = match order.order_type() {
            OrderType::Market => GrowwOrderType::Market,
            OrderType::Limit => GrowwOrderType::Limit,
            OrderType::StopMarket => GrowwOrderType::StopLossMarket,
            OrderType::StopLimit => GrowwOrderType::StopLoss,
            other => anyhow::bail!("Groww does not accept {other} orders"),
        };
        let transaction_type = match order.order_side() {
            OrderSide::Buy => GrowwTransactionType::Buy,
            OrderSide::Sell => GrowwTransactionType::Sell,
            OrderSide::NoOrderSide => anyhow::bail!("order has no side"),
        };
        match order.time_in_force() {
            TimeInForce::Day | TimeInForce::Gtc => {}
            other => anyhow::bail!("Groww accepts only DAY validity, not {other}"),
        }

        let quantity = order.quantity().as_f64();
        anyhow::ensure!(
            quantity.fract() == 0.0 && quantity > 0.0,
            "Groww trades whole units; quantity was {quantity}"
        );

        Ok(CreateOrderRequest {
            trading_symbol: metadata.trading_symbol.clone(),
            quantity: quantity as u64,
            price: order.price().map_or(0.0, |p| p.as_f64()),
            trigger_price: order.trigger_price().map(|p| p.as_f64()),
            validity: GrowwValidity::Day,
            exchange: metadata.exchange,
            segment: metadata.segment,
            product: self.config.product,
            order_type,
            transaction_type,
            order_reference_id: order_reference_for(&order.client_order_id()),
        })
    }

    /// Applies one order update from the feed, fetching trades over REST when fills advance.
    #[allow(clippy::too_many_lines)]
    fn spawn_update_dispatcher(&self, mut events: tokio::sync::mpsc::UnboundedReceiver<FeedEvent>) {
        let emitter = self.emitter.clone();
        let http = Arc::clone(&self.http);
        let ids = Arc::clone(&self.ids);
        let seen_trades = Arc::clone(&self.seen_trades);
        let provider = self.provider.clone();
        let account_id = self.core.account_id;
        let clock = self.clock;
        let cancellation_token = self.cancellation_token.clone();

        get_runtime().spawn(async move {
            loop {
                tokio::select! {
                    () = cancellation_token.cancelled() => break,
                    event = events.recv() => {
                        let Some(event) = event else { break };
                        let FeedEvent::Message { message, .. } = event else { continue };
                        let FeedMessage::OrderUpdate(update) = message else { continue };
                        let Some(detail) = update.order_detail_update_dto else { continue };

                        let venue_order_id = VenueOrderId::new(detail.groww_order_id.as_str());
                        let status = map_feed_status(detail.order_status);
                        tracing::debug!(
                            "Order update: {venue_order_id} status={status:?} filled={}",
                            detail.filled_qty
                        );

                        let _ = status;
                        let (client_order_id, instrument_id) = {
                            let map = ids.lock().expect(MUTEX_POISONED);
                            (
                                map.by_venue.get(&venue_order_id).copied(),
                                map.instrument_by_venue.get(&venue_order_id).copied(),
                            )
                        };
                        let Some(instrument_id) = instrument_id else {
                            // An order this session did not submit; reconciliation covers it.
                            continue;
                        };
                        let Some(instrument) = provider.get(&instrument_id) else { continue };
                        let ts = clock.get_time_ns();

                        // Fetch the authoritative order state over REST and hand the report to
                        // the execution manager, which reconciles it into lifecycle events.
                        // The feed's own price fields are unscaled integers and never used.
                        let segment = metadata_segment(&provider, instrument_id);
                        match http
                            .http_get_order_detail(&detail.groww_order_id, segment)
                            .await
                        {
                            Ok(order) => {
                                match parse_order_status_report(
                                    &order,
                                    account_id,
                                    &instrument,
                                    client_order_id,
                                    ts,
                                ) {
                                    Ok(report) => emitter.send_order_status_report(report),
                                    Err(e) => tracing::warn!("Failed to parse order report: {e}"),
                                }
                            }
                            Err(e) => tracing::warn!(
                                "Order status lookup failed for {venue_order_id}: {e}"
                            ),
                        }

                        // A fill advanced: fetch the trades and emit any not yet seen.
                        if detail.filled_qty > 0 {
                            match http
                                .http_get_order_trades(&detail.groww_order_id, segment, 0, 100)
                                .await
                            {
                                Ok(trades) => {
                                    for trade in trades {
                                        let is_new = seen_trades
                                            .lock()
                                            .expect(MUTEX_POISONED)
                                            .insert(trade.groww_trade_id.clone());
                                        if !is_new {
                                            continue;
                                        }
                                        match parse_fill_report(
                                            &trade,
                                            account_id,
                                            &instrument,
                                            client_order_id,
                                            ts,
                                        ) {
                                            Ok(report) => emitter.send_fill_report(report),
                                            Err(e) => {
                                                tracing::warn!("Failed to parse trade: {e}");
                                            }
                                        }
                                    }
                                }
                                Err(e) => tracing::warn!(
                                    "Trade lookup failed for {venue_order_id}: {e}"
                                ),
                            }
                        }
                    }
                }
            }
            tracing::debug!("Groww execution dispatcher stopped");
        });
    }

    /// Fetches the venue margin detail and emits an account state.
    async fn update_account_state(&self) -> anyhow::Result<()> {
        let margin: UserMargin = self
            .http
            .http_get_margin()
            .await
            .map_err(|e| anyhow::anyhow!("margin request failed: {e}"))?;

        let inr = Currency::INR();
        let free = margin.clear_cash;
        let locked = margin.net_margin_used;
        let total = free + locked;
        let balance = AccountBalance::new(
            Money::new(total, inr),
            Money::new(locked, inr),
            Money::new(free, inr),
        );
        self.emitter.emit_account_state(
            vec![balance],
            Vec::<MarginBalance>::new(),
            true,
            self.clock.get_time_ns(),
            None,
        );
        Ok(())
    }
}

/// Maps a feed status code onto the venue status enum.
const fn map_feed_status(code: i32) -> Option<GrowwOrderStatus> {
    // Codes follow `StocksOrderStatus.Enum` in the recovered schema.
    match code {
        0 => Some(GrowwOrderStatus::New),
        1 => Some(GrowwOrderStatus::Acked),
        2 => Some(GrowwOrderStatus::TriggerPending),
        3 => Some(GrowwOrderStatus::Approved),
        4 => Some(GrowwOrderStatus::Rejected),
        5 => Some(GrowwOrderStatus::Failed),
        6 => Some(GrowwOrderStatus::Executed),
        7 => Some(GrowwOrderStatus::DeliveryAwaited),
        8 => Some(GrowwOrderStatus::Cancelled),
        9 => Some(GrowwOrderStatus::CancellationRequested),
        10 => Some(GrowwOrderStatus::ModificationRequested),
        11 => Some(GrowwOrderStatus::Completed),
        _ => None,
    }
}

/// Returns the segment recorded for an instrument, defaulting to cash.
fn metadata_segment(
    provider: &GrowwInstrumentProvider,
    instrument_id: InstrumentId,
) -> GrowwSegment {
    provider
        .metadata(&instrument_id)
        .map_or(GrowwSegment::Cash, |m| m.segment)
}

#[async_trait(?Send)]
impl nautilus_common::clients::ExecutionClient for GrowwExecutionClient {
    fn is_connected(&self) -> bool {
        self.core.is_connected()
    }

    fn client_id(&self) -> ClientId {
        self.core.client_id
    }

    fn account_id(&self) -> AccountId {
        self.core.account_id
    }

    fn venue(&self) -> nautilus_model::identifiers::Venue {
        self.core.venue
    }

    fn oms_type(&self) -> OmsType {
        self.core.oms_type
    }

    fn get_account(&self) -> Option<AccountAny> {
        self.core.cache().account_owned(&self.core.account_id)
    }

    fn generate_account_state(
        &self,
        balances: Vec<AccountBalance>,
        margins: Vec<MarginBalance>,
        reported: bool,
        ts_event: UnixNanos,
        info: Option<Params>,
    ) -> anyhow::Result<()> {
        self.emitter
            .emit_account_state(balances, margins, reported, ts_event, info);
        Ok(())
    }

    fn start(&mut self) -> anyhow::Result<()> {
        self.emitter.set_sender(get_exec_event_sender());
        tracing::info!("Starting Groww execution client {}", self.core.client_id);
        Ok(())
    }

    fn stop(&mut self) -> anyhow::Result<()> {
        tracing::info!("Stopping Groww execution client {}", self.core.client_id);
        self.cancellation_token.cancel();
        if let Some(feed) = &self.feed {
            feed.request_disconnect();
        }
        self.core.set_disconnected();
        Ok(())
    }

    async fn connect(&mut self) -> anyhow::Result<()> {
        if self.core.is_connected() {
            return Ok(());
        }

        let filter = InstrumentFilter {
            exchanges: self.config.exchanges.clone(),
            segments: self.config.segments.clone(),
            instrument_types: vec![],
        };
        self.provider
            .load(&filter)
            .await
            .map_err(|e| anyhow::anyhow!("failed to load instruments: {e}"))?;

        self.update_account_state().await?;

        let mut feed =
            GrowwFeedClient::new(Arc::clone(&self.http), self.config.base_url_ws.as_deref());
        let events = feed
            .connect()
            .await
            .map_err(|e| anyhow::anyhow!("failed to connect feed: {e}"))?;
        let subscription_id = feed
            .subscription_id()
            .await
            .ok_or_else(|| anyhow::anyhow!("missing feed subscription id"))?;

        feed.subscribe(equity_order_updates_subject(&subscription_id))
            .await
            .map_err(|e| anyhow::anyhow!("failed to subscribe order updates: {e}"))?;
        if self.segments().contains(&GrowwSegment::Fno) {
            feed.subscribe(derivatives_order_updates_subject(&subscription_id))
                .await
                .map_err(|e| anyhow::anyhow!("failed to subscribe FNO order updates: {e}"))?;
        }

        self.feed = Some(feed);
        self.spawn_update_dispatcher(events);
        self.core.set_connected();
        Ok(())
    }

    async fn disconnect(&mut self) -> anyhow::Result<()> {
        self.stop()
    }

    fn submit_order(
        &self,
        cmd: nautilus_common::messages::execution::SubmitOrder,
    ) -> anyhow::Result<()> {
        let order = self.core.cache().try_order_owned(&cmd.client_order_id)?;
        if order.is_closed() {
            tracing::warn!("Cannot submit closed order {}", order.client_order_id());
            return Ok(());
        }

        let instrument_id = order.instrument_id();
        let (_, metadata) = match self.resolve(instrument_id) {
            Ok(resolved) => resolved,
            Err(e) => {
                self.emitter.emit_order_denied(&order, &e.to_string());
                return Ok(());
            }
        };
        let request = match self.build_create_request(&order, &metadata) {
            Ok(request) => request,
            Err(e) => {
                self.emitter.emit_order_denied(&order, &e.to_string());
                return Ok(());
            }
        };

        let client_order_id = order.client_order_id();
        let strategy_id = order.strategy_id();
        {
            let mut map = self.ids.lock().expect(MUTEX_POISONED);
            map.by_reference
                .insert(request.order_reference_id.clone(), client_order_id);
        }
        self.emitter.emit_order_submitted(&order);

        let http = Arc::clone(&self.http);
        let emitter = self.emitter.clone();
        let ids = Arc::clone(&self.ids);
        let provider = self.provider.clone();
        let account_id = self.core.account_id;
        let clock = self.clock;
        let segment = metadata_segment(&self.provider, instrument_id);

        get_runtime().spawn(async move {
            match http.http_place_order(&request).await {
                Ok(ack) => {
                    let venue_order_id = VenueOrderId::new(ack.groww_order_id.as_str());
                    {
                        let mut map = ids.lock().expect(MUTEX_POISONED);
                        map.by_venue.insert(venue_order_id, client_order_id);
                        map.instrument_by_venue
                            .insert(venue_order_id, instrument_id);
                    }
                    if matches!(
                        ack.order_status,
                        Some(GrowwOrderStatus::Rejected | GrowwOrderStatus::Failed)
                    ) {
                        let reason = ack.remark.as_deref().unwrap_or("venue rejected");
                        emitter.emit_order_rejected_event(
                            strategy_id,
                            instrument_id,
                            client_order_id,
                            reason,
                            clock.get_time_ns(),
                            false,
                        );
                        return;
                    }
                    // Confirm through the authoritative REST state so accepted orders carry
                    // their venue timestamps and any immediate fill is not missed.
                    if let Some(instrument) = provider.get(&instrument_id) {
                        match http
                            .http_get_order_detail(&ack.groww_order_id, segment)
                            .await
                        {
                            Ok(order) => match parse_order_status_report(
                                &order,
                                account_id,
                                &instrument,
                                Some(client_order_id),
                                clock.get_time_ns(),
                            ) {
                                Ok(report) => emitter.send_order_status_report(report),
                                Err(e) => tracing::warn!("Failed to parse submit ack: {e}"),
                            },
                            Err(e) => {
                                tracing::warn!("Submit confirmation lookup failed: {e}");
                            }
                        }
                    }
                }
                Err(e) => {
                    emitter.emit_order_rejected_event(
                        strategy_id,
                        instrument_id,
                        client_order_id,
                        &e.to_string(),
                        clock.get_time_ns(),
                        false,
                    );
                }
            }
        });
        Ok(())
    }

    fn modify_order(
        &self,
        cmd: nautilus_common::messages::execution::ModifyOrder,
    ) -> anyhow::Result<()> {
        let order = self.core.cache().try_order_owned(&cmd.client_order_id)?;
        let Some(venue_order_id) = order.venue_order_id() else {
            anyhow::bail!("order {} has no venue order id yet", cmd.client_order_id);
        };
        let (instrument, metadata) = self.resolve(order.instrument_id())?;
        let _ = instrument;

        let order_type = match order.order_type() {
            OrderType::Market => GrowwOrderType::Market,
            OrderType::Limit => GrowwOrderType::Limit,
            OrderType::StopMarket => GrowwOrderType::StopLossMarket,
            OrderType::StopLimit => GrowwOrderType::StopLoss,
            other => anyhow::bail!("Groww does not accept {other} orders"),
        };
        let quantity = cmd.quantity.unwrap_or_else(|| order.quantity());
        let request = ModifyOrderRequest {
            groww_order_id: venue_order_id.to_string(),
            segment: metadata.segment,
            order_type,
            quantity: quantity.as_f64() as u64,
            price: cmd.price.map(|p| p.as_f64()),
            trigger_price: cmd.trigger_price.map(|p| p.as_f64()),
        };

        let http = Arc::clone(&self.http);
        let emitter = self.emitter.clone();
        let clock = self.clock;
        let client_order_id = cmd.client_order_id;
        let strategy_id = order.strategy_id();
        let instrument_id = order.instrument_id();
        let price = cmd.price;
        let trigger_price = cmd.trigger_price;
        let order_for_update = order;

        get_runtime().spawn(async move {
            match http.http_modify_order(&request).await {
                Ok(_) => emitter.emit_order_updated(
                    &order_for_update,
                    venue_order_id,
                    quantity,
                    price,
                    trigger_price,
                    None,
                    clock.get_time_ns(),
                ),
                Err(e) => emitter.emit_order_modify_rejected_event(
                    strategy_id,
                    instrument_id,
                    client_order_id,
                    Some(venue_order_id),
                    &e.to_string(),
                    clock.get_time_ns(),
                ),
            }
        });
        Ok(())
    }

    fn cancel_order(
        &self,
        cmd: nautilus_common::messages::execution::CancelOrder,
    ) -> anyhow::Result<()> {
        let order = self.core.cache().try_order_owned(&cmd.client_order_id)?;
        let Some(venue_order_id) = order.venue_order_id() else {
            anyhow::bail!("order {} has no venue order id yet", cmd.client_order_id);
        };
        let (_, metadata) = self.resolve(order.instrument_id())?;

        let request = CancelOrderRequest {
            groww_order_id: venue_order_id.to_string(),
            segment: metadata.segment,
        };
        let http = Arc::clone(&self.http);
        let emitter = self.emitter.clone();
        let clock = self.clock;
        let client_order_id = cmd.client_order_id;
        let strategy_id = order.strategy_id();
        let instrument_id = order.instrument_id();
        let order_for_cancel = order;

        get_runtime().spawn(async move {
            match http.http_cancel_order(&request).await {
                Ok(_) => emitter.emit_order_canceled(
                    &order_for_cancel,
                    Some(venue_order_id),
                    clock.get_time_ns(),
                ),
                Err(e) => emitter.emit_order_cancel_rejected_event(
                    strategy_id,
                    instrument_id,
                    client_order_id,
                    Some(venue_order_id),
                    &e.to_string(),
                    clock.get_time_ns(),
                ),
            }
        });
        Ok(())
    }

    fn query_account(
        &self,
        _cmd: nautilus_common::messages::execution::QueryAccount,
    ) -> anyhow::Result<()> {
        let http = Arc::clone(&self.http);
        let emitter = self.emitter.clone();
        let clock = self.clock;
        get_runtime().spawn(async move {
            match http.http_get_margin().await {
                Ok(margin) => {
                    let inr = Currency::INR();
                    let free = margin.clear_cash;
                    let locked = margin.net_margin_used;
                    let balance = AccountBalance::new(
                        Money::new(free + locked, inr),
                        Money::new(locked, inr),
                        Money::new(free, inr),
                    );
                    emitter.emit_account_state(
                        vec![balance],
                        Vec::new(),
                        true,
                        clock.get_time_ns(),
                        None,
                    );
                }
                Err(e) => tracing::error!("Account query failed: {e}"),
            }
        });
        Ok(())
    }

    async fn generate_order_status_report(
        &self,
        cmd: &nautilus_common::messages::execution::GenerateOrderStatusReport,
    ) -> anyhow::Result<Option<OrderStatusReport>> {
        let Some(venue_order_id) = cmd.venue_order_id else {
            return Ok(None);
        };
        let ts_init = self.clock.get_time_ns();

        for segment in self.segments() {
            match self
                .http
                .http_get_order_detail(venue_order_id.as_str(), segment)
                .await
            {
                Ok(order) => {
                    let instrument = order
                        .trading_symbol
                        .as_deref()
                        .zip(order.exchange)
                        .and_then(|(symbol, exchange)| {
                            self.instrument_by_symbol(symbol, exchange.as_ref())
                        });
                    let Some(instrument) = instrument else {
                        continue;
                    };
                    let client_order_id =
                        self.client_order_id_for_reference(order.order_reference_id.as_deref());
                    return Ok(Some(
                        parse_order_status_report(
                            &order,
                            self.core.account_id,
                            &instrument,
                            client_order_id,
                            ts_init,
                        )
                        .map_err(|e| anyhow::anyhow!("{e}"))?,
                    ));
                }
                Err(e) => {
                    tracing::debug!("Order {venue_order_id} not in segment {segment}: {e}");
                }
            }
        }
        Ok(None)
    }

    async fn generate_order_status_reports(
        &self,
        cmd: &nautilus_common::messages::execution::GenerateOrderStatusReports,
    ) -> anyhow::Result<Vec<OrderStatusReport>> {
        let ts_init = self.clock.get_time_ns();
        let mut reports = Vec::new();

        for segment in self.segments() {
            let mut page = 0u32;
            loop {
                let orders = self
                    .http
                    .http_get_order_list(segment, page, 100)
                    .await
                    .map_err(|e| anyhow::anyhow!("order list failed: {e}"))?;
                let done = orders.len() < 100;

                for order in &orders {
                    let instrument = order
                        .trading_symbol
                        .as_deref()
                        .zip(order.exchange)
                        .and_then(|(symbol, exchange)| {
                            self.instrument_by_symbol(symbol, exchange.as_ref())
                        });
                    let Some(instrument) = instrument else {
                        continue;
                    };
                    if cmd.open_only && order.order_status.is_terminal() {
                        continue;
                    }
                    let client_order_id =
                        self.client_order_id_for_reference(order.order_reference_id.as_deref());
                    match parse_order_status_report(
                        order,
                        self.core.account_id,
                        &instrument,
                        client_order_id,
                        ts_init,
                    ) {
                        Ok(report) => reports.push(report),
                        Err(e) => tracing::warn!("Skipping order report: {e}"),
                    }
                }

                if done {
                    break;
                }
                page += 1;
            }
        }
        Ok(reports)
    }

    async fn generate_fill_reports(
        &self,
        cmd: nautilus_common::messages::execution::GenerateFillReports,
    ) -> anyhow::Result<Vec<FillReport>> {
        let ts_init = self.clock.get_time_ns();
        let mut reports = Vec::new();

        // The venue exposes trades per order rather than per account, so walk the order list
        // and fetch trades for orders showing any fill.
        for segment in self.segments() {
            let mut page = 0u32;
            loop {
                let orders = self
                    .http
                    .http_get_order_list(segment, page, 100)
                    .await
                    .map_err(|e| anyhow::anyhow!("order list failed: {e}"))?;
                let done = orders.len() < 100;

                for order in &orders {
                    if order.filled_quantity <= 0.0 {
                        continue;
                    }
                    if let Some(filter_venue_id) = cmd.venue_order_id
                        && filter_venue_id.as_str() != order.groww_order_id
                    {
                        continue;
                    }
                    let instrument = order
                        .trading_symbol
                        .as_deref()
                        .zip(order.exchange)
                        .and_then(|(symbol, exchange)| {
                            self.instrument_by_symbol(symbol, exchange.as_ref())
                        });
                    let Some(instrument) = instrument else {
                        continue;
                    };
                    let client_order_id =
                        self.client_order_id_for_reference(order.order_reference_id.as_deref());

                    let trades = self
                        .http
                        .http_get_order_trades(&order.groww_order_id, segment, 0, 100)
                        .await
                        .map_err(|e| anyhow::anyhow!("trade list failed: {e}"))?;
                    for trade in &trades {
                        match parse_fill_report(
                            trade,
                            self.core.account_id,
                            &instrument,
                            client_order_id,
                            ts_init,
                        ) {
                            Ok(report) => reports.push(report),
                            Err(e) => tracing::warn!("Skipping fill report: {e}"),
                        }
                    }
                }

                if done {
                    break;
                }
                page += 1;
            }
        }
        Ok(reports)
    }

    async fn generate_position_status_reports(
        &self,
        _cmd: &nautilus_common::messages::execution::GeneratePositionStatusReports,
    ) -> anyhow::Result<Vec<PositionStatusReport>> {
        let ts_init = self.clock.get_time_ns();
        let positions = self
            .http
            .http_get_positions()
            .await
            .map_err(|e| anyhow::anyhow!("positions request failed: {e}"))?;

        let mut reports = Vec::new();
        for position in &positions {
            // Positions omit the exchange for cross-listed holdings; NSE is the venue's
            // primary routing default.
            let exchange = position.exchange.map_or("NSE", |e| match e {
                crate::common::enums::GrowwExchange::Nse => "NSE",
                crate::common::enums::GrowwExchange::Bse => "BSE",
                crate::common::enums::GrowwExchange::Mcx => "MCX",
            });
            let Some(instrument) = self.instrument_by_symbol(&position.trading_symbol, exchange)
            else {
                continue;
            };
            match parse_position_status_report(
                position,
                self.core.account_id,
                instrument.id(),
                ts_init,
            ) {
                Ok(report) => reports.push(report),
                Err(e) => tracing::warn!("Skipping position report: {e}"),
            }
        }
        Ok(reports)
    }

    async fn generate_mass_status(
        &self,
        _lookback_mins: Option<u64>,
    ) -> anyhow::Result<Option<ExecutionMassStatus>> {
        let ts_init = self.clock.get_time_ns();

        let order_cmd =
            nautilus_common::messages::execution::GenerateOrderStatusReportsBuilder::default()
                .ts_init(ts_init)
                .open_only(false)
                .build()
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        let orders = self.generate_order_status_reports(&order_cmd).await?;

        let fill_cmd = nautilus_common::messages::execution::GenerateFillReportsBuilder::default()
            .ts_init(ts_init)
            .build()
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let fills = self.generate_fill_reports(fill_cmd).await?;

        let position_cmd =
            nautilus_common::messages::execution::GeneratePositionStatusReportsBuilder::default()
                .ts_init(ts_init)
                .build()
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        let positions = self.generate_position_status_reports(&position_cmd).await?;

        let mut mass_status = ExecutionMassStatus::new(
            self.core.client_id,
            self.core.account_id,
            self.core.venue,
            ts_init,
            None,
        );
        mass_status.add_order_reports(orders);
        mass_status.add_fill_reports(fills);
        mass_status.add_position_reports(positions);
        Ok(Some(mass_status))
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    fn test_order_reference_shape_and_determinism() {
        let id = ClientOrderId::new("O-20260829-103000-001-001-1");
        let reference = order_reference_for(&id);
        assert_eq!(reference.len(), 18);
        assert!(reference.starts_with("NT"));
        assert!(reference.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_eq!(reference, order_reference_for(&id));

        let other = order_reference_for(&ClientOrderId::new("O-20260829-103000-001-001-2"));
        assert_ne!(reference, other);
    }
}
