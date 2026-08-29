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

//! Factories for creating Groww clients from a trading node.

use std::{any::Any, cell::RefCell, rc::Rc};

use nautilus_common::{
    cache::CacheView,
    clients::{DataClient, ExecutionClient},
    clock::Clock,
    factories::{ClientConfig, DataClientFactory, ExecutionClientFactory},
};
use nautilus_live::ExecutionClientCore;
use nautilus_model::{
    enums::{AccountType, OmsType},
    identifiers::{AccountId, ClientId, TraderId},
    types::Currency,
};

use crate::{
    common::consts::{GROWW, NSE_VENUE},
    config::{GrowwDataClientConfig, GrowwExecClientConfig},
    data::GrowwDataClient,
    execution::GrowwExecutionClient,
};

impl ClientConfig for GrowwDataClientConfig {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl ClientConfig for GrowwExecClientConfig {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Factory for creating Groww data clients.
#[derive(Debug, Default, Clone)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(module = "nautilus_trader.adapters.groww", from_py_object)
)]
#[cfg_attr(
    feature = "python",
    pyo3_stub_gen::derive::gen_stub_pyclass(module = "nautilus_trader.adapters.groww")
)]
pub struct GrowwDataClientFactory;

impl GrowwDataClientFactory {
    /// Creates a new [`GrowwDataClientFactory`] instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl DataClientFactory for GrowwDataClientFactory {
    fn create(
        &self,
        name: &str,
        config: &dyn ClientConfig,
        _cache: CacheView,
        _clock: Rc<RefCell<dyn Clock>>,
    ) -> anyhow::Result<Box<dyn DataClient>> {
        let config = config
            .as_any()
            .downcast_ref::<GrowwDataClientConfig>()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Invalid config type for GrowwDataClientFactory; expected GrowwDataClientConfig"
                )
            })?
            .clone();

        let client = GrowwDataClient::new(Some(ClientId::from(name)), config)?;
        Ok(Box::new(client))
    }

    fn name(&self) -> &'static str {
        GROWW
    }

    fn config_type(&self) -> &'static str {
        "GrowwDataClientConfig"
    }
}

/// Factory for creating Groww execution clients.
///
/// Indian equity trading settles through a cash account; the venue exposes no hedge mode, so the
/// OMS is always netting. The venue spans several exchanges, and the NSE venue is registered as
/// the client's primary; instruments carry their own venue for routing.
#[derive(Debug, Default, Clone)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(module = "nautilus_trader.adapters.groww", from_py_object)
)]
#[cfg_attr(
    feature = "python",
    pyo3_stub_gen::derive::gen_stub_pyclass(module = "nautilus_trader.adapters.groww")
)]
pub struct GrowwExecutionClientFactory;

impl GrowwExecutionClientFactory {
    /// Creates a new [`GrowwExecutionClientFactory`] instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl ExecutionClientFactory for GrowwExecutionClientFactory {
    fn create(
        &self,
        trader_id: TraderId,
        name: &str,
        config: &dyn ClientConfig,
        cache: CacheView,
    ) -> anyhow::Result<Box<dyn ExecutionClient>> {
        let config = config
            .as_any()
            .downcast_ref::<GrowwExecClientConfig>()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Invalid config type for GrowwExecutionClientFactory; expected GrowwExecClientConfig"
                )
            })?
            .clone();

        let account_id = config
            .account_id
            .unwrap_or_else(|| AccountId::new("GROWW-001"));

        let core = ExecutionClientCore::new(
            trader_id,
            ClientId::from(name),
            *NSE_VENUE,
            OmsType::Netting,
            account_id,
            AccountType::Cash,
            Some(Currency::INR()),
            cache,
        );

        let client = GrowwExecutionClient::new(core, config)?;
        Ok(Box::new(client))
    }

    fn name(&self) -> &'static str {
        GROWW
    }

    fn config_type(&self) -> &'static str {
        "GrowwExecClientConfig"
    }
}
