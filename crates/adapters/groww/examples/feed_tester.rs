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

//! Connects to the Groww streaming feed and prints events.
//!
//! Reads `GROWW_API_KEY` and `GROWW_API_SECRET` from the environment or a `.env` file. Outside
//! Indian market hours the connection and subscriptions succeed but no market data flows.

use std::{sync::Arc, time::Duration};

use nautilus_groww::{
    common::{
        credential::GrowwCredential,
        enums::{GrowwExchange, GrowwSegment},
    },
    http::client::GrowwHttpClient,
    websocket::{
        client::{FeedEvent, GrowwFeedClient, wait_connected},
        messages::{FeedKind, equity_order_updates_subject, market_data_subject},
    },
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt().with_env_filter("info").init();

    let credential = GrowwCredential::resolve(None, None)
        .ok_or_else(|| anyhow::anyhow!("set GROWW_API_KEY and GROWW_API_SECRET"))?;
    let http = Arc::new(GrowwHttpClient::with_credentials(
        credential, None, 30, None, None,
    )?);

    let mut feed = GrowwFeedClient::new(Arc::clone(&http), None);
    let mut events = feed.connect().await?;
    wait_connected(&mut events, Duration::from_secs(15)).await?;
    println!("HANDSHAKE OK: authenticated and connected");

    let subscription_id = feed
        .subscription_id()
        .await
        .ok_or_else(|| anyhow::anyhow!("missing subscription id"))?;
    println!("subscription_id: {subscription_id}");

    // RELIANCE (2885) and TCS (11536) live prices, RELIANCE depth, and account order updates.
    for subject in [
        market_data_subject(
            GrowwSegment::Cash,
            GrowwExchange::Nse,
            FeedKind::LivePrice,
            "2885",
        )?,
        market_data_subject(
            GrowwSegment::Cash,
            GrowwExchange::Nse,
            FeedKind::LivePrice,
            "11536",
        )?,
        market_data_subject(
            GrowwSegment::Cash,
            GrowwExchange::Nse,
            FeedKind::MarketDepth,
            "2885",
        )?,
        equity_order_updates_subject(&subscription_id),
    ] {
        println!("subscribing: {subject}");
        feed.subscribe(subject).await?;
    }

    println!("listening for 20s (market data flows only during NSE hours)...");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut count = 0usize;
    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(FeedEvent::Message { subject, message })) => {
                count += 1;
                println!("MESSAGE {count} [{subject}]: {message:?}");
                if count >= 10 {
                    break;
                }
            }
            Ok(Some(event)) => println!("EVENT: {event:?}"),
            Ok(None) => break,
            Err(_) => break, // Deadline reached.
        }
    }
    println!("received {count} feed messages");

    feed.disconnect();
    Ok(())
}
