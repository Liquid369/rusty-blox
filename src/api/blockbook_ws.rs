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

/// Mirrors the REST broadcast cap: ws sendTransaction writes to the node and
/// must not become an uncapped side door.
static WS_SEND_LIMIT: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

/// Ws-side reindex gate, same contract as the REST 503.
fn require_addr_index(ctx: &Ctx) -> Result<(), String> {
    if crate::chain_state::addr_index_ready(&ctx.db) {
        Ok(())
    } else {
        Err("Address index is reindexing; please retry shortly".to_string())
    }
}

struct Ctx {
    db: Arc<DB>,
    cache: Arc<CacheManager>,
    mempool: Arc<MempoolState>,
}

#[derive(Default)]
struct Subs {
    new_block: Option<serde_json::Value>,
    new_tx: Option<serde_json::Value>,
    addresses: Option<(serde_json::Value, HashSet<String>)>,
}

pub async fn blockbook_websocket_handler(
    ws: WebSocketUpgrade,
    headers: axum::http::HeaderMap,
    Extension(db): Extension<Arc<DB>>,
    Extension(cache): Extension<Arc<CacheManager>>,
    Extension(mempool): Extension<Arc<MempoolState>>,
    Extension(broadcaster): Extension<Arc<EventBroadcaster>>,
) -> Response {
    // Same connection cap + origin policy as the /ws/* channels; the permit
    // lives for the socket lifetime. 1MB frame cap bounds request parsing.
    let permit = match crate::websocket::ws_guard(&headers) {
        Ok(p) => p,
        Err(resp) => return resp,
    };
    ws.max_message_size(1 << 20)
        .on_upgrade(move |socket| async move {
            let _permit = permit;
            handle_connection(socket, Ctx { db, cache, mempool }, broadcaster).await;
        })
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

/// Rates object honoring an optional ws `currencies` filter list.
fn ws_rates(params: &serde_json::Value, rates: crate::api::tickers::DayRates) -> serde_json::Value {
    let filter: Option<Vec<String>> =
        params
            .get("currencies")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|c| c.as_str())
                    .map(|c| c.to_lowercase())
                    .collect()
            });
    let mut out = serde_json::Map::new();
    for cur in crate::api::tickers::CURRENCIES {
        if filter
            .as_ref()
            .map(|f| f.iter().any(|c| c == cur))
            .unwrap_or(true)
        {
            if let Some(v) = rates.get(cur) {
                out.insert(cur.to_string(), serde_json::json!(v));
            }
        }
    }
    serde_json::Value::Object(out)
}

fn envelope(id: &serde_json::Value, data: serde_json::Value) -> String {
    serde_json::json!({ "id": id, "data": data }).to_string()
}

fn err_envelope(id: &serde_json::Value, msg: &str) -> String {
    envelope(id, serde_json::json!({ "error": { "message": msg } }))
}

async fn handle_request(ctx: &Ctx, subs: &mut Subs, text: &str) -> String {
    let Ok(req) = serde_json::from_str::<serde_json::Value>(text) else {
        return err_envelope(&serde_json::Value::Null, "invalid JSON");
    };
    let id = req.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(serde_json::json!({}));

    match dispatch(ctx, subs, &id, method, params).await {
        Ok(data) => envelope(&id, data),
        Err(msg) => err_envelope(&id, &msg),
    }
}

async fn dispatch(
    ctx: &Ctx,
    subs: &mut Subs,
    id: &serde_json::Value,
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
            require_addr_index(ctx)?;
            let descriptor = params
                .get("descriptor")
                .and_then(|v| v.as_str())
                .ok_or("missing descriptor")?
                .to_string();
            let details_given = params.get("details").is_some();
            let mut q: super::types::AddressQuery =
                serde_json::from_value(params).map_err(|e| format!("bad params: {e}"))?;
            if !details_given {
                // Blockbook's ws default; AddressQuery's serde default is the
                // REST "txids".
                q.details = "basic".to_string();
            }
            if descriptor.starts_with("xpub") {
                if !super::addresses::is_valid_xpub(&descriptor) {
                    return Err("Invalid xpub".to_string());
                }
                let db = Arc::clone(&ctx.db);
                let d = descriptor.clone();
                let qc = q.clone();
                let mut info = ctx
                    .cache
                    .get_or_compute(
                        &format!("xpub:{descriptor}:{q:?}"),
                        std::time::Duration::from_secs(300),
                        || async move { super::addresses::compute_xpub_info(&db, &d, &qc).await },
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                super::addresses::apply_xpub_mempool_overlay(
                    &ctx.mempool,
                    &descriptor,
                    q.gap_limit.unwrap_or(20),
                    &mut info,
                )
                .await;
                serde_json::to_value(info).map_err(|e| e.to_string())
            } else {
                if !super::addresses::is_valid_address(&descriptor) {
                    return Err(format!("Invalid address '{descriptor}'"));
                }
                let db = Arc::clone(&ctx.db);
                let d = descriptor.clone();
                let qc = q.clone();
                let mut info = ctx
                    .cache
                    .get_or_compute(
                        &format!("addr:{descriptor}:{q:?}"),
                        std::time::Duration::from_secs(30),
                        || async move { super::addresses::compute_address_info(&db, &d, &qc).await },
                    )
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
            require_addr_index(ctx)?;
            let db = Arc::clone(&ctx.db);
            let d = descriptor.to_string();
            let mut utxos = ctx
                .cache
                .get_or_compute(
                    &format!("utxo:{descriptor}:false"),
                    std::time::Duration::from_secs(30),
                    || async move { super::addresses::compute_utxos(&db, &d).await },
                )
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
            require_addr_index(ctx)?;
            let db = Arc::clone(&ctx.db);
            let d = descriptor.to_string();
            let mut series = ctx
                .cache
                .get_or_compute(
                    &format!("balhist:{descriptor}:{group_by}:{from:?}:{to:?}"),
                    std::time::Duration::from_secs(300),
                    || async move {
                        crate::api::balance_history::compute_balance_history(
                            &db, &d, group_by, from, to,
                        )
                        .await
                    },
                )
                .await
                .map_err(|e| e.to_string())?;
            crate::api::balance_history::attach_rates(
                &ctx.db,
                &ctx.cache,
                &mut series,
                params.get("fiatcurrency").and_then(|v| v.as_str()),
            )
            .await;
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
            let _permit = WS_SEND_LIMIT
                .try_acquire()
                .map_err(|_| "server busy: broadcast limit reached".to_string())?;
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
            subs.new_block = Some(id.clone());
            Ok(serde_json::json!({ "subscribed": true }))
        }
        "unsubscribeNewBlock" => {
            subs.new_block = None;
            Ok(serde_json::json!({ "subscribed": false }))
        }
        "subscribeNewTransaction" => {
            subs.new_tx = Some(id.clone());
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
            subs.addresses = Some((id.clone(), addrs));
            Ok(serde_json::json!({ "subscribed": true }))
        }
        "unsubscribeAddresses" => {
            subs.addresses = None;
            Ok(serde_json::json!({ "subscribed": false }))
        }
        // Fiat pushes are not implemented (clients poll getCurrentFiatRates);
        // subscribed:false tells them so honestly.
        "subscribeFiatRates" => Ok(serde_json::json!({ "subscribed": false })),
        "unsubscribeFiatRates" => Ok(serde_json::json!({ "subscribed": false })),
        "getCurrentFiatRates" => {
            let series = super::tickers::cached_series_pub(&ctx.db, &ctx.cache).await;
            let (ts, rates) =
                super::tickers::rate_at(&series, None).ok_or("no fiat rates available")?;
            Ok(serde_json::json!({ "ts": ts, "rates": ws_rates(&params, rates) }))
        }
        "getFiatRatesForTimestamps" => {
            let series = super::tickers::cached_series_pub(&ctx.db, &ctx.cache).await;
            let stamps: Vec<u64> = params
                .get("timestamps")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|t| t.as_u64()).collect())
                .unwrap_or_default();
            if stamps.is_empty() {
                return Err("missing timestamps".to_string());
            }
            let tickers: Vec<serde_json::Value> = stamps
                .iter()
                .map(|t| match super::tickers::rate_at(&series, Some(*t)) {
                    Some((ts, rates)) => {
                        serde_json::json!({ "ts": ts, "rates": ws_rates(&params, rates) })
                    }
                    None => serde_json::json!({ "ts": t, "rates": {} }),
                })
                .collect();
            Ok(serde_json::json!({ "tickers": tickers }))
        }
        "getFiatRatesTickersList" => {
            let series = super::tickers::cached_series_pub(&ctx.db, &ctx.cache).await;
            let (ts, _) =
                super::tickers::rate_at(&series, params.get("timestamp").and_then(|v| v.as_u64()))
                    .ok_or("no fiat rates available")?;
            Ok(serde_json::json!({
                "ts": ts,
                "available_currencies": super::tickers::CURRENCIES,
            }))
        }
        "ping" => Ok(serde_json::json!({})),
        other => {
            debug!(method = %other, "ws unknown method");
            Err(format!("unknown method '{other}'"))
        }
    }
}
