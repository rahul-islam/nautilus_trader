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

//! Instrument provider backed by the Groww instrument master.
//!
//! Groww publishes every tradable instrument as a single unauthenticated CSV refreshed daily. The
//! file covers all exchanges and segments in one listing, so a row's `exchange` column decides the
//! Nautilus venue and its `instrument_type` decides which instrument variant is built.

use std::sync::Arc;

use ahash::AHashMap;
use nautilus_core::{AtomicMap, UnixNanos, time::get_atomic_clock_realtime};
use nautilus_model::{
    enums::{AssetClass, OptionKind},
    identifiers::{InstrumentId, Symbol, Venue},
    instruments::{
        Equity, FuturesContract, IndexInstrument, Instrument, InstrumentAny, OptionContract,
    },
    types::{Currency, Price, Quantity},
};
use serde::Deserialize;
use ustr::Ustr;

use crate::{
    common::{
        consts::{BSE_VENUE, MCX_VENUE, NSE_VENUE},
        enums::{GrowwExchange, GrowwInstrumentType, GrowwSegment},
        parse::{parse_expiry_date, precision_from_str},
    },
    http::{
        client::GrowwHttpClient,
        error::{Error, Result},
    },
};

/// A single row of the Groww instrument master.
#[derive(Debug, Clone, Deserialize)]
pub struct InstrumentRecord {
    /// Exchange the instrument is listed on.
    pub exchange: GrowwExchange,
    /// Exchange-assigned numeric token, used as the feed subject suffix.
    pub exchange_token: String,
    /// Venue trading symbol, used on the order endpoints.
    pub trading_symbol: String,
    /// Groww's cross-exchange instrument key, used on the historical endpoints.
    pub groww_symbol: String,
    /// Descriptive name, populated for cash instruments.
    #[serde(default)]
    pub name: String,
    /// Instrument type.
    pub instrument_type: GrowwInstrumentType,
    /// Trading segment.
    pub segment: GrowwSegment,
    /// Exchange series code, such as `EQ` on the NSE.
    #[serde(default)]
    pub series: String,
    /// International Securities Identification Number.
    #[serde(default)]
    pub isin: String,
    /// Underlying trading symbol, for derivatives.
    #[serde(default)]
    pub underlying_symbol: String,
    /// Underlying exchange token, for derivatives.
    #[serde(default)]
    pub underlying_exchange_token: String,
    /// Expiry date as `YYYY-MM-DD`, for derivatives.
    #[serde(default)]
    pub expiry_date: String,
    /// Strike price, for options.
    #[serde(default)]
    pub strike_price: String,
    /// Contract lot size.
    #[serde(default)]
    pub lot_size: String,
    /// Minimum price increment.
    #[serde(default)]
    pub tick_size: String,
    /// Maximum single-order quantity permitted by the exchange.
    #[serde(default)]
    pub freeze_quantity: String,
    /// Whether the instrument is reserved and not generally tradable.
    #[serde(default)]
    pub is_reserved: String,
    /// Whether buy orders are permitted.
    #[serde(default)]
    pub buy_allowed: String,
    /// Whether sell orders are permitted.
    #[serde(default)]
    pub sell_allowed: String,
    /// Broker-internal trading symbol.
    #[serde(default)]
    pub internal_trading_symbol: String,
    /// Whether the instrument is intraday-only.
    #[serde(default)]
    pub is_intraday: String,
}

impl InstrumentRecord {
    /// Returns the Nautilus venue for this row.
    #[must_use]
    pub fn venue(&self) -> Venue {
        match self.exchange {
            GrowwExchange::Nse => *NSE_VENUE,
            GrowwExchange::Bse => *BSE_VENUE,
            GrowwExchange::Mcx => *MCX_VENUE,
        }
    }

    /// Returns the Nautilus instrument ID for this row.
    ///
    /// # Errors
    ///
    /// Returns an error if the row carries no trading symbol. The master contains a small number
    /// of such rows, all of them indices the venue publishes without a symbol.
    pub fn instrument_id(&self) -> Result<InstrumentId> {
        let symbol = self.trading_symbol.trim();
        if symbol.is_empty() {
            return Err(Error::decode(format!(
                "instrument master row for exchange_token `{}` has no trading symbol",
                self.exchange_token
            )));
        }
        Ok(InstrumentId::new(Symbol::from(symbol), self.venue()))
    }
}

/// Metadata a row carries that the Nautilus instrument types do not model.
///
/// The exchange token is the feed subject suffix and the Groww symbol keys the historical
/// endpoints, so both must survive alongside the parsed instrument.
#[derive(Debug, Clone)]
pub struct InstrumentMetadata {
    /// Exchange-assigned numeric token, used as the feed subject suffix.
    pub exchange_token: String,
    /// Groww's cross-exchange instrument key.
    pub groww_symbol: String,
    /// Exchange the instrument is listed on.
    pub exchange: GrowwExchange,
    /// Trading segment.
    pub segment: GrowwSegment,
    /// Venue trading symbol.
    pub trading_symbol: String,
}

/// Price precision used for index instruments, which the master leaves blank.
const INDEX_PRICE_PRECISION: u8 = 2;

/// Price increment used for index instruments, which the master leaves blank.
const INDEX_PRICE_INCREMENT: f64 = 0.01;

/// Asset class assigned to every instrument from this venue.
///
/// Groww's tradable universe is Indian equities, equity derivatives, and commodity derivatives.
/// Commodity rows are distinguished by segment rather than by a separate asset class column.
const fn asset_class_for(segment: GrowwSegment) -> AssetClass {
    match segment {
        GrowwSegment::Commodity => AssetClass::Commodity,
        _ => AssetClass::Equity,
    }
}

/// Parses one instrument master row into a Nautilus instrument.
///
/// # Errors
///
/// Returns an error if a field required for the row's instrument type is missing or malformed.
pub fn parse_instrument(record: &InstrumentRecord, ts_init: UnixNanos) -> Result<InstrumentAny> {
    let instrument_id = record.instrument_id()?;
    let raw_symbol = Symbol::from(record.trading_symbol.trim());
    let currency = Currency::INR();

    // Index rows carry no tick size because an index is quoted rather than traded. Nautilus still
    // requires a positive increment, so they fall back to the two-decimal grid indices are
    // published on.
    let (price_precision, tick_size) = if record.tick_size.trim().is_empty() {
        (INDEX_PRICE_PRECISION, INDEX_PRICE_INCREMENT)
    } else {
        let precision = precision_from_str(&record.tick_size);
        let tick: f64 = record.tick_size.trim().parse().map_err(|_| {
            Error::decode(format!(
                "invalid tick_size `{}` for {instrument_id}",
                record.tick_size
            ))
        })?;
        (precision, tick)
    };
    let price_increment = Price::new(tick_size, price_precision);

    // The master reports lot size as the exchange's contract multiplier. Cash rows carry `1`.
    let lot_size_value: f64 = if record.lot_size.trim().is_empty() {
        1.0
    } else {
        record.lot_size.trim().parse().map_err(|_| {
            Error::decode(format!(
                "invalid lot_size `{}` for {instrument_id}",
                record.lot_size
            ))
        })?
    };
    let lot_size = Quantity::new(lot_size_value, 0);

    // The exchange freeze quantity is the largest quantity a single order may carry.
    let max_quantity = match record.freeze_quantity.trim() {
        "" => None,
        value => value
            .parse::<f64>()
            .ok()
            .filter(|q| *q > 0.0)
            .map(|q| Quantity::new(q, 0)),
    };

    let isin = (!record.isin.trim().is_empty()).then(|| Ustr::from(record.isin.trim()));
    let exchange = Some(Ustr::from(record.exchange.as_ref()));
    let asset_class = asset_class_for(record.segment);

    match record.instrument_type {
        GrowwInstrumentType::Index => Ok(InstrumentAny::IndexInstrument(
            IndexInstrument::builder()
                .instrument_id(instrument_id)
                .raw_symbol(raw_symbol)
                .currency(currency)
                .price_precision(price_precision)
                // An index level has no traded size, so the size grid is whole units.
                .size_precision(0)
                .price_increment(price_increment)
                .size_increment(Quantity::new(1.0, 0))
                .ts_event(ts_init)
                .ts_init(ts_init)
                .build()
                .map_err(|e| Error::decode(format!("invalid index {instrument_id}: {e}")))?,
        )),
        GrowwInstrumentType::Equity => Ok(InstrumentAny::Equity(
            Equity::builder()
                .instrument_id(instrument_id)
                .raw_symbol(raw_symbol)
                .maybe_isin(isin)
                .currency(currency)
                .price_precision(price_precision)
                .price_increment(price_increment)
                .maybe_lot_size(Some(lot_size))
                .maybe_max_quantity(max_quantity)
                .ts_event(ts_init)
                .ts_init(ts_init)
                .build()
                .map_err(|e| Error::decode(format!("invalid equity {instrument_id}: {e}")))?,
        )),
        GrowwInstrumentType::Future => {
            let expiration_ns = parse_expiry_date(record.expiry_date.trim())?;
            Ok(InstrumentAny::FuturesContract(
                FuturesContract::builder()
                    .instrument_id(instrument_id)
                    .raw_symbol(raw_symbol)
                    .asset_class(asset_class)
                    .maybe_exchange(exchange)
                    .underlying(underlying_of(record))
                    // The master publishes no listing date, so activation is taken as the epoch:
                    // an instrument absent from today's file is simply not loaded.
                    .activation_ns(UnixNanos::default())
                    .expiration_ns(expiration_ns)
                    .currency(currency)
                    .price_precision(price_precision)
                    .price_increment(price_increment)
                    .multiplier(lot_size)
                    .lot_size(lot_size)
                    .maybe_max_quantity(max_quantity)
                    .ts_event(ts_init)
                    .ts_init(ts_init)
                    .build()
                    .map_err(|e| {
                        Error::decode(format!("invalid futures contract {instrument_id}: {e}"))
                    })?,
            ))
        }
        GrowwInstrumentType::Call | GrowwInstrumentType::Put => {
            let expiration_ns = parse_expiry_date(record.expiry_date.trim())?;
            let strike: f64 = record.strike_price.trim().parse().map_err(|_| {
                Error::decode(format!(
                    "invalid strike_price `{}` for {instrument_id}",
                    record.strike_price
                ))
            })?;
            let option_kind = match record.instrument_type {
                GrowwInstrumentType::Call => OptionKind::Call,
                _ => OptionKind::Put,
            };
            // A strike is quoted on the same grid as the instrument's price.
            let strike_price = Price::new(strike, price_precision);

            Ok(InstrumentAny::OptionContract(
                OptionContract::builder()
                    .instrument_id(instrument_id)
                    .raw_symbol(raw_symbol)
                    .asset_class(asset_class)
                    .maybe_exchange(exchange)
                    .underlying(underlying_of(record))
                    .option_kind(option_kind)
                    .strike_price(strike_price)
                    .currency(currency)
                    .activation_ns(UnixNanos::default())
                    .expiration_ns(expiration_ns)
                    .price_precision(price_precision)
                    .price_increment(price_increment)
                    .multiplier(lot_size)
                    .lot_size(lot_size)
                    .maybe_max_quantity(max_quantity)
                    .ts_event(ts_init)
                    .ts_init(ts_init)
                    .build()
                    .map_err(|e| {
                        Error::decode(format!("invalid option contract {instrument_id}: {e}"))
                    })?,
            ))
        }
    }
}

/// Returns the underlying symbol for a derivative row, falling back to its own trading symbol.
fn underlying_of(record: &InstrumentRecord) -> Ustr {
    if record.underlying_symbol.trim().is_empty() {
        Ustr::from(record.trading_symbol.as_str())
    } else {
        Ustr::from(record.underlying_symbol.trim())
    }
}

/// Filter applied when loading the instrument master.
///
/// The master carries over 135,000 rows across every exchange and segment, the bulk of them option
/// contracts. Loading all of them costs memory and parse time that a strategy trading a handful of
/// equities has no use for, so callers narrow the load rather than paying for the whole file.
#[derive(Debug, Clone, Default)]
pub struct InstrumentFilter {
    /// Exchanges to load. Empty loads every exchange.
    pub exchanges: Vec<GrowwExchange>,
    /// Segments to load. Empty loads every segment.
    pub segments: Vec<GrowwSegment>,
    /// Instrument types to load. Empty loads every type.
    pub instrument_types: Vec<GrowwInstrumentType>,
}

impl InstrumentFilter {
    /// Returns whether `record` passes the filter.
    #[must_use]
    pub fn matches(&self, record: &InstrumentRecord) -> bool {
        (self.exchanges.is_empty() || self.exchanges.contains(&record.exchange))
            && (self.segments.is_empty() || self.segments.contains(&record.segment))
            && (self.instrument_types.is_empty()
                || self.instrument_types.contains(&record.instrument_type))
    }
}

/// Loads and caches Groww instruments from the venue's instrument master.
#[derive(Debug, Clone)]
pub struct GrowwInstrumentProvider {
    client: Arc<GrowwHttpClient>,
    instruments: Arc<AtomicMap<InstrumentId, InstrumentAny>>,
    metadata: Arc<AtomicMap<InstrumentId, InstrumentMetadata>>,
}

impl GrowwInstrumentProvider {
    /// Creates a new [`GrowwInstrumentProvider`].
    #[must_use]
    pub fn new(client: Arc<GrowwHttpClient>) -> Self {
        Self {
            client,
            instruments: Arc::new(AtomicMap::new()),
            metadata: Arc::new(AtomicMap::new()),
        }
    }

    /// Returns the instrument cache.
    #[must_use]
    pub fn instruments(&self) -> &Arc<AtomicMap<InstrumentId, InstrumentAny>> {
        &self.instruments
    }

    /// Returns the number of cached instruments.
    #[must_use]
    pub fn count(&self) -> usize {
        self.instruments.len()
    }

    /// Returns a cached instrument by ID, if present.
    #[must_use]
    pub fn get(&self, instrument_id: &InstrumentId) -> Option<InstrumentAny> {
        self.instruments.get_cloned(instrument_id)
    }

    /// Returns the venue metadata for an instrument, if present.
    #[must_use]
    pub fn metadata(&self, instrument_id: &InstrumentId) -> Option<InstrumentMetadata> {
        self.metadata.get_cloned(instrument_id)
    }

    /// Loads instruments matching `filter` from the venue's instrument master.
    ///
    /// # Errors
    ///
    /// Returns an error if the master cannot be fetched or its header cannot be parsed.
    pub async fn load(&self, filter: &InstrumentFilter) -> Result<Vec<InstrumentAny>> {
        let csv = self.client.http_get_instruments_csv().await?;
        self.load_from_csv(&csv, filter)
    }

    /// Parses `csv` and caches every instrument matching `filter`.
    ///
    /// A row that fails to parse is skipped with a warning rather than failing the load: the master
    /// covers every exchange and segment, and one malformed contract should not deny a strategy
    /// the rest of the universe.
    ///
    /// # Errors
    ///
    /// Returns an error if the CSV header cannot be parsed.
    pub fn load_from_csv(
        &self,
        csv: &str,
        filter: &InstrumentFilter,
    ) -> Result<Vec<InstrumentAny>> {
        let ts_init = get_atomic_clock_realtime().get_time_ns();
        let mut reader = csv::Reader::from_reader(csv.as_bytes());
        let mut loaded = Vec::new();
        let mut skipped: AHashMap<String, usize> = AHashMap::new();

        // `AtomicMap` writes clone the whole map, so the master's six-figure row count is staged
        // here and committed in a single swap per map. Inserting row by row would make loading
        // quadratic in the number of instruments.
        let mut staged_instruments: Vec<(InstrumentId, InstrumentAny)> = Vec::new();
        let mut staged_metadata: Vec<(InstrumentId, InstrumentMetadata)> = Vec::new();

        for result in reader.deserialize::<InstrumentRecord>() {
            let record = match result {
                Ok(record) => record,
                Err(e) => {
                    *skipped.entry(row_error_kind(&e)).or_default() += 1;
                    continue;
                }
            };

            if !filter.matches(&record) {
                continue;
            }

            match parse_instrument(&record, ts_init) {
                Ok(instrument) => {
                    let instrument_id = instrument.id();
                    staged_metadata.push((
                        instrument_id,
                        InstrumentMetadata {
                            exchange_token: record.exchange_token.clone(),
                            groww_symbol: record.groww_symbol.clone(),
                            exchange: record.exchange,
                            segment: record.segment,
                            trading_symbol: record.trading_symbol.clone(),
                        },
                    ));
                    staged_instruments.push((instrument_id, instrument.clone()));
                    loaded.push(instrument);
                }
                Err(e) => {
                    *skipped.entry(e.to_string()).or_default() += 1;
                }
            }
        }

        self.instruments.rcu(|map| {
            for (id, instrument) in &staged_instruments {
                map.insert(*id, instrument.clone());
            }
        });
        self.metadata.rcu(|map| {
            for (id, meta) in &staged_metadata {
                map.insert(*id, meta.clone());
            }
        });

        if !skipped.is_empty() {
            let total: usize = skipped.values().sum();
            // Report a bounded summary: a malformed column can affect tens of thousands of rows,
            // and logging each one would bury the rest of the startup log.
            let mut kinds: Vec<_> = skipped.into_iter().collect();
            kinds.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
            kinds.truncate(5);
            tracing::warn!(
                "Skipped {total} Groww instrument rows; most common: {}",
                kinds
                    .iter()
                    .map(|(kind, count)| format!("{count}x {kind}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            );
        }

        tracing::info!("Loaded {} Groww instruments", loaded.len());
        Ok(loaded)
    }
}

/// Reduces a CSV row error to a stable label so repeated failures aggregate.
fn row_error_kind(error: &csv::Error) -> String {
    match error.kind() {
        csv::ErrorKind::Deserialize { err, .. } => format!("deserialize: {err}"),
        csv::ErrorKind::UnequalLengths { .. } => "unequal field count".to_string(),
        other => format!("{other:?}"),
    }
}
