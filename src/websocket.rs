/// WebSocket Support - Real-time blockchain event streaming
/// 
/// Provides:
/// - /ws/blocks - Subscribe to new block events
/// - /ws/transactions - Subscribe to new transaction events
/// - /ws/mempool - Subscribe to mempool updates
/// 
/// Uses tokio broadcast channels for pub/sub pattern

use axum::{
    extract::ws::{WebSocketUpgrade, WebSocket, Message},
    response::Response,
    Extension,
};
use futures::{stream::StreamExt, SinkExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::broadcast;

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
    pub fn broadcast_transaction(&self, txid: String, block_height: Option<i32>, value: Option<f64>) {
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
    Extension(broadcaster): Extension<Arc<EventBroadcaster>>,
) -> Response {
    ws.on_upgrade(|socket| handle_block_socket(socket, broadcaster))
}

/// WebSocket handler for transaction events
pub async fn ws_transactions_handler(
    ws: WebSocketUpgrade,
    Extension(broadcaster): Extension<Arc<EventBroadcaster>>,
) -> Response {
    ws.on_upgrade(|socket| handle_transaction_socket(socket, broadcaster))
}

/// WebSocket handler for mempool events
pub async fn ws_mempool_handler(
    ws: WebSocketUpgrade,
    Extension(broadcaster): Extension<Arc<EventBroadcaster>>,
) -> Response {
    ws.on_upgrade(|socket| handle_mempool_socket(socket, broadcaster))
}

/// Handle WebSocket connection for block events
async fn handle_block_socket(socket: WebSocket, broadcaster: Arc<EventBroadcaster>) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = broadcaster.block_tx.subscribe();

    // Send initial connection message
    let welcome = serde_json::json!({
        "type": "connected",
        "channel": "blocks",
        "message": "Subscribed to new block events"
    });
    
    if sender.send(Message::Text(welcome.to_string().into())).await.is_err() {
        return;
    }

    // Spawn task to receive messages from client (for potential ping/pong)
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if matches!(msg, Message::Close(_)) {
                break;
            }
        }
    });

    // Main loop - forward broadcast events to client
    let mut send_task = tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            if let Ok(json) = serde_json::to_string(&event) {
                if sender.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
        }
    });

    // Wait for either task to finish
    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    }
}

/// Handle WebSocket connection for transaction events
async fn handle_transaction_socket(socket: WebSocket, broadcaster: Arc<EventBroadcaster>) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = broadcaster.transaction_tx.subscribe();

    // Send initial connection message
    let welcome = serde_json::json!({
        "type": "connected",
        "channel": "transactions",
        "message": "Subscribed to new transaction events"
    });
    
    if sender.send(Message::Text(welcome.to_string().into())).await.is_err() {
        return;
    }

    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if matches!(msg, Message::Close(_)) {
                break;
            }
        }
    });

    let mut send_task = tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            if let Ok(json) = serde_json::to_string(&event) {
                if sender.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
        }
    });

    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    }
}

/// Handle WebSocket connection for mempool events
async fn handle_mempool_socket(socket: WebSocket, broadcaster: Arc<EventBroadcaster>) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = broadcaster.mempool_tx.subscribe();

    // Send initial connection message
    let welcome = serde_json::json!({
        "type": "connected",
        "channel": "mempool",
        "message": "Subscribed to mempool events"
    });
    
    if sender.send(Message::Text(welcome.to_string().into())).await.is_err() {
        return;
    }

    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if matches!(msg, Message::Close(_)) {
                break;
            }
        }
    });

    let mut send_task = tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            if let Ok(json) = serde_json::to_string(&event) {
                if sender.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
        }
    });

    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    }
}
