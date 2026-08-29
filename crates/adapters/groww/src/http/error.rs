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

//! Error types for the Groww HTTP client.

use nautilus_network::http::{HttpClientError, ReqwestError};
use thiserror::Error;

/// Error type for Groww operations.
#[derive(Debug, Error)]
pub enum Error {
    /// Transport layer errors (network, connection issues).
    #[error("transport error: {0}")]
    Transport(String),

    /// JSON serialization/deserialization errors.
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),

    /// Authentication errors (invalid key, expired token).
    #[error("auth error: {0}")]
    Auth(String),

    /// Credentials were required but not supplied.
    #[error("missing credentials: set `{key_var}` and `{secret_var}`, or pass them to the config")]
    MissingCredentials {
        /// Name of the API key environment variable.
        key_var: &'static str,
        /// Name of the API secret environment variable.
        secret_var: &'static str,
    },

    /// Rate limiting errors.
    #[error("rate limited (retry_after_ms={retry_after_ms:?})")]
    RateLimit {
        /// Delay the venue asks the client to wait, when supplied.
        retry_after_ms: Option<u64>,
    },

    /// Bad request errors (client-side invalid payload).
    #[error("bad request: {0}")]
    BadRequest(String),

    /// Venue-specific errors reported in an otherwise well-formed response.
    #[error("venue error: {0}")]
    Venue(String),

    /// Request timeout.
    #[error("timeout")]
    Timeout,

    /// Message decoding/parsing errors.
    #[error("decode error: {0}")]
    Decode(String),

    /// HTTP errors with status code.
    #[error("HTTP error {status}: {message}")]
    Http {
        /// HTTP status code.
        status: u16,
        /// Response body or venue-supplied message.
        message: String,
    },

    /// URL parsing errors.
    #[error("URL parse error: {0}")]
    UrlParse(#[from] url::ParseError),

    /// CSV parsing errors from the instrument master.
    #[error("CSV error: {0}")]
    Csv(#[from] csv::Error),

    /// Standard IO errors.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

impl Error {
    /// Creates a transport error.
    pub fn transport(msg: impl Into<String>) -> Self {
        Self::Transport(msg.into())
    }

    /// Creates an auth error.
    pub fn auth(msg: impl Into<String>) -> Self {
        Self::Auth(msg.into())
    }

    /// Creates a bad request error.
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::BadRequest(msg.into())
    }

    /// Creates a venue error.
    pub fn venue(msg: impl Into<String>) -> Self {
        Self::Venue(msg.into())
    }

    /// Creates a decode error.
    pub fn decode(msg: impl Into<String>) -> Self {
        Self::Decode(msg.into())
    }

    /// Returns whether the error is worth retrying.
    ///
    /// Retrying a rejected order or a malformed request cannot succeed, so only transport-level
    /// faults, timeouts, and explicit rate limits are treated as transient.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        match self {
            Self::Transport(_) | Self::Timeout | Self::RateLimit { .. } => true,
            Self::Http { status, .. } => *status >= 500 || *status == 429,
            _ => false,
        }
    }
}

impl From<HttpClientError> for Error {
    fn from(error: HttpClientError) -> Self {
        match error {
            HttpClientError::TimeoutError(msg) => {
                tracing::trace!("Groww HTTP request timed out: {msg}");
                Self::Timeout
            }
            other => Self::Transport(other.to_string()),
        }
    }
}

impl From<ReqwestError> for Error {
    fn from(error: ReqwestError) -> Self {
        Self::Transport(error.to_string())
    }
}

/// Result type alias for Groww operations.
pub type Result<T> = std::result::Result<T, Error>;
