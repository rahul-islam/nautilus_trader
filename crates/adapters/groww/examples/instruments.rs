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

//! Loads the Groww instrument master and reports what was parsed.
//!
//! Reads `GROWW_API_KEY` and `GROWW_API_SECRET` from the environment or a `.env` file, though the
//! instrument master itself is served without authentication.

use std::sync::Arc;

use nautilus_groww::{
    common::{
        credential::GrowwCredential,
        enums::{GrowwExchange, GrowwInstrumentType, GrowwSegment},
    },
    http::client::GrowwHttpClient,
    provider::{GrowwInstrumentProvider, InstrumentFilter},
};
use nautilus_model::instruments::Instrument;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt().with_env_filter("info").init();

    let credential = GrowwCredential::resolve(None, None);
    let client = match credential {
        Some(credential) => GrowwHttpClient::with_credentials(credential, None, 60, None, None)?,
        None => GrowwHttpClient::new(None, 60, None, None)?,
    };
    let provider = GrowwInstrumentProvider::new(Arc::new(client));

    // Equities on the two cash exchanges, the universe this account trades.
    let filter = InstrumentFilter {
        exchanges: vec![GrowwExchange::Nse, GrowwExchange::Bse],
        segments: vec![GrowwSegment::Cash],
        instrument_types: vec![GrowwInstrumentType::Equity],
    };
    let equities = provider.load(&filter).await?;
    println!("cash equities loaded: {}", equities.len());

    for id in ["RELIANCE.NSE", "RELIANCE.BSE", "SUZLON.NSE"] {
        let instrument_id = id.parse()?;
        match provider.get(&instrument_id) {
            Some(instrument) => println!(
                "  {id}: tick={} precision={} lot={:?} currency={}",
                instrument.price_increment(),
                instrument.price_precision(),
                instrument.lot_size(),
                instrument.quote_currency(),
            ),
            None => println!("  {id}: not found"),
        }
        if let Some(meta) = provider.metadata(&instrument_id) {
            println!(
                "      token={} groww_symbol={} segment={}",
                meta.exchange_token, meta.groww_symbol, meta.segment
            );
        }
    }

    // Now the whole master, to exercise every instrument variant.
    let all = GrowwInstrumentProvider::new(Arc::new(GrowwHttpClient::new(None, 60, None, None)?));
    let loaded = all.load(&InstrumentFilter::default()).await?;
    let mut equity = 0usize;
    let mut futures = 0usize;
    let mut options = 0usize;
    let mut indices = 0usize;
    for instrument in &loaded {
        match instrument {
            nautilus_model::instruments::InstrumentAny::Equity(_) => equity += 1,
            nautilus_model::instruments::InstrumentAny::FuturesContract(_) => futures += 1,
            nautilus_model::instruments::InstrumentAny::OptionContract(_) => options += 1,
            nautilus_model::instruments::InstrumentAny::IndexInstrument(_) => indices += 1,
            _ => {}
        }
    }
    println!(
        "full master: {} total ({equity} equity, {futures} futures, {options} options, {indices} indices)",
        loaded.len()
    );

    Ok(())
}
