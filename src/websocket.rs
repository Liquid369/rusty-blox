/// WebSocket Support - Real-time blockchain event streaming
///
/// Provides:
/// - /ws/blocks - Subscribe to new block events
/// - /ws/transactions - Subscribe to new transaction events
/// - /ws/mempool - Subscribe to mempool updates
///
/// Uses tokio broadcast channels for pub/sub pattern
use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension,
};
use futures::{stream::StreamExt, SinkExt};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};
use tokio::sync::{broadcast, OwnedSemaphorePermit, Semaphore};

/// P2-2 hardening — global cap on concurrent WebSocket connections across all
/// channels (blocks/transactions/mempool). Sockets carry only public chain
/// data, so this is resource-exhaustion hardening, not confidentiality. New
/// handshakes past the cap are rejected with 503 instead of being accepted and
/// starving the process of file descriptors / memory.
const MAX_WS_CONNECTIONS: usize = 4096;

/// Idle timeout: if no traffic (event to send or client frame, incl. pongs)
/// flows for this long, the socket is closed. Combined with the periodic ping
/// below this reaps half-open connections that never send a TCP FIN.
const WS_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Interval at which the server sends a ping to keep the connection live and
/// to detect dead peers (a missing pong eventually trips the idle timeout).
const WS_PING_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

fn ws_semaphore() -> &'static Arc<Semaphore> {
    static SEM: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SEM.get_or_init(|| Arc::new(Semaphore::new(MAX_WS_CONNECTIONS)))
}

/// Allowlist of permitted WebSocket `Origin` values.
///
/// Read once from config key `server.ws_allowed_origins` (comma-separated list
/// of full origins, e.g. `https://explorer.example,http://localhost:3005`).
/// When the key is absent or empty we fall back to a permissive-but-bounded
/// policy (see [`origin_allowed`]) so local dev is never hard-broken.
fn ws_allowed_origins() -> &'static Vec<String> {
    static ORIGINS: OnceLock<Vec<String>> = OnceLock::new();
    ORIGINS.get_or_init(|| {
        crate::config::get_global_config()
            .get_string("server.ws_allowed_origins")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().trim_end_matches('/').to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect()
    })
}

/// Decide whether a handshake's `Origin` header is acceptable.
///
/// - No `Origin` header → allow. Non-browser clients (curl, native apps,
///   server-to-server) legitimately omit it, and Origin is browser-enforced
///   anti-CSWSH, not an auth boundary for this public read-only data.
/// - Explicit allowlist configured → the origin must match it exactly.
/// - No allowlist configured → permissive-but-bounded default: allow loopback
///   origins (localhost / 127.0.0.1 / [::1], any port) so local dev works, and
///   allow when the request's `Host` matches the origin's host (same-origin).
fn origin_allowed(headers: &HeaderMap) -> bool {
    let origin = match headers
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok())
    {
        Some(o) => o.trim().trim_end_matches('/').to_ascii_lowercase(),
        None => return true, // no Origin (non-browser client) — allow
    };

    let allowed = ws_allowed_origins();
    if !allowed.is_empty() {
        return allowed.iter().any(|a| a == &origin);
    }

    // Permissive-but-bounded default (no allowlist configured).
    // Extract the host[:port] portion after the scheme.
    let host_part = origin.split("://").nth(1).unwrap_or(&origin);
    let host_only = host_part.split(':').next().unwrap_or(host_part);

    // Always allow loopback for local development.
    if host_only == "localhost"
        || host_only == "127.0.0.1"
        || host_only == "[::1]"
        || host_only == "::1"
    {
        return true;
    }

    // Otherwise require same-origin: the Origin host must equal the request Host.
    if let Some(req_host) = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
    {
        let req_host = req_host.trim().to_ascii_lowercase();
        return host_part == req_host
            || host_only == req_host.split(':').next().unwrap_or(&req_host);
    }

    false
}

/// Shared handshake gate: enforce the Origin allowlist and the global
/// connection cap before upgrading. On success returns the acquired permit,
/// which is moved into the socket handler so it is released on disconnect.
fn ws_guard(headers: &HeaderMap) -> Result<OwnedSemaphorePermit, Response> {
    if !origin_allowed(headers) {
        return Err((StatusCode::FORBIDDEN, "origin not allowed").into_response());
    }
    ws_semaphore().clone().try_acquire_owned().map_err(|_| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "websocket connection limit reached",
        )
            .into_response()
    })
}

/// Event types that can be broadcast to WebSocket clients
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum BlockchainEvent {
    NewBlock {
        height: i32,
        hash: String,
        timestamp: u64,
        tx_count: usize,
    },
    NewTransaction {
        txid: String,
        block_height: Option<i32>,
        value: Option<f64>,
    },
    MempoolUpdate {
        txid: String,
        action: String, // "added" or "removed"
    },
    ChainReorg {
        old_height: i32,
        new_height: i32,
        common_ancestor: String,
    },
}

/// Global broadcast channels for different event types
pub struct EventBroadcaster {
    pub block_tx: broadcast::Sender<BlockchainEvent>,
    pub transaction_tx: broadcast::Sender<BlockchainEvent>,
    pub mempool_tx: broadcast::Sender<BlockchainEvent>,
}

impl Default for EventBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBroadcaster {
    pub fn new() -> Self {
        // Create broadcast channels with capacity of 1000 events
        let (block_tx, _) = broadcast::channel(1000);
        let (transaction_tx, _) = broadcast::channel(1000);
        let (mempool_tx, _) = broadcast::channel(1000);

        Self {
            block_tx,
            transaction_tx,
            mempool_tx,
        }
    }

    /// Broadcast a new block event
    pub fn broadcast_block(&self, height: i32, hash: String, timestamp: u64, tx_count: usize) {
        let event = BlockchainEvent::NewBlock {
            height,
            hash,
            timestamp,
            tx_count,
        };
        let _ = self.block_tx.send(event);
    }

    /// Broadcast a new transaction event
    pub fn broadcast_transaction(
        &self,
        txid: String,
        block_height: Option<i32>,
        value: Option<f64>,
    ) {
        let event = BlockchainEvent::NewTransaction {
            txid,
            block_height,
            value,
        };
        let _ = self.transaction_tx.send(event);
    }

    /// Broadcast a mempool update event
    pub fn broadcast_mempool_update(&self, txid: String, action: String) {
        let event = BlockchainEvent::MempoolUpdate { txid, action };
        let _ = self.mempool_tx.send(event);
    }

    /// Broadcast a chain reorg event
    pub fn broadcast_reorg(&self, old_height: i32, new_height: i32, common_ancestor: String) {
        let event = BlockchainEvent::ChainReorg {
            old_height,
            new_height,
            common_ancestor,
        };
        let _ = self.block_tx.send(event);
    }
}

/// WebSocket handler for block events
pub async fn ws_blocks_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    Extension(broadcaster): Extension<Arc<EventBroadcaster>>,
) -> Response {
    let permit = match ws_guard(&headers) {
        Ok(p) => p,
        Err(resp) => return resp,
    };
    ws.on_upgrade(move |socket| handle_block_socket(socket, broadcaster, permit))
}

/// WebSocket handler for transaction events
pub async fn ws_transactions_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    Extension(broadcaster): Extension<Arc<EventBroadcaster>>,
) -> Response {
    let permit = match ws_guard(&headers) {
        Ok(p) => p,
        Err(resp) => return resp,
    };
    ws.on_upgrade(move |socket| handle_transaction_socket(socket, broadcaster, permit))
}

/// WebSocket handler for mempool events
pub async fn ws_mempool_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    Extension(broadcaster): Extension<Arc<EventBroadcaster>>,
) -> Response {
    let permit = match ws_guard(&headers) {
        Ok(p) => p,
        Err(resp) => return resp,
    };
    ws.on_upgrade(move |socket| handle_mempool_socket(socket, broadcaster, permit))
}

/// Handle WebSocket connection for block events.
async fn handle_block_socket(
    socket: WebSocket,
    broadcaster: Arc<EventBroadcaster>,
    permit: OwnedSemaphorePermit,
) {
    let rx = broadcaster.block_tx.subscribe();
    serve_socket(
        socket,
        rx,
        "blocks",
        "Subscribed to new block events",
        permit,
    )
    .await;
}

/// Handle WebSocket connection for transaction events.
async fn handle_transaction_socket(
    socket: WebSocket,
    broadcaster: Arc<EventBroadcaster>,
    permit: OwnedSemaphorePermit,
) {
    let rx = broadcaster.transaction_tx.subscribe();
    serve_socket(
        socket,
        rx,
        "transactions",
        "Subscribed to new transaction events",
        permit,
    )
    .await;
}

/// Handle WebSocket connection for mempool events.
async fn handle_mempool_socket(
    socket: WebSocket,
    broadcaster: Arc<EventBroadcaster>,
    permit: OwnedSemaphorePermit,
) {
    let rx = broadcaster.mempool_tx.subscribe();
    serve_socket(
        socket,
        rx,
        "mempool",
        "Subscribed to mempool events",
        permit,
    )
    .await;
}

/// Shared per-connection event loop for every channel.
///
/// Holds `permit` for the connection's lifetime (released on return, freeing a
/// global WS slot). A single `select!` multiplexes: broadcast events out,
/// client frames in (close detection), a periodic server ping, and an idle
/// timeout that reaps half-open peers.
async fn serve_socket(
    socket: WebSocket,
    mut rx: broadcast::Receiver<BlockchainEvent>,
    channel: &str,
    welcome_message: &str,
    _permit: OwnedSemaphorePermit,
) {
    let (mut sender, mut receiver) = socket.split();

    // Send initial connection message.
    let welcome = serde_json::json!({
        "type": "connected",
        "channel": channel,
        "message": welcome_message,
    });
    if sender
        .send(Message::Text(welcome.to_string().into()))
        .await
        .is_err()
    {
        return;
    }

    let mut ping = tokio::time::interval(WS_PING_INTERVAL);
    // The first tick fires immediately; skip sending a ping at t=0.
    ping.tick().await;

    // Idle deadline reset on every inbound frame (data, ping or pong). A peer
    // that has gone silent — including one whose TCP is half-open and never
    // answers our pings — trips this and the socket is closed.
    let idle = tokio::time::sleep(WS_IDLE_TIMEOUT);
    tokio::pin!(idle);

    loop {
        tokio::select! {
            // Outbound: broadcast events. On lag, keep the socket alive.
            event = rx.recv() => {
                match event {
                    Ok(ev) => {
                        if let Ok(json) = serde_json::to_string(&ev) {
                            if sender.send(Message::Text(json.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            // Inbound: client frames. Used only for close/keepalive detection;
            // channels are read-only so payloads are ignored. Any inbound frame
            // counts as liveness and refreshes the idle deadline.
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {
                        idle.as_mut().reset(tokio::time::Instant::now() + WS_IDLE_TIMEOUT);
                        continue;
                    }
                    Some(Err(_)) => break,
                }
            }
            // Keepalive ping (prompts a pong, which refreshes the idle deadline).
            _ = ping.tick() => {
                if sender.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
            // Idle timeout: no inbound frame for WS_IDLE_TIMEOUT.
            _ = idle.as_mut() => {
                break;
            }
        }
    }
    // `_permit` drops here, releasing the global connection slot.
}
