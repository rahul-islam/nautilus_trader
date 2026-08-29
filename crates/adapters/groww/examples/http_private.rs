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

//! Exercises the authenticated Groww REST endpoints against the live venue.
//!
//! Reads `GROWW_API_KEY` and `GROWW_API_SECRET` from the environment or a `.env` file. Only issues
//! read requests; it never places, modifies, or cancels an order.

use nautilus_groww::{
    common::{credential::GrowwCredential, enums::GrowwSegment},
    http::client::GrowwHttpClient,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt().with_env_filter("info").init();

    let credential = GrowwCredential::resolve(None, None)
        .ok_or_else(|| anyhow::anyhow!("set GROWW_API_KEY and GROWW_API_SECRET"))?;
    let client = GrowwHttpClient::with_credentials(credential, None, 30, None, None)?;

    let user = client.http_get_user_detail().await?;
    println!("user: ucc={} segments={:?}", user.ucc, user.active_segments);

    let margin = client.http_get_margin().await?;
    println!(
        "margin: clear_cash={} net_used={}",
        margin.clear_cash, margin.net_margin_used
    );

    let holdings = client.http_get_holdings().await?;
    println!("holdings: {} rows", holdings.len());
    for holding in holdings.iter().take(3) {
        println!(
            "  {} qty={} avg={}",
            holding.trading_symbol, holding.quantity, holding.average_price
        );
    }

    let positions = client.http_get_positions().await?;
    println!("positions: {} rows", positions.len());

    let orders = client
        .http_get_order_list(GrowwSegment::Cash, 0, 10)
        .await?;
    println!("orders (CASH): {} rows", orders.len());
    for order in orders.iter().take(3) {
        println!(
            "  {} {:?} qty={} filled={}",
            order.groww_order_id, order.order_status, order.quantity, order.filled_quantity
        );
    }

    let csv = client.http_get_instruments_csv().await?;
    println!("instrument master: {} bytes", csv.len());

    Ok(())
}
