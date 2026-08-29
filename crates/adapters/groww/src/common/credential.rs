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

//! Credential handling for the Groww API.
//!
//! Groww separates two secrets. The API key and secret mint a bearer access token over REST, and
//! that token in turn authorizes a short-lived JWT for the streaming feed. The feed additionally
//! requires an ed25519 NKEY pair which the client generates locally and never transmits in full:
//! only the public half is sent to the venue.

use std::fmt::{Debug, Display};

use base64::prelude::*;
use ed25519_dalek::{SigningKey, VerifyingKey};
use nautilus_core::env::resolve_env_var_pair;
use rand::RngExt;
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::http::error::{Error, Result};

/// Returns the `(api_key, api_secret)` environment variable names.
#[must_use]
pub fn credential_env_vars() -> (&'static str, &'static str) {
    ("GROWW_API_KEY", "GROWW_API_SECRET")
}

/// Groww API key pair with zeroization on drop.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct GrowwCredential {
    api_key: String,
    api_secret: String,
}

impl GrowwCredential {
    /// Creates a new [`GrowwCredential`] instance.
    #[must_use]
    pub const fn new(api_key: String, api_secret: String) -> Self {
        Self {
            api_key,
            api_secret,
        }
    }

    /// Resolves credentials from provided values or [`credential_env_vars`],
    /// returning `None` when neither yields a complete pair.
    #[must_use]
    pub fn resolve(api_key: Option<String>, api_secret: Option<String>) -> Option<Self> {
        let (key_var, secret_var) = credential_env_vars();
        let (api_key, api_secret) = resolve_env_var_pair(api_key, api_secret, key_var, secret_var)?;
        Some(Self::new(api_key, api_secret))
    }

    /// Returns the API key.
    #[must_use]
    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    /// Returns the SHA-256 checksum authorizing an access-token request at `timestamp`.
    ///
    /// The venue defines the checksum as `sha256(api_secret || timestamp)`, with `timestamp` the
    /// decimal UNIX epoch in seconds rendered as ASCII. Both halves are concatenated without a
    /// separator, so the exact rendering of `timestamp` is part of the contract.
    #[must_use]
    pub fn checksum(&self, timestamp: i64) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.api_secret.as_bytes());
        hasher.update(timestamp.to_string().as_bytes());
        hex_encode(&hasher.finalize())
    }
}

impl Debug for GrowwCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(stringify!(GrowwCredential))
            .field("api_key", &"<redacted>")
            .field("api_secret", &"<redacted>")
            .finish()
    }
}

impl Display for GrowwCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}(<redacted>)", stringify!(GrowwCredential))
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// NKEY prefix byte for a user seed (`S`), shifted into its base32 position.
const PREFIX_BYTE_SEED: u8 = 18 << 3;
/// NKEY prefix byte for a user public key (`U`).
const PREFIX_BYTE_USER: u8 = 20 << 3;

/// An ed25519 key pair encoded in the NATS NKEY format.
///
/// The feed authenticates with an NKEY the client generates for each connection. The seed never
/// leaves the process; only [`Self::public_key`] is sent to the venue in exchange for a feed JWT.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct NKeyPair {
    seed: [u8; 32],
}

impl NKeyPair {
    /// Generates a new random NKEY pair.
    #[must_use]
    pub fn generate() -> Self {
        let mut seed = [0u8; 32];
        rand::rng().fill(&mut seed[..]);
        Self { seed }
    }

    /// Creates a pair from raw ed25519 seed bytes.
    #[must_use]
    pub const fn from_seed_bytes(seed: [u8; 32]) -> Self {
        Self { seed }
    }

    /// Returns the NKEY-encoded seed, the form a NATS client authenticates with.
    #[must_use]
    pub fn encoded_seed(&self) -> String {
        // A seed encodes two prefixes: the seed marker in the top 5 bits of the first byte, and
        // the key role in the low 3 bits of the first byte plus the top 5 of the second.
        let mut raw = Vec::with_capacity(34);
        raw.push(PREFIX_BYTE_SEED | (PREFIX_BYTE_USER >> 5));
        raw.push((PREFIX_BYTE_USER & 31) << 3);
        raw.extend_from_slice(&self.seed);
        encode_nkey(&raw)
    }

    /// Returns the NKEY-encoded public key to send to the venue.
    #[must_use]
    pub fn public_key(&self) -> String {
        let verifying: VerifyingKey = SigningKey::from_bytes(&self.seed).verifying_key();
        let mut raw = Vec::with_capacity(33);
        raw.push(PREFIX_BYTE_USER);
        raw.extend_from_slice(verifying.as_bytes());
        encode_nkey(&raw)
    }

    /// Signs the server `nonce` with the seed, returning the signature for the CONNECT command.
    ///
    /// Encoded as standard padded base64 rather than base64url. The NATS reference clients emit
    /// the padded form, and matching them keeps the handshake identical to the one the venue is
    /// known to accept.
    #[must_use]
    pub fn sign(&self, nonce: &[u8]) -> String {
        use ed25519_dalek::Signer;
        let signature = SigningKey::from_bytes(&self.seed).sign(nonce);
        BASE64_STANDARD.encode(signature.to_bytes())
    }
}

impl Debug for NKeyPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(stringify!(NKeyPair))
            .field("public_key", &self.public_key())
            .field("seed", &"<redacted>")
            .finish()
    }
}

/// Appends the NKEY CRC-16/XMODEM checksum and encodes the result as unpadded base32.
fn encode_nkey(raw: &[u8]) -> String {
    let mut buf = raw.to_vec();
    buf.extend_from_slice(&crc16(raw).to_le_bytes());
    base32_encode(&buf)
}

/// CRC-16/XMODEM over `data`, the checksum NKEY uses to detect transcription errors.
fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for byte in data {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// RFC 4648 base32 without padding, the alphabet NKEY uses.
fn base32_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

    let mut out = String::with_capacity(data.len().div_ceil(5) * 8);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for byte in data {
        buffer = (buffer << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let index = ((buffer >> bits) & 31) as usize;
            out.push(ALPHABET[index] as char);
        }
    }
    if bits > 0 {
        let index = ((buffer << (5 - bits)) & 31) as usize;
        out.push(ALPHABET[index] as char);
    }
    out
}

/// Returns an error when credentials are required but absent.
pub fn require_credential(credential: Option<&GrowwCredential>) -> Result<&GrowwCredential> {
    credential.ok_or_else(|| Error::MissingCredentials {
        key_var: credential_env_vars().0,
        secret_var: credential_env_vars().1,
    })
}
