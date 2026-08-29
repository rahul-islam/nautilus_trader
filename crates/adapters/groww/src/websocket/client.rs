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

//! Feed client for the Groww streaming feed.
//!
//! Connects to the venue's NATS server over WebSocket, authenticates with a locally generated
//! NKEY and a venue-issued JWT, and hands decoded feed messages to the caller over a channel.
//!
//! # Connection lifecycle
//!
//! The NATS server opens every connection, including reconnections, by sending `INFO` with a
//! fresh nonce; the client must answer `CONNECT` carrying the JWT and an ed25519 signature over
//! that nonce. Authentication therefore lives in the frame handler rather than in `connect`:
//! whenever an `INFO` arrives, the handshake runs and every registered subject is re-subscribed,
//! which makes reconnection self-healing without separate recovery logic.
//!
//! Feed JWTs expire daily, so the reconnect sentinel also refreshes the socket token before the
//! next handshake uses it.

use std::{collections::HashMap, sync::Arc, time::Duration};

use nautilus_network::{
    RECONNECTED,
    websocket::{WebSocketClient, WebSocketConfig, channel_message_handler},
};
use tokio::sync::{RwLock, mpsc};
use tokio_tungstenite::tungstenite::Message;

use crate::{
    common::{credential::NKeyPair, urls},
    http::client::GrowwHttpClient,
    websocket::{
        error::{Error, Result},
        messages::{FeedMessage, decode_message},
        nats::{
            ConnectOptions, NatsDecoder, ServerFrame, ping_command, pong_command, sub_command,
            unsub_command,
        },
    },
};

/// An event delivered to the feed consumer.
#[derive(Debug)]
pub enum FeedEvent {
    /// The handshake completed and subscriptions are active.
    Connected,
    /// The transport reconnected; the handshake will run again on the new connection.
    Reconnected,
    /// A decoded feed message together with the subject it arrived on.
    Message {
        /// Subject the message was published to.
        subject: String,
        /// The decoded payload.
        message: FeedMessage,
    },
    /// The server reported an error.
    Error(String),
}

/// Commands accepted by the handler task.
enum HandlerCommand {
    Subscribe(String),
    Unsubscribe(String),
    Disconnect,
}

impl std::fmt::Debug for HandlerCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Subscribe(subject) => write!(f, "Subscribe({subject})"),
            Self::Unsubscribe(subject) => write!(f, "Unsubscribe({subject})"),
            Self::Disconnect => write!(f, "Disconnect"),
        }
    }
}

/// Credentials the handshake signs with, refreshed when the venue rotates the JWT.
#[derive(Debug)]
struct FeedCredentials {
    jwt: String,
    subscription_id: String,
}

/// Feed client for the Groww streaming feed.
#[derive(Debug)]
pub struct GrowwFeedClient {
    http: Arc<GrowwHttpClient>,
    url: String,
    key_pair: NKeyPair,
    credentials: Arc<RwLock<Option<FeedCredentials>>>,
    command_tx: Option<mpsc::UnboundedSender<HandlerCommand>>,
    subscriptions: Arc<RwLock<HashMap<String, u64>>>,
}

impl GrowwFeedClient {
    /// Creates a new [`GrowwFeedClient`] using `http` for token requests.
    #[must_use]
    pub fn new(http: Arc<GrowwHttpClient>, url: Option<&str>) -> Self {
        Self {
            http,
            url: urls::get_ws_url(url),
            key_pair: NKeyPair::generate(),
            credentials: Arc::new(RwLock::new(None)),
            command_tx: None,
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Returns the account subscription id, present after [`Self::connect`].
    pub async fn subscription_id(&self) -> Option<String> {
        self.credentials
            .read()
            .await
            .as_ref()
            .map(|c| c.subscription_id.clone())
    }

    /// Fetches fresh feed credentials for the client's key pair.
    async fn refresh_credentials(&self) -> Result<()> {
        let token = self.http.http_create_socket_token(&self.key_pair).await?;
        *self.credentials.write().await = Some(FeedCredentials {
            jwt: token.token,
            subscription_id: token.subscription_id,
        });
        Ok(())
    }

    /// Connects to the feed and returns the event stream.
    ///
    /// # Errors
    ///
    /// Returns an error if credentials cannot be obtained or the transport fails to connect.
    pub async fn connect(&mut self) -> Result<mpsc::UnboundedReceiver<FeedEvent>> {
        self.refresh_credentials().await?;

        let (message_handler, message_rx) = channel_message_handler();

        let config = WebSocketConfig::builder()
            .url(self.url.clone())
            .connect_timeout_ms(10_000)
            .reconnect_delay_initial_ms(1_000)
            .reconnect_delay_max_ms(30_000)
            .reconnect_backoff_factor(2.0)
            .reconnect_jitter_ms(500)
            // The NATS server pings the client on its own cadence and expects PONG replies; a
            // transport-level heartbeat is unnecessary. Reconnect if the server goes silent for
            // longer than its ping interval allows.
            .idle_timeout_ms(120_000)
            .build()
            .map_err(|e| Error::transport(e.to_string()))?;

        let client = WebSocketClient::builder()
            .config(config)
            .message_handler(message_handler)
            .connect()
            .await
            .map_err(|e| Error::transport(e.to_string()))?;

        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        self.command_tx = Some(command_tx);

        let task = HandlerTask {
            client,
            http: Arc::clone(&self.http),
            key_pair: self.key_pair.clone(),
            credentials: Arc::clone(&self.credentials),
            subscriptions: Arc::clone(&self.subscriptions),
            decoder: NatsDecoder::new(),
            next_sid: 1,
            event_tx,
        };
        tokio::spawn(task.run(message_rx, command_rx));

        Ok(event_rx)
    }

    /// Registers `subject` and subscribes on the live connection.
    ///
    /// # Errors
    ///
    /// Returns an error if the client is not connected.
    pub async fn subscribe(&self, subject: String) -> Result<()> {
        let command_tx = self.command_tx.as_ref().ok_or(Error::NotConnected)?;
        command_tx
            .send(HandlerCommand::Subscribe(subject))
            .map_err(|_| Error::NotConnected)
    }

    /// Unsubscribes `subject` and removes it from the registry.
    ///
    /// # Errors
    ///
    /// Returns an error if the client is not connected.
    pub async fn unsubscribe(&self, subject: String) -> Result<()> {
        let command_tx = self.command_tx.as_ref().ok_or(Error::NotConnected)?;
        command_tx
            .send(HandlerCommand::Unsubscribe(subject))
            .map_err(|_| Error::NotConnected)
    }

    /// Disconnects the feed.
    pub fn disconnect(&mut self) {
        if let Some(command_tx) = self.command_tx.take() {
            let _ = command_tx.send(HandlerCommand::Disconnect);
        }
    }

    /// Requests a disconnect without consuming the command channel.
    pub fn request_disconnect(&self) {
        if let Some(command_tx) = &self.command_tx {
            let _ = command_tx.send(HandlerCommand::Disconnect);
        }
    }

    /// Returns a cheap handle for subscribing from other tasks.
    ///
    /// # Panics
    ///
    /// Panics if called before [`Self::connect`].
    #[must_use]
    pub fn sender(&self) -> FeedHandle {
        FeedHandle {
            command_tx: self.command_tx.clone().expect("feed client not connected"),
        }
    }
}

/// A cloneable handle for subscribing and unsubscribing feed subjects.
#[derive(Debug, Clone)]
pub struct FeedHandle {
    command_tx: mpsc::UnboundedSender<HandlerCommand>,
}

impl FeedHandle {
    /// Subscribes `subject` on the live connection.
    ///
    /// # Errors
    ///
    /// Returns an error if the feed handler has stopped.
    pub async fn subscribe(&self, subject: String) -> Result<()> {
        self.command_tx
            .send(HandlerCommand::Subscribe(subject))
            .map_err(|_| Error::NotConnected)
    }

    /// Unsubscribes `subject` on the live connection.
    ///
    /// # Errors
    ///
    /// Returns an error if the feed handler has stopped.
    pub async fn unsubscribe(&self, subject: String) -> Result<()> {
        self.command_tx
            .send(HandlerCommand::Unsubscribe(subject))
            .map_err(|_| Error::NotConnected)
    }
}

/// Owns the connection state and processes frames and commands.
struct HandlerTask {
    client: WebSocketClient,
    http: Arc<GrowwHttpClient>,
    key_pair: NKeyPair,
    credentials: Arc<RwLock<Option<FeedCredentials>>>,
    subscriptions: Arc<RwLock<HashMap<String, u64>>>,
    decoder: NatsDecoder,
    next_sid: u64,
    event_tx: mpsc::UnboundedSender<FeedEvent>,
}

impl HandlerTask {
    async fn run(
        mut self,
        mut message_rx: mpsc::UnboundedReceiver<Message>,
        mut command_rx: mpsc::UnboundedReceiver<HandlerCommand>,
    ) {
        loop {
            tokio::select! {
                message = message_rx.recv() => {
                    match message {
                        Some(message) => {
                            if !self.handle_transport_message(message).await {
                                break;
                            }
                        }
                        None => {
                            tracing::debug!("Groww feed transport channel closed");
                            break;
                        }
                    }
                }
                command = command_rx.recv() => {
                    match command {
                        Some(HandlerCommand::Subscribe(subject)) => {
                            self.subscribe_subject(subject).await;
                        }
                        Some(HandlerCommand::Unsubscribe(subject)) => {
                            self.unsubscribe_subject(&subject).await;
                        }
                        Some(HandlerCommand::Disconnect) | None => {
                            self.client.disconnect().await;
                            break;
                        }
                    }
                }
            }
        }
        tracing::debug!("Groww feed handler task stopped");
    }

    /// Processes one transport message, returning `false` to stop the task.
    async fn handle_transport_message(&mut self, message: Message) -> bool {
        match message {
            Message::Text(text) if text.as_str() == RECONNECTED => {
                tracing::info!("Groww feed transport reconnected");
                // The JWT may have expired while disconnected; refresh it so the handshake
                // triggered by the incoming INFO signs with a live token. Failure is not fatal
                // here: the old token may still be valid, and a rejected handshake surfaces as
                // a server error that triggers another reconnect cycle.
                if let Err(e) = self.refresh_credentials().await {
                    tracing::warn!("Failed to refresh Groww feed credentials: {e}");
                }
                let _ = self.event_tx.send(FeedEvent::Reconnected);
                true
            }
            Message::Text(text) => self.ingest(text.as_bytes()).await,
            Message::Binary(bytes) => self.ingest(&bytes).await,
            Message::Close(_) => {
                tracing::debug!("Groww feed received close frame");
                true // The transport reconnects on its own.
            }
            _ => true,
        }
    }

    /// Feeds raw bytes to the decoder and processes every complete frame.
    async fn ingest(&mut self, bytes: &[u8]) -> bool {
        self.decoder.extend(bytes);
        loop {
            match self.decoder.next_frame() {
                Ok(Some(frame)) => {
                    if !self.handle_frame(frame).await {
                        return false;
                    }
                }
                Ok(None) => return true,
                Err(e) => {
                    // A corrupt stream cannot be resynchronized reliably; force a reconnect by
                    // reporting and dropping this connection's state.
                    tracing::error!("Groww feed protocol error: {e}");
                    let _ = self.event_tx.send(FeedEvent::Error(e.to_string()));
                    self.decoder = NatsDecoder::new();
                    return true;
                }
            }
        }
    }

    async fn handle_frame(&self, frame: ServerFrame) -> bool {
        match frame {
            ServerFrame::Info(info) => {
                let nonce = info.nonce.clone().unwrap_or_default();
                if let Err(e) = self.handshake(&nonce).await {
                    tracing::error!("Groww feed handshake failed: {e}");
                    let _ = self.event_tx.send(FeedEvent::Error(e.to_string()));
                }
                true
            }
            ServerFrame::Ping => {
                if let Err(e) = self.client.send_text(pong_command(), None).await {
                    tracing::warn!("Failed to send PONG: {e}");
                }
                true
            }
            ServerFrame::Pong | ServerFrame::Ok => true,
            ServerFrame::Err(message) => {
                tracing::error!("Groww feed server error: {message}");
                let auth_failure = message.to_ascii_lowercase().contains("authorization");
                if auth_failure {
                    // The JWT was rejected; refresh so the next handshake signs with a new one.
                    if let Err(e) = self.refresh_credentials().await {
                        tracing::warn!("Failed to refresh Groww feed credentials: {e}");
                    }
                }
                let _ = self.event_tx.send(FeedEvent::Error(message));
                true
            }
            ServerFrame::Msg(message) => {
                match decode_message(&message.subject, &message.payload) {
                    Ok(feed_message) => {
                        let event = FeedEvent::Message {
                            subject: message.subject.clone(),
                            message: feed_message,
                        };
                        if self.event_tx.send(event).is_err() {
                            // The consumer dropped the stream; stop the task.
                            return false;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to decode Groww feed message on `{}`: {e}",
                            message.subject
                        );
                    }
                }
                true
            }
        }
    }

    /// Answers a server `INFO` with `CONNECT` and re-subscribes every registered subject.
    async fn handshake(&self, nonce: &str) -> Result<()> {
        let jwt = self
            .credentials
            .read()
            .await
            .as_ref()
            .map(|c| c.jwt.clone())
            .ok_or(Error::NotConnected)?;

        let sig = self.key_pair.sign(nonce.as_bytes());
        let connect = ConnectOptions::new(jwt, sig).to_command()?;
        self.client
            .send_text(connect, None)
            .await
            .map_err(|e| Error::transport(e.to_string()))?;
        self.client
            .send_text(ping_command(), None)
            .await
            .map_err(|e| Error::transport(e.to_string()))?;

        // Re-subscribe with the sids already assigned so inbound MSG frames keep matching.
        let subscriptions = self.subscriptions.read().await.clone();
        for (subject, sid) in subscriptions {
            self.client
                .send_text(sub_command(&subject, sid), None)
                .await
                .map_err(|e| Error::transport(e.to_string()))?;
        }

        let _ = self.event_tx.send(FeedEvent::Connected);
        Ok(())
    }

    async fn subscribe_subject(&mut self, subject: String) {
        let mut subscriptions = self.subscriptions.write().await;
        if subscriptions.contains_key(&subject) {
            return;
        }
        let sid = self.next_sid;
        self.next_sid += 1;
        subscriptions.insert(subject.clone(), sid);
        drop(subscriptions);

        if let Err(e) = self
            .client
            .send_text(sub_command(&subject, sid), None)
            .await
        {
            tracing::warn!("Failed to subscribe `{subject}`: {e}");
        }
    }

    async fn unsubscribe_subject(&self, subject: &str) {
        let sid = self.subscriptions.write().await.remove(subject);
        if let Some(sid) = sid
            && let Err(e) = self.client.send_text(unsub_command(sid), None).await
        {
            tracing::warn!("Failed to unsubscribe `{subject}`: {e}");
        }
    }

    async fn refresh_credentials(&self) -> Result<()> {
        let token = self.http.http_create_socket_token(&self.key_pair).await?;
        *self.credentials.write().await = Some(FeedCredentials {
            jwt: token.token,
            subscription_id: token.subscription_id,
        });
        Ok(())
    }
}

/// Waits up to `timeout` for the first [`FeedEvent::Connected`] on `events`.
///
/// # Errors
///
/// Returns an error if the stream ends or the timeout elapses first.
pub async fn wait_connected(
    events: &mut mpsc::UnboundedReceiver<FeedEvent>,
    timeout: Duration,
) -> Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let event = tokio::time::timeout_at(deadline, events.recv())
            .await
            .map_err(|_| Error::transport("timed out waiting for feed handshake"))?;
        match event {
            Some(FeedEvent::Connected) => return Ok(()),
            Some(FeedEvent::Error(message)) => return Err(Error::server(message)),
            Some(_) => {}
            None => return Err(Error::transport("feed stream ended during handshake")),
        }
    }
}
