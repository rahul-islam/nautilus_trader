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

//! Error types for the Groww streaming feed.

use thiserror::Error;

/// Error type for streaming feed operations.
#[derive(Debug, Error)]
pub enum Error {
    /// The peer violated the NATS client protocol.
    #[error("protocol error: {0}")]
    Protocol(String),

    /// The server rejected the connection or a command.
    #[error("server error: {0}")]
    Server(String),

    /// Authentication failed.
    #[error("auth error: {0}")]
    Auth(String),

    /// A protobuf payload could not be decoded.
    #[error("decode error: {0}")]
    Decode(String),

    /// The transport failed.
    #[error("transport error: {0}")]
    Transport(String),

    /// The client is not connected.
    #[error("not connected")]
    NotConnected,

    /// An HTTP request made on behalf of the feed failed.
    #[error(transparent)]
    Http(#[from] crate::http::error::Error),
}

impl Error {
    /// Creates a protocol error.
    pub fn protocol(msg: impl Into<String>) -> Self {
        Self::Protocol(msg.into())
    }

    /// Creates a server error.
    pub fn server(msg: impl Into<String>) -> Self {
        Self::Server(msg.into())
    }

    /// Creates a decode error.
    pub fn decode(msg: impl Into<String>) -> Self {
        Self::Decode(msg.into())
    }

    /// Creates a transport error.
    pub fn transport(msg: impl Into<String>) -> Self {
        Self::Transport(msg.into())
    }
}

/// Result type alias for streaming feed operations.
pub type Result<T> = std::result::Result<T, Error>;
