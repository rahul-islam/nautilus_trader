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

//! Streaming feed client for Groww.
//!
//! Groww streams market data and order updates over NATS carried on a WebSocket, with protobuf
//! message payloads.

pub mod client;
pub mod error;
pub mod messages;
pub mod nats;
pub mod parse;

/// Types generated from the vendored protobuf schemas in `proto/`.
///
/// See `proto/README.md` for the regeneration procedure.
pub mod proto;
