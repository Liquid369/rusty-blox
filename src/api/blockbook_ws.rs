// Blockbook websocket protocol at /websocket. Requests are
// {"id":"..","method":"..","params":{..}}, responses {"id":"..","data":{..}},
// errors {"id":"..","data":{"error":{"message":".."}}} (the ws layer keeps
// Blockbook's NESTED error object, unlike the REST string shape). Subscription
// pushes reuse the id that subscribed. Every method delegates to the same
// compute fns the REST handlers use; nothing here reimplements chain logic.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use axum::Extension;
use rocksdb::DB;
use std::collections::HashSet;
use std::sync::Arc;
use tracing::{debug, warn};

use crate::cache::CacheManager;
use crate::mempool::MempoolState;
use crate::websocket::{BlockchainEvent, EventBroadcaster};

struct Ctx {
    db: Arc<DB>,
    cache: Arc<CacheManager>,
    mempool: Arc<MempoolState>,
}

#[derive(Default)]
struct Subs {
    new_block: Option<String>,
    new_tx: Option<String>,
    addresses: Option<(String, HashSet<String>)>,
}

pub async fn blockbook_websocket_handler(
    ws: WebSocketUpgrade,
    Extension(db): Extension<Arc<DB>>,
    Extension(cache): Extension<Arc<CacheManager>>,
    Extension(mempool): Extension<Arc<MempoolState>>,
    Extension(broadcaster): Extension<Arc<EventBroadcaster>>,
) -> Response {
    ws.on_upgrade(move |socket| handle_connection(socket, Ctx { db, cache, mempool }, broadcaster))
}

async fn handle_connection(mut socket: WebSocket, ctx: Ctx, broadcaster: Arc<EventBroadcaster>) {
    let mut block_rx = broadcaster.block_tx.subscribe();
    let mut tx_rx = broadcaster.transaction_tx.subscribe();
    let mut mp_rx = broadcaster.mempool_tx.subscribe();
    let mut subs = Subs::default();

    loop {
        tokio::select! {
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        let reply = handle_request(&ctx, &mut subs, &text).await;
                        if socket.send(Message::Text(reply.into())).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Ping(p))) => {
                        if socket.send(Message::Pong(p)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
            ev = block_rx.recv() => {
                if let (Ok(BlockchainEvent::NewBlock { height, hash, .. }), Some(id)) = (ev, subs.new_block.as_ref()) {
                    let push = envelope(id, serde_json::json!({ "height": height, "hash": hash }));
                    if socket.send(Message::Text(push.into())).await.is_err() {
                        break;
                    }
                }
            }
            ev = tx_rx.recv() => {
                if let Ok(BlockchainEvent::NewTransaction { txid, .. }) = ev {
                    if push_tx_events(&ctx, &subs, &mut socket, &txid, false).await.is_err() {
                        break;
                    }
                }
            }
            ev = mp_rx.recv() => {
                if let Ok(BlockchainEvent::MempoolUpdate { txid, action }) = ev {
                    if action == "added"
                        && push_tx_events(&ctx, &subs, &mut socket, &txid, true).await.is_err()
                    {
                        break;
                    }
                }
            }
        }
    }
}

/// Push a tx to newTransaction and matching subscribeAddresses subscribers.
/// The tx object is built once, only when a relevant subscription exists.
async fn push_tx_events(
    ctx: &Ctx,
    subs: &Subs,
    socket: &mut WebSocket,
    txid: &str,
    from_mempool: bool,
) -> Result<(), axum::Error> {
    let want_tx = subs.new_tx.is_some();
    let want_addr = subs
        .addresses
        .as_ref()
        .map(|(_, set)| !set.is_empty())
        .unwrap_or(false);
    if !want_tx && !want_addr {
        return Ok(());
    }

    let tx_value = if from_mempool {
        match ctx.mempool.raw_hex(txid).await {
            Some(raw) => {
                match crate::api::transactions::build_unconfirmed_transaction(&ctx.db, txid, &raw)
                    .await
                {
                    Ok(t) => serde_json::to_value(t).ok(),
                    Err(_) => None,
                }
            }
            None => None,
        }
    } else {
        crate::api::transactions::compute_transaction_details(&ctx.db, txid)
            .await
            .ok()
    };
    let Some(tx_value) = tx_value else {
        return Ok(());
    };

    if let Some(id) = subs.new_tx.as_ref() {
        socket
            .send(Message::Text(envelope(id, tx_value.clone()).into()))
            .await?;
    }
    if let Some((id, set)) = subs.addresses.as_ref() {
        for addr in tx_addresses(&tx_value) {
            if set.contains(&addr) {
                let push = envelope(
                    id,
                    serde_json::json!({ "address": addr, "tx": tx_value.clone() }),
                );
                socket.send(Message::Text(push.into())).await?;
            }
        }
    }
    Ok(())
}

/// Every distinct address in a tx object's vin+vout.
fn tx_addresses(tx: &serde_json::Value) -> HashSet<String> {
    let mut out = HashSet::new();
    for side in ["vin", "vout"] {
        if let Some(items) = tx.get(side).and_then(|v| v.as_array()) {
            for item in items {
                if let Some(addrs) = item.get("addresses").and_then(|a| a.as_array()) {
                    for a in addrs {
                        if let Some(s) = a.as_str() {
                            out.insert(s.to_string());
                        }
                    }
                }
            }
        }
    }
    out
}

fn envelope(id: &str, data: serde_json::Value) -> String {
    serde_json::json!({ "id": id, "data": data }).to_string()
}

fn err_envelope(id: &str, msg: &str) -> String {
    envelope(id, serde_json::json!({ "error": { "message": msg } }))
}

async fn handle_request(ctx: &Ctx, subs: &mut Subs, text: &str) -> String {
    let Ok(req) = serde_json::from_str::<serde_json::Value>(text) else {
        return err_envelope("", "invalid JSON");
    };
    let id = req.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(serde_json::json!({}));

    match dispatch(ctx, subs, id, method, params).await {
        Ok(data) => envelope(id, data),
        Err(msg) => err_envelope(id, &msg),
    }
}

async fn dispatch(
    ctx: &Ctx,
    subs: &mut Subs,
    id: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    match method {
        "getInfo" => {
            let (height, hash) = match crate::chain_state::get_chain_state(&ctx.db) {
                Ok(cs) => (cs.height, cs.hash),
                Err(_) => (0, String::new()),
            };
            Ok(serde_json::json!({
                "name": "PIVX",
                "shortcut": "PIVX",
                "network": "PIVX",
                "decimals": 8,
                "version": env!("CARGO_PKG_VERSION"),
                "bestHeight": height,
                "bestHash": hash,
                "testnet": false,
                "backend": { "version": "", "subversion": "" },
            }))
        }
        "getBlockHash" => {
            let height = params
                .get("height")
                .and_then(|v| v.as_i64())
                .ok_or("missing height")? as i32;
            let cf = ctx
                .db
                .cf_handle("chain_metadata")
                .ok_or("chain_metadata CF not found")?;
            match ctx.db.get_cf(&cf, height.to_le_bytes()) {
                Ok(Some(h)) => Ok(serde_json::json!({ "hash": hex::encode(h) })),
                _ => Err("Block not found".to_string()),
            }
        }
        "getAccountInfo" => {
            let descriptor = params
                .get("descriptor")
                .and_then(|v| v.as_str())
                .ok_or("missing descriptor")?
                .to_string();
            let mut q: super::types::AddressQuery =
                serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
            if q.details.is_empty() {
                q.details = "basic".to_string();
            }
            if descriptor.starts_with("xpub") {
                if !super::addresses::is_valid_xpub(&descriptor) {
                    return Err("Invalid xpub".to_string());
                }
                let mut info = super::addresses::compute_xpub_info(&ctx.db, &descriptor, &q)
                    .await
                    .map_err(|e| e.to_string())?;
                super::addresses::apply_xpub_mempool_overlay(&ctx.mempool, &mut info).await;
                serde_json::to_value(info).map_err(|e| e.to_string())
            } else {
                if !super::addresses::is_valid_address(&descriptor) {
                    return Err(format!("Invalid address '{descriptor}'"));
                }
                let mut info = super::addresses::compute_address_info(&ctx.db, &descriptor, &q)
                    .await
                    .map_err(|e| e.to_string())?;
                super::addresses::apply_address_mempool_overlay(
                    &ctx.db,
                    &ctx.mempool,
                    &mut info,
                    &q,
                )
                .await;
                serde_json::to_value(info).map_err(|e| e.to_string())
            }
        }
        "getAccountUtxo" => {
            let descriptor = params
                .get("descriptor")
                .and_then(|v| v.as_str())
                .ok_or("missing descriptor")?;
            if descriptor.starts_with("xpub") {
                return Err("xpub utxo not supported yet; query per address".to_string());
            }
            if !super::addresses::is_valid_address(descriptor) {
                return Err(format!("Invalid address '{descriptor}'"));
            }
            let mut utxos = super::addresses::compute_utxos(&ctx.db, descriptor)
                .await
                .map_err(|e| e.to_string())?;
            super::addresses::apply_utxo_mempool_overlay(&ctx.mempool, descriptor, &mut utxos)
                .await;
            serde_json::to_value(utxos).map_err(|e| e.to_string())
        }
        "getTransaction" => {
            let txid = params
                .get("txid")
                .and_then(|v| v.as_str())
                .ok_or("missing txid")?;
            if txid.len() != 64 || !txid.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("Invalid txid".to_string());
            }
            let mut tx = crate::api::transactions::compute_transaction_details(&ctx.db, txid)
                .await
                .map_err(|_| format!("Transaction '{txid}' not found"))?;
            crate::api::transactions::freshen_confirmations(&ctx.db, &mut tx);
            Ok(tx)
        }
        "getTransactionSpecific" => {
            let txid = params
                .get("txid")
                .and_then(|v| v.as_str())
                .ok_or("missing txid")?;
            if txid.len() != 64 || !txid.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("Invalid txid".to_string());
            }
            super::helpers::rpc_call_json("getrawtransaction", serde_json::json!([txid, 1]))
                .await
                .map_err(|_| format!("Transaction '{txid}' not found"))
        }
        "getBalanceHistory" => {
            let descriptor = params
                .get("descriptor")
                .and_then(|v| v.as_str())
                .ok_or("missing descriptor")?;
            if !super::addresses::is_valid_address(descriptor) {
                return Err(format!("Invalid address '{descriptor}'"));
            }
            let group_by = params
                .get("groupBy")
                .and_then(|v| v.as_u64())
                .unwrap_or(3600)
                .clamp(60, 2_592_000);
            let from = params.get("from").and_then(|v| v.as_u64());
            let to = params.get("to").and_then(|v| v.as_u64());
            let series = crate::api::balance_history::compute_balance_history(
                &ctx.db, descriptor, group_by, from, to,
            )
            .await
            .map_err(|e| e.to_string())?;
            serde_json::to_value(series).map_err(|e| e.to_string())
        }
        "estimateFee" => {
            let blocks: Vec<u32> = params
                .get("blocks")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|b| b.as_u64())
                        .map(|b| b.min(1008) as u32)
                        .collect()
                })
                .unwrap_or_else(|| vec![2]);
            let mut out = Vec::with_capacity(blocks.len());
            for b in blocks {
                let rate = super::blockbook_status::estimate_fee_rate(&ctx.cache, b).await;
                // feePerUnit is satoshis per kB as a string, per Blockbook.
                out.push(serde_json::json!({
                    "feePerUnit": ((rate * 100_000_000.0).round() as i64).to_string()
                }));
            }
            Ok(serde_json::Value::Array(out))
        }
        "sendTransaction" => {
            let hex_tx = params
                .get("hex")
                .and_then(|v| v.as_str())
                .ok_or("missing hex")?;
            if hex_tx.is_empty()
                || hex_tx.len() > 200_000
                || !hex_tx.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("Invalid transaction hex".to_string());
            }
            match super::helpers::rpc_call_json("sendrawtransaction", serde_json::json!([hex_tx]))
                .await
            {
                Ok(v) => {
                    let txid = v
                        .as_str()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| v.to_string());
                    Ok(serde_json::json!({ "result": txid }))
                }
                Err(e) => Err(format!("Failed to send transaction: {e}")),
            }
        }
        "subscribeNewBlock" => {
            subs.new_block = Some(id.to_string());
            Ok(serde_json::json!({ "subscribed": true }))
        }
        "unsubscribeNewBlock" => {
            subs.new_block = None;
            Ok(serde_json::json!({ "subscribed": false }))
        }
        "subscribeNewTransaction" => {
            subs.new_tx = Some(id.to_string());
            Ok(serde_json::json!({ "subscribed": true }))
        }
        "unsubscribeNewTransaction" => {
            subs.new_tx = None;
            Ok(serde_json::json!({ "subscribed": false }))
        }
        "subscribeAddresses" => {
            let addrs: HashSet<String> = params
                .get("addresses")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|a| a.as_str())
                        .map(|s| s.to_string())
                        .collect()
                })
                .unwrap_or_default();
            // Bound the per-connection set; Blockbook itself subscribes whole
            // wallets, but an unbounded set is a memory hole.
            if addrs.len() > 10_000 {
                return Err("too many addresses".to_string());
            }
            debug!(count = addrs.len(), "ws subscribeAddresses");
            subs.addresses = Some((id.to_string(), addrs));
            Ok(serde_json::json!({ "subscribed": true }))
        }
        "unsubscribeAddresses" => {
            subs.addresses = None;
            Ok(serde_json::json!({ "subscribed": false }))
        }
        // Fiat rates land with the tickers work; honest refusals until then.
        "subscribeFiatRates" => Ok(serde_json::json!({ "subscribed": false })),
        "unsubscribeFiatRates" => Ok(serde_json::json!({ "subscribed": false })),
        "getCurrentFiatRates" | "getFiatRatesForTimestamps" | "getFiatRatesTickersList" => {
            Err("fiat rates not available".to_string())
        }
        "ping" => Ok(serde_json::json!({})),
        other => {
            warn!(method = %other, "ws unknown method");
            Err(format!("unknown method '{other}'"))
        }
    }
}
