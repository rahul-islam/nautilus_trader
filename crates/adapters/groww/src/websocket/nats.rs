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

//! Minimal NATS client protocol, sufficient for the Groww feed.
//!
//! Groww streams over a NATS server reached by WebSocket rather than a bespoke protocol. Only the
//! subscriber half of the protocol is needed: connect and authenticate, subscribe and unsubscribe,
//! answer server pings, and decode inbound messages. Publishing, request/reply, JetStream, and
//! cluster discovery are deliberately absent.
//!
//! The protocol is line-oriented ASCII with a binary payload following the `MSG` header. Frames
//! are carried as WebSocket messages, so the transport already preserves message boundaries; a
//! single WebSocket frame may still carry several protocol frames, which [`NatsDecoder`] handles.

use serde::{Deserialize, Serialize};

use crate::websocket::error::{Error, Result};

/// Protocol line terminator.
pub const CRLF: &str = "\r\n";

/// Server `INFO` payload, reduced to the fields this client uses.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ServerInfo {
    /// Server identifier.
    #[serde(default)]
    pub server_id: String,
    /// Server version.
    #[serde(default)]
    pub version: String,
    /// Whether the server requires authentication.
    #[serde(default)]
    pub auth_required: bool,
    /// Single-use nonce the client signs to prove possession of its NKEY seed.
    #[serde(default)]
    pub nonce: Option<String>,
    /// Largest payload the server accepts.
    #[serde(default)]
    pub max_payload: Option<u64>,
}

/// Client `CONNECT` payload.
///
/// Groww authenticates with a JWT issued by the socket-token endpoint together with a signature
/// over the server nonce, so `jwt` and `sig` are the only credential fields sent.
#[derive(Debug, Clone, Serialize)]
pub struct ConnectOptions {
    /// Whether the server should echo messages published by this connection.
    pub echo: bool,
    /// Whether the client understands message headers.
    pub headers: bool,
    /// JWT authorizing the connection.
    pub jwt: String,
    /// Base64 signature over the server nonce.
    pub sig: String,
    /// Client language, reported for server-side diagnostics.
    pub lang: &'static str,
    /// Whether the client understands no-responder replies.
    pub no_responders: bool,
    /// Whether the server should apply strict protocol checks.
    pub pedantic: bool,
    /// Protocol revision.
    pub protocol: u8,
    /// Whether the server should acknowledge every command with `+OK`.
    ///
    /// Left off so the server does not send an acknowledgement per subscription.
    pub verbose: bool,
    /// Client version, reported for server-side diagnostics.
    pub version: &'static str,
}

impl ConnectOptions {
    /// Creates connect options carrying `jwt` and `sig`.
    #[must_use]
    pub fn new(jwt: String, sig: String) -> Self {
        Self {
            echo: true,
            headers: true,
            jwt,
            sig,
            lang: "rust",
            no_responders: true,
            pedantic: false,
            protocol: 1,
            verbose: false,
            version: env!("CARGO_PKG_VERSION"),
        }
    }

    /// Renders the `CONNECT` command line.
    ///
    /// # Errors
    ///
    /// Returns an error if the options cannot be serialized.
    pub fn to_command(&self) -> Result<String> {
        let json = serde_json::to_string(self)
            .map_err(|e| Error::protocol(format!("failed to encode CONNECT: {e}")))?;
        Ok(format!("CONNECT {json}{CRLF}"))
    }
}

/// Renders a `SUB` command line for `subject` with subscription id `sid`.
#[must_use]
pub fn sub_command(subject: &str, sid: u64) -> String {
    format!("SUB {subject} {sid}{CRLF}")
}

/// Renders an `UNSUB` command line for subscription id `sid`.
#[must_use]
pub fn unsub_command(sid: u64) -> String {
    format!("UNSUB {sid}{CRLF}")
}

/// Renders the `PING` command line.
#[must_use]
pub fn ping_command() -> String {
    format!("PING{CRLF}")
}

/// Renders the `PONG` command line.
#[must_use]
pub fn pong_command() -> String {
    format!("PONG{CRLF}")
}

/// A message delivered to a subscription.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NatsMessage {
    /// Subject the message was published to.
    pub subject: String,
    /// Subscription id the message was delivered on.
    pub sid: u64,
    /// Optional reply subject.
    pub reply_to: Option<String>,
    /// Message payload.
    pub payload: Vec<u8>,
}

/// A decoded server frame.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerFrame {
    /// Server information, sent on connect and on cluster changes.
    Info(Box<ServerInfo>),
    /// A message delivered to a subscription.
    Msg(NatsMessage),
    /// Server keepalive; the client must answer with `PONG`.
    Ping,
    /// Reply to a client `PING`.
    Pong,
    /// Command acknowledgement, sent only in verbose mode.
    Ok,
    /// Protocol or authorization error reported by the server.
    Err(String),
}

/// Incremental decoder for the NATS client protocol.
///
/// A `MSG` header announces a payload length, and the payload can be split across WebSocket frames,
/// so decoding is buffered rather than per-frame.
#[derive(Debug, Default)]
pub struct NatsDecoder {
    buffer: Vec<u8>,
}

/// Largest protocol control line accepted before the stream is treated as corrupt.
///
/// Control lines are short by construction. Without a bound, a stream that never sends a line
/// terminator would grow the buffer without limit.
const MAX_CONTROL_LINE: usize = 4096;

impl NatsDecoder {
    /// Creates a new empty decoder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends raw bytes received from the transport.
    pub fn extend(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    /// Returns the number of buffered bytes not yet decoded.
    #[must_use]
    pub fn buffered(&self) -> usize {
        self.buffer.len()
    }

    /// Decodes the next complete frame, returning `None` when more bytes are needed.
    ///
    /// # Errors
    ///
    /// Returns an error if the stream violates the protocol.
    pub fn next_frame(&mut self) -> Result<Option<ServerFrame>> {
        let Some(line_end) = find_crlf(&self.buffer) else {
            if self.buffer.len() > MAX_CONTROL_LINE {
                return Err(Error::protocol(format!(
                    "control line exceeded {MAX_CONTROL_LINE} bytes without a terminator"
                )));
            }
            return Ok(None);
        };

        let line = std::str::from_utf8(&self.buffer[..line_end])
            .map_err(|e| Error::protocol(format!("control line is not valid UTF-8: {e}")))?
            .to_string();

        let (verb, rest) = match line.split_once(' ') {
            Some((verb, rest)) => (verb, rest.trim()),
            None => (line.as_str(), ""),
        };

        // Verbs are case-insensitive in the protocol.
        match verb.to_ascii_uppercase().as_str() {
            "MSG" => self.decode_msg(line_end, rest),
            "PING" => {
                self.consume(line_end + CRLF.len());
                Ok(Some(ServerFrame::Ping))
            }
            "PONG" => {
                self.consume(line_end + CRLF.len());
                Ok(Some(ServerFrame::Pong))
            }
            "+OK" => {
                self.consume(line_end + CRLF.len());
                Ok(Some(ServerFrame::Ok))
            }
            "-ERR" => {
                self.consume(line_end + CRLF.len());
                Ok(Some(ServerFrame::Err(
                    rest.trim_matches('\'').trim().to_string(),
                )))
            }
            "INFO" => {
                let info: ServerInfo = serde_json::from_str(rest)
                    .map_err(|e| Error::protocol(format!("failed to decode INFO: {e}")))?;
                self.consume(line_end + CRLF.len());
                Ok(Some(ServerFrame::Info(Box::new(info))))
            }
            other => Err(Error::protocol(format!("unknown protocol verb `{other}`"))),
        }
    }

    /// Decodes a `MSG` header and its payload.
    ///
    /// The header is `MSG <subject> <sid> [reply-to] <#bytes>`, with the payload following the
    /// terminator and itself terminated by another `CRLF`.
    fn decode_msg(&mut self, line_end: usize, rest: &str) -> Result<Option<ServerFrame>> {
        let parts: Vec<&str> = rest.split_whitespace().collect();
        let (subject, sid, reply_to, len) = match parts.as_slice() {
            [subject, sid, len] => (*subject, *sid, None, *len),
            [subject, sid, reply_to, len] => (*subject, *sid, Some((*reply_to).to_string()), *len),
            _ => {
                return Err(Error::protocol(format!("malformed MSG header `{rest}`")));
            }
        };

        let sid: u64 = sid
            .parse()
            .map_err(|_| Error::protocol(format!("invalid subscription id `{sid}`")))?;
        let payload_len: usize = len
            .parse()
            .map_err(|_| Error::protocol(format!("invalid payload length `{len}`")))?;

        let payload_start = line_end + CRLF.len();
        let payload_end = payload_start + payload_len;
        let frame_end = payload_end + CRLF.len();

        if self.buffer.len() < frame_end {
            return Ok(None); // Await the rest of the payload.
        }

        let payload = self.buffer[payload_start..payload_end].to_vec();
        let message = NatsMessage {
            subject: subject.to_string(),
            sid,
            reply_to,
            payload,
        };
        self.consume(frame_end);
        Ok(Some(ServerFrame::Msg(message)))
    }

    fn consume(&mut self, count: usize) {
        self.buffer.drain(..count.min(self.buffer.len()));
    }
}

/// Returns the index of the first `CRLF` in `buffer`.
fn find_crlf(buffer: &[u8]) -> Option<usize> {
    buffer.windows(2).position(|w| w == b"\r\n")
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::common::credential::NKeyPair;

    #[rstest]
    fn test_signature_matches_reference_vector() {
        // Vector produced by the NATS reference implementation (`nkeys` and `PyNaCl`) for a fixed
        // seed, pinning both the ed25519 signature and its padded base64 encoding.
        let seed: [u8; 32] = std::array::from_fn(|i| i as u8);
        let pair = NKeyPair::from_seed_bytes(seed);

        assert_eq!(
            pair.sign(b"nonce-abc-123"),
            "3nbNJvYI6aoCC2mSwJbTIDX6+XiydPg+5p5hGs1W+l+vKJ2gisORBZkfG3NyOt2MnRkUmFJjhekLNNDo9xErBg=="
        );
        assert_eq!(
            pair.public_key(),
            "UAB2CB576PHBBPQ5ODORRZ2LYCMWPZGWGCN2KDK7DXOIMZASKUY3RLKK"
        );
    }

    #[rstest]
    fn test_decode_info_ping_and_err() {
        let mut decoder = NatsDecoder::new();
        decoder.extend(b"INFO {\"server_id\":\"abc\",\"auth_required\":true,\"nonce\":\"xyz\"}\r\nPING\r\n-ERR 'Authorization Violation'\r\n");

        let ServerFrame::Info(info) = decoder.next_frame().unwrap().unwrap() else {
            panic!("expected INFO");
        };
        assert_eq!(info.server_id, "abc");
        assert!(info.auth_required);
        assert_eq!(info.nonce.as_deref(), Some("xyz"));

        assert_eq!(decoder.next_frame().unwrap(), Some(ServerFrame::Ping));
        assert_eq!(
            decoder.next_frame().unwrap(),
            Some(ServerFrame::Err("Authorization Violation".to_string()))
        );
        assert_eq!(decoder.next_frame().unwrap(), None);
    }

    #[rstest]
    fn test_decode_msg_with_binary_payload() {
        let mut decoder = NatsDecoder::new();
        // A protobuf payload can contain CRLF, so the decoder must honour the declared length
        // rather than scanning for a terminator.
        let payload: &[u8] = &[0x08, 0x0d, 0x0a, 0xff, 0x00];
        let mut frame = format!("MSG /ld/eq/nse/price.2885 7 {}\r\n", payload.len()).into_bytes();
        frame.extend_from_slice(payload);
        frame.extend_from_slice(b"\r\n");
        decoder.extend(&frame);

        let ServerFrame::Msg(msg) = decoder.next_frame().unwrap().unwrap() else {
            panic!("expected MSG");
        };
        assert_eq!(msg.subject, "/ld/eq/nse/price.2885");
        assert_eq!(msg.sid, 7);
        assert_eq!(msg.reply_to, None);
        assert_eq!(msg.payload, payload);
        assert_eq!(decoder.buffered(), 0);
    }

    #[rstest]
    fn test_decode_msg_split_across_frames() {
        let mut decoder = NatsDecoder::new();
        decoder.extend(b"MSG subject.a 1 5\r\nab");
        assert_eq!(decoder.next_frame().unwrap(), None);

        decoder.extend(b"cde\r\n");
        let ServerFrame::Msg(msg) = decoder.next_frame().unwrap().unwrap() else {
            panic!("expected MSG");
        };
        assert_eq!(msg.payload, b"abcde");
    }

    #[rstest]
    fn test_decode_msg_with_reply_subject() {
        let mut decoder = NatsDecoder::new();
        decoder.extend(b"MSG subject.a 2 reply.b 2\r\nhi\r\n");

        let ServerFrame::Msg(msg) = decoder.next_frame().unwrap().unwrap() else {
            panic!("expected MSG");
        };
        assert_eq!(msg.reply_to.as_deref(), Some("reply.b"));
        assert_eq!(msg.payload, b"hi");
    }

    #[rstest]
    fn test_commands_are_crlf_terminated() {
        assert_eq!(
            sub_command("/ld/eq/nse/price.2885", 3),
            "SUB /ld/eq/nse/price.2885 3\r\n"
        );
        assert_eq!(unsub_command(3), "UNSUB 3\r\n");
        assert_eq!(ping_command(), "PING\r\n");
        assert_eq!(pong_command(), "PONG\r\n");
    }

    #[rstest]
    fn test_connect_command_carries_credentials() {
        let options = ConnectOptions::new("jwt-value".to_string(), "sig-value".to_string());
        let command = options.to_command().unwrap();

        assert!(command.starts_with("CONNECT {"));
        assert!(command.ends_with("\r\n"));
        assert!(command.contains("\"jwt\":\"jwt-value\""));
        assert!(command.contains("\"sig\":\"sig-value\""));
        // Verbose acknowledgements would add a server round trip per subscription.
        assert!(command.contains("\"verbose\":false"));
    }

    #[rstest]
    fn test_unterminated_control_line_is_bounded() {
        let mut decoder = NatsDecoder::new();
        decoder.extend(&vec![b'x'; MAX_CONTROL_LINE + 1]);
        assert!(decoder.next_frame().is_err());
    }
}
