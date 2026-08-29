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

//! Fetches live Groww market data for a few NSE equities and prints it.
//!
//! Reads `GROWW_API_KEY` and `GROWW_API_SECRET` from the environment or a `.env` file. Read-only:
//! it never places, modifies, or cancels an order.
//!
//! Outside Indian market hours the venue returns the last values from the previous session.

use std::sync::Arc;

use nautilus_groww::{
    common::{
        credential::GrowwCredential,
        enums::{GrowwExchange, GrowwInstrumentType, GrowwSegment},
    },
    http::client::GrowwHttpClient,
    provider::{GrowwInstrumentProvider, InstrumentFilter},
};
use nautilus_model::{instruments::Instrument, types::Price};

const SYMBOLS: [&str; 5] = ["RELIANCE", "TCS", "INFY", "SUZLON", "RPOWER"];

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt().with_env_filter("warn").init();

    let credential = GrowwCredential::resolve(None, None)
        .ok_or_else(|| anyhow::anyhow!("set GROWW_API_KEY and GROWW_API_SECRET"))?;
    let client = Arc::new(GrowwHttpClient::with_credentials(
        credential, None, 30, None, None,
    )?);

    // Load the NSE cash universe so quotes can be rendered on each instrument's own price grid.
    let provider = GrowwInstrumentProvider::new(Arc::clone(&client));
    provider
        .load(&InstrumentFilter {
            exchanges: vec![GrowwExchange::Nse],
            segments: vec![GrowwSegment::Cash],
            instrument_types: vec![GrowwInstrumentType::Equity],
        })
        .await?;
    println!("Loaded {} NSE cash equities\n", provider.count());

    let keys: Vec<String> = SYMBOLS.iter().map(|s| format!("NSE_{s}")).collect();
    let ltps = client.http_get_ltp(GrowwSegment::Cash, &keys).await?;

    println!(
        "{:<12} {:>12} {:>12} {:>12} {:>12}",
        "SYMBOL", "LTP", "OPEN", "HIGH", "LOW"
    );
    println!("{}", "-".repeat(64));

    for symbol in SYMBOLS {
        let instrument_id = format!("{symbol}.NSE").parse()?;
        let precision = provider
            .get(&instrument_id)
            .map_or(2, |i| i.price_precision());

        let quote = client
            .http_get_quote(GrowwExchange::Nse, GrowwSegment::Cash, symbol)
            .await?;

        // Render through the domain type so every price sits on the instrument's own tick grid.
        let fmt = |value: f64| Price::new(value, precision).to_string();
        println!(
            "{symbol:<12} {:>12} {:>12} {:>12} {:>12}",
            fmt(quote.last_price),
            fmt(quote.ohlc.open),
            fmt(quote.ohlc.high),
            fmt(quote.ohlc.low),
        );

        // Cross-check the bulk LTP endpoint against the per-instrument quote.
        if let Some(ltp) = ltps.get(&format!("NSE_{symbol}"))
            && (ltp - quote.last_price).abs() > f64::EPSILON
        {
            println!("{:<12} (bulk LTP reports {})", "", fmt(*ltp));
        }
    }

    // Five-level depth for one instrument, the shape the feed streams live.
    let quote = client
        .http_get_quote(GrowwExchange::Nse, GrowwSegment::Cash, "RELIANCE")
        .await?;
    println!("\nRELIANCE.NSE depth (5 levels):");
    println!(
        "{:>14} {:>10}  |  {:<10} {:<14}",
        "BID QTY", "BID", "ASK", "ASK QTY"
    );
    for i in 0..5 {
        let bid = quote.depth.buy.get(i);
        let ask = quote.depth.sell.get(i);
        println!(
            "{:>14} {:>10}  |  {:<10} {:<14}",
            bid.map_or(0.0, |l| l.quantity),
            bid.map_or(0.0, |l| l.price),
            ask.map_or(0.0, |l| l.price),
            ask.map_or(0.0, |l| l.quantity),
        );
    }

    // A day of five-minute candles, the same path a bar request would take.
    let candles = client
        .http_get_candles(
            GrowwExchange::Nse,
            GrowwSegment::Cash,
            "NSE-RELIANCE",
            "2026-08-27 09:15:00",
            "2026-08-27 15:30:00",
            "5minute",
        )
        .await?;
    println!("\nRELIANCE.NSE 5-minute candles: {} bars", candles.len());
    for candle in candles.iter().take(3) {
        if let Some((open, high, low, close, volume)) = candle.ohlcv() {
            println!(
                "  {}  O={open} H={high} L={low} C={close} V={volume}",
                candle.timestamp(),
            );
        }
    }

    Ok(())
}
