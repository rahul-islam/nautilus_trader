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

//! Python bindings from `pyo3`.

pub mod config;
pub mod factories;

use nautilus_common::factories::{ClientConfig, DataClientFactory, ExecutionClientFactory};
use nautilus_core::python::{to_pyruntime_err, to_pyvalue_err};
use nautilus_system::get_global_pyo3_registry;
use pyo3::prelude::*;

use crate::{
    common::consts::{BSE_VENUE, GROWW, GROWW_CLIENT_ID, MCX_VENUE, NSE_VENUE},
    config::{GrowwDataClientConfig, GrowwExecClientConfig},
    factories::{GrowwDataClientFactory, GrowwExecutionClientFactory},
};

#[expect(clippy::needless_pass_by_value)]
fn extract_groww_data_factory(
    py: Python<'_>,
    factory: Py<PyAny>,
) -> PyResult<Box<dyn DataClientFactory>> {
    match factory.extract::<GrowwDataClientFactory>(py) {
        Ok(f) => Ok(Box::new(f)),
        Err(e) => Err(to_pyvalue_err(format!(
            "Failed to extract GrowwDataClientFactory: {e}"
        ))),
    }
}

#[expect(clippy::needless_pass_by_value)]
fn extract_groww_exec_factory(
    py: Python<'_>,
    factory: Py<PyAny>,
) -> PyResult<Box<dyn ExecutionClientFactory>> {
    match factory.extract::<GrowwExecutionClientFactory>(py) {
        Ok(f) => Ok(Box::new(f)),
        Err(e) => Err(to_pyvalue_err(format!(
            "Failed to extract GrowwExecutionClientFactory: {e}"
        ))),
    }
}

#[expect(clippy::needless_pass_by_value)]
fn extract_groww_data_config(py: Python<'_>, config: Py<PyAny>) -> PyResult<Box<dyn ClientConfig>> {
    match config.extract::<GrowwDataClientConfig>(py) {
        Ok(c) => Ok(Box::new(c)),
        Err(e) => Err(to_pyvalue_err(format!(
            "Failed to extract GrowwDataClientConfig: {e}"
        ))),
    }
}

#[expect(clippy::needless_pass_by_value)]
fn extract_groww_exec_config(py: Python<'_>, config: Py<PyAny>) -> PyResult<Box<dyn ClientConfig>> {
    match config.extract::<GrowwExecClientConfig>(py) {
        Ok(c) => Ok(Box::new(c)),
        Err(e) => Err(to_pyvalue_err(format!(
            "Failed to extract GrowwExecClientConfig: {e}"
        ))),
    }
}

/// Exposed through `nautilus_trader.adapters.groww`.
///
/// # Errors
///
/// Returns an error if any bindings fail to register with the Python module.
#[pymodule]
pub fn groww(_: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add(stringify!(GROWW), GROWW)?;
    m.add(stringify!(GROWW_CLIENT_ID), *GROWW_CLIENT_ID)?;
    m.add(stringify!(NSE_VENUE), *NSE_VENUE)?;
    m.add(stringify!(BSE_VENUE), *BSE_VENUE)?;
    m.add(stringify!(MCX_VENUE), *MCX_VENUE)?;
    m.add_class::<GrowwDataClientConfig>()?;
    m.add_class::<GrowwDataClientFactory>()?;
    m.add_class::<GrowwExecClientConfig>()?;
    m.add_class::<GrowwExecutionClientFactory>()?;

    let registry = get_global_pyo3_registry();

    if let Err(e) =
        registry.register_factory_extractor(GROWW.to_string(), extract_groww_data_factory)
    {
        return Err(to_pyruntime_err(format!(
            "Failed to register Groww data factory extractor: {e}"
        )));
    }
    if let Err(e) =
        registry.register_exec_factory_extractor(GROWW.to_string(), extract_groww_exec_factory)
    {
        return Err(to_pyruntime_err(format!(
            "Failed to register Groww exec factory extractor: {e}"
        )));
    }
    if let Err(e) = registry.register_config_extractor(
        "GrowwDataClientConfig".to_string(),
        extract_groww_data_config,
    ) {
        return Err(to_pyruntime_err(format!(
            "Failed to register Groww data config extractor: {e}"
        )));
    }
    if let Err(e) = registry.register_config_extractor(
        "GrowwExecClientConfig".to_string(),
        extract_groww_exec_config,
    ) {
        return Err(to_pyruntime_err(format!(
            "Failed to register Groww exec config extractor: {e}"
        )));
    }

    Ok(())
}
