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

//! Python bindings for Groww configuration.

use std::str::FromStr;

use nautilus_core::python::to_pyvalue_err;
use nautilus_model::identifiers::AccountId;
use pyo3::{PyResult, pymethods};

use crate::{
    common::enums::{GrowwExchange, GrowwInstrumentType, GrowwProduct, GrowwSegment},
    config::{GrowwDataClientConfig, GrowwExecClientConfig},
};

fn parse_list<T: FromStr>(values: Option<Vec<String>>, what: &str) -> PyResult<Vec<T>>
where
    T::Err: std::fmt::Display,
{
    values
        .unwrap_or_default()
        .iter()
        .map(|value| {
            T::from_str(value).map_err(|e| to_pyvalue_err(format!("invalid {what} `{value}`: {e}")))
        })
        .collect()
}

#[pymethods]
#[pyo3_stub_gen::derive::gen_stub_pymethods]
impl GrowwDataClientConfig {
    /// Configuration for the Groww live data client.
    ///
    /// Exchange, segment, and instrument-type filters take the venue's own strings
    /// (`"NSE"`, `"CASH"`, `"EQ"`, ...); an omitted filter falls back to the NSE and BSE
    /// cash segment.
    #[new]
    #[pyo3(signature = (
        api_key = None,
        api_secret = None,
        base_url_http = None,
        base_url_ws = None,
        http_timeout_secs = None,
        exchanges = None,
        segments = None,
        instrument_types = None,
    ))]
    #[expect(clippy::too_many_arguments)]
    fn py_new(
        api_key: Option<String>,
        api_secret: Option<String>,
        base_url_http: Option<String>,
        base_url_ws: Option<String>,
        http_timeout_secs: Option<u64>,
        exchanges: Option<Vec<String>>,
        segments: Option<Vec<String>>,
        instrument_types: Option<Vec<String>>,
    ) -> PyResult<Self> {
        let defaults = Self::default();
        Ok(Self {
            api_key,
            api_secret,
            base_url_http,
            base_url_ws,
            http_timeout_secs: http_timeout_secs.unwrap_or(defaults.http_timeout_secs),
            exchanges: match exchanges {
                Some(values) => parse_list::<GrowwExchange>(Some(values), "exchange")?,
                None => defaults.exchanges,
            },
            segments: match segments {
                Some(values) => parse_list::<GrowwSegment>(Some(values), "segment")?,
                None => defaults.segments,
            },
            instrument_types: parse_list::<GrowwInstrumentType>(
                instrument_types,
                "instrument type",
            )?,
        })
    }
}

#[pymethods]
#[pyo3_stub_gen::derive::gen_stub_pymethods]
impl GrowwExecClientConfig {
    /// Configuration for the Groww live execution client.
    ///
    /// `product` selects the settlement treatment sent with orders: `"CNC"` for delivery or
    /// `"MIS"` for intraday.
    #[new]
    #[pyo3(signature = (
        api_key = None,
        api_secret = None,
        base_url_http = None,
        base_url_ws = None,
        http_timeout_secs = None,
        account_id = None,
        product = None,
        exchanges = None,
        segments = None,
    ))]
    #[expect(clippy::too_many_arguments)]
    fn py_new(
        api_key: Option<String>,
        api_secret: Option<String>,
        base_url_http: Option<String>,
        base_url_ws: Option<String>,
        http_timeout_secs: Option<u64>,
        account_id: Option<AccountId>,
        product: Option<String>,
        exchanges: Option<Vec<String>>,
        segments: Option<Vec<String>>,
    ) -> PyResult<Self> {
        let defaults = Self::default();
        Ok(Self {
            api_key,
            api_secret,
            base_url_http,
            base_url_ws,
            http_timeout_secs: http_timeout_secs.unwrap_or(defaults.http_timeout_secs),
            account_id,
            product: match product {
                Some(value) => GrowwProduct::from_str(&value)
                    .map_err(|e| to_pyvalue_err(format!("invalid product `{value}`: {e}")))?,
                None => defaults.product,
            },
            exchanges: match exchanges {
                Some(values) => parse_list::<GrowwExchange>(Some(values), "exchange")?,
                None => defaults.exchanges,
            },
            segments: match segments {
                Some(values) => parse_list::<GrowwSegment>(Some(values), "segment")?,
                None => defaults.segments,
            },
        })
    }
}
