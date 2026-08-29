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

//! Downloads Groww historical candles into CSV files ready for backtesting.
//!
//! Reads `GROWW_API_KEY` and `GROWW_API_SECRET` from the environment or a `.env` file.
//!
//! ```text
//! groww-download-bars [SYMBOL ...]
//!
//! GROWW_INTERVAL  candle interval (default: 5minute)
//! GROWW_DAYS      how many days back to fetch (default: 365)
//! GROWW_OUT       output directory (default: ~/groww_data/bars)
//! ```
//!
//! One CSV is written per symbol with header `timestamp_utc,open,high,low,close,volume`, where
//! `timestamp_utc` is the bar **close** time in RFC 3339. The venue stamps candles with their
//! open time in naive Indian local time; stamping bars at close instead means a backtest engine
//! never sees a bar before the interval it summarizes has finished. An `instruments.json` is
//! also written carrying the tick size, precision, and lot size each symbol needs on the
//! Python side.

use std::{collections::BTreeMap, sync::Arc};

use jiff::{ToSpan, Zoned};
use nautilus_groww::{
    common::{
        credential::GrowwCredential,
        enums::{GrowwExchange, GrowwInstrumentType, GrowwSegment},
        parse::india_tz,
    },
    http::client::GrowwHttpClient,
    provider::{GrowwInstrumentProvider, InstrumentFilter},
};
use nautilus_model::instruments::Instrument;

/// Returns the venue's maximum request range, in days, for `interval`.
fn max_days_for(interval: &str) -> anyhow::Result<i64> {
    // Documented caps: 30 days for 1-5 minute, 90 for 10-30 minute, 180 for hourly and above.
    let days = match interval {
        "1minute" | "2minute" | "3minute" | "5minute" => 30,
        "10minute" | "15minute" | "30minute" => 90,
        "1hour" | "4hour" | "1day" | "1week" | "1month" => 180,
        other => anyhow::bail!("unknown candle interval `{other}`"),
    };
    Ok(days)
}

/// Returns the interval length in seconds, used to shift open-stamped candles to close time.
fn interval_seconds(interval: &str) -> anyhow::Result<i64> {
    let secs = match interval {
        "1minute" => 60,
        "2minute" => 120,
        "3minute" => 180,
        "5minute" => 300,
        "10minute" => 600,
        "15minute" => 900,
        "30minute" => 1_800,
        "1hour" => 3_600,
        "4hour" => 14_400,
        // Daily and coarser bars close at the 15:30 session close, handled separately.
        "1day" | "1week" | "1month" => 0,
        other => anyhow::bail!("unknown candle interval `{other}`"),
    };
    Ok(secs)
}

/// Converts a venue candle timestamp to the bar's close time as RFC 3339 UTC.
fn close_time_utc(venue_ts: &str, interval: &str) -> anyhow::Result<String> {
    let civil: jiff::civil::DateTime = venue_ts
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid candle timestamp `{venue_ts}`: {e}"))?;

    let secs = interval_seconds(interval)?;
    let civil = if secs > 0 {
        civil
            .checked_add(secs.seconds())
            .map_err(|e| anyhow::anyhow!("timestamp overflow for `{venue_ts}`: {e}"))?
    } else {
        // Daily and coarser candles are stamped at midnight; their bar closes with the session.
        civil.with().hour(15).minute(30).second(0).build()?
    };

    let zoned = civil
        .to_zoned(india_tz().map_err(|e| anyhow::anyhow!("{e}"))?)
        .map_err(|e| anyhow::anyhow!("unresolvable timestamp `{venue_ts}`: {e}"))?;
    Ok(zoned.timestamp().to_string())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt().with_env_filter("warn").init();

    let symbols: Vec<String> = {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.is_empty() {
            ["RELIANCE", "TCS", "INFY", "SUZLON", "RPOWER"]
                .map(String::from)
                .to_vec()
        } else {
            args
        }
    };
    let interval = std::env::var("GROWW_INTERVAL").unwrap_or_else(|_| "5minute".to_string());
    let days: i64 = std::env::var("GROWW_DAYS")
        .unwrap_or_else(|_| "365".to_string())
        .parse()?;
    let out_dir = std::env::var("GROWW_OUT").map_or_else(
        |_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            std::path::PathBuf::from(home)
                .join("groww_data")
                .join("bars")
        },
        std::path::PathBuf::from,
    );
    std::fs::create_dir_all(&out_dir)?;

    let chunk_days = max_days_for(&interval)?;

    let credential = GrowwCredential::resolve(None, None)
        .ok_or_else(|| anyhow::anyhow!("set GROWW_API_KEY and GROWW_API_SECRET"))?;
    let client = Arc::new(GrowwHttpClient::with_credentials(
        credential, None, 60, None, None,
    )?);

    // Resolve precision, tick size, and the venue's cross-exchange symbol per instrument.
    let provider = GrowwInstrumentProvider::new(Arc::clone(&client));
    provider
        .load(&InstrumentFilter {
            exchanges: vec![GrowwExchange::Nse],
            segments: vec![GrowwSegment::Cash],
            instrument_types: vec![GrowwInstrumentType::Equity],
        })
        .await?;

    let tz = india_tz().map_err(|e| anyhow::anyhow!("{e}"))?;
    let now: Zoned = Zoned::now().with_time_zone(tz);
    let range_start = now.checked_sub(days.days())?;

    let mut instrument_meta = Vec::new();

    for symbol in &symbols {
        let instrument_id = format!("{symbol}.NSE").parse()?;
        let Some(instrument) = provider.get(&instrument_id) else {
            eprintln!("SKIP {symbol}: not found in the NSE cash instrument master");
            continue;
        };
        let meta = provider
            .metadata(&instrument_id)
            .ok_or_else(|| anyhow::anyhow!("missing metadata for {instrument_id}"))?;

        // Fetch in venue-sized chunks, deduplicating on the venue timestamp across boundaries.
        let mut rows: BTreeMap<String, (f64, f64, f64, f64, f64)> = BTreeMap::new();
        let mut dropped_nulls = 0usize;
        let mut chunk_start = range_start.clone();

        while chunk_start < now {
            let chunk_end = chunk_start.checked_add(chunk_days.days())?.min(now.clone());

            let candles = client
                .http_get_candles(
                    GrowwExchange::Nse,
                    GrowwSegment::Cash,
                    &meta.groww_symbol,
                    &chunk_start.strftime("%Y-%m-%d %H:%M:%S").to_string(),
                    &chunk_end.strftime("%Y-%m-%d %H:%M:%S").to_string(),
                    &interval,
                )
                .await?;

            for candle in candles {
                // The venue occasionally publishes a candle with a null OHLC field; a bar cannot
                // be built from a partial row, so it is dropped and counted rather than invented.
                let Some((open, high, low, close, volume)) = candle.ohlcv() else {
                    dropped_nulls += 1;
                    continue;
                };
                let key = close_time_utc(candle.timestamp(), &interval)?;
                rows.insert(key, (open, high, low, close, volume));
            }

            chunk_start = chunk_end;
        }

        let path = out_dir.join(format!("{symbol}.NSE_{interval}.csv"));
        let mut csv = String::with_capacity(rows.len() * 64);
        csv.push_str("timestamp_utc,open,high,low,close,volume\n");
        for (ts, (open, high, low, close, volume)) in &rows {
            use std::fmt::Write as _;
            writeln!(csv, "{ts},{open},{high},{low},{close},{volume}")?;
        }
        std::fs::write(&path, csv)?;

        let dropped = if dropped_nulls > 0 {
            format!(" ({dropped_nulls} partial rows dropped)")
        } else {
            String::new()
        };
        println!(
            "{symbol}.NSE: {} bars -> {}{dropped}",
            rows.len(),
            path.display()
        );

        instrument_meta.push(serde_json::json!({
            "instrument_id": instrument_id.to_string(),
            "raw_symbol": symbol,
            "isin": instrument.isin().map(|i| i.to_string()),
            "currency": "INR",
            "price_precision": instrument.price_precision(),
            "price_increment": instrument.price_increment().to_string(),
            "lot_size": 1,
            "groww_symbol": meta.groww_symbol,
            "exchange_token": meta.exchange_token,
        }));
    }

    let meta_path = out_dir.join("instruments.json");
    std::fs::write(&meta_path, serde_json::to_string_pretty(&instrument_meta)?)?;
    println!("instrument metadata -> {}", meta_path.display());

    Ok(())
}
