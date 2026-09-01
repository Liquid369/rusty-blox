use pivx_rpc_rs::PivxRpcClient;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
/// Mempool Service
///
/// Monitors unconfirmed transactions:
/// - Polls RPC getrawmempool
/// - Tracks pending transactions
/// - Provides fee estimates
/// - Notifies of new transactions
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{error, warn};

use crate::config::get_global_config;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MempoolTransaction {
    pub txid: String,
    pub size: Option<usize>,
    pub fee: Option<f64>,
    pub time: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MempoolInfo {
    pub size: usize,
    pub bytes: usize,
    pub usage: Option<usize>,
    pub transactions: Vec<MempoolTransaction>,
}

/// One parsed mempool tx's transparent footprint, kept across polls so each
/// tx is fetched and prevout-resolved exactly once.
#[derive(Debug, Clone)]
pub struct ParsedMempoolTx {
    pub raw_hex: String,
    /// (address, vout index, value sats) created by this tx.
    pub credits: Vec<(String, u32, i64)>,
    /// (address, value sats, prev display txid, prev vout) spent by this tx.
    pub debits: Vec<(String, i64, String, u32)>,
    pub size: usize,
    /// in + valueBalance - out, sats; None when any prevout is unresolvable.
    pub fee_sats: Option<i64>,
}

/// Address-keyed view over the current mempool, rebuilt on every poll (the
/// PIVX mempool is tiny, so a full rebuild is cheaper than incremental
/// bookkeeping and cannot drift).
#[derive(Debug, Default)]
pub struct MempoolAddressIndex {
    /// addr -> (net pending delta sats, touching txids newest-last)
    pub by_address: HashMap<String, (i64, Vec<String>)>,
    /// Confirmed outpoints (display txid, vout) spent by a mempool tx; /utxo
    /// must hide these or a wallet double-spends its own pending change.
    pub spent_outpoints: std::collections::HashSet<(String, u32)>,
    /// addr -> UTXOs created in the mempool: (txid, vout, value sats).
    pub created_utxos: HashMap<String, Vec<(String, u32, i64)>>,
}

/// Shared mempool state
pub struct MempoolState {
    pub transactions: RwLock<HashMap<String, MempoolTransaction>>,
    /// Parsed footprints, keyed by txid (retained while the tx stays pending).
    pub parsed: RwLock<HashMap<String, ParsedMempoolTx>>,
    pub address_index: RwLock<MempoolAddressIndex>,
}

impl Default for MempoolState {
    fn default() -> Self {
        Self::new()
    }
}

impl MempoolState {
    pub fn new() -> Self {
        Self {
            transactions: RwLock::new(HashMap::new()),
            parsed: RwLock::new(HashMap::new()),
            address_index: RwLock::new(MempoolAddressIndex::default()),
        }
    }

    pub async fn get_info(&self) -> MempoolInfo {
        let txs = self.transactions.read().await;

        MempoolInfo {
            size: txs.len(),
            bytes: txs.values().map(|tx| tx.size.unwrap_or(0)).sum(),
            usage: None,
            transactions: txs.values().cloned().collect(),
        }
    }

    pub async fn get_transaction(&self, txid: &str) -> Option<MempoolTransaction> {
        let txs = self.transactions.read().await;
        txs.get(txid).cloned()
    }

    /// Pending view for one address: (net delta sats, txids touching it).
    pub async fn pending_for_address(&self, address: &str) -> Option<(i64, Vec<String>)> {
        self.address_index
            .read()
            .await
            .by_address
            .get(address)
            .cloned()
    }

    /// Raw hex of a pending tx (for building full unconfirmed tx objects
    /// without another node round trip).
    pub async fn raw_hex(&self, txid: &str) -> Option<String> {
        self.parsed
            .read()
            .await
            .get(txid)
            .map(|p| p.raw_hex.clone())
    }

    /// Mempool adjustments for /utxo: (created utxos for addr, spent outpoints).
    pub async fn utxo_overlay(
        &self,
        address: &str,
    ) -> (
        Vec<(String, u32, i64)>,
        std::collections::HashSet<(String, u32)>,
    ) {
        let idx = self.address_index.read().await;
        (
            idx.created_utxos.get(address).cloned().unwrap_or_default(),
            idx.spent_outpoints.clone(),
        )
    }
}

/// Parse one mempool tx and resolve its transparent footprint. Prevouts are
/// resolved from already-parsed mempool parents first, then the confirmed tx
/// index (stub-safe reader). An unresolvable prevout leaves fee_sats None and
/// contributes no debit; the next poll retries it (mempool parent chains).
async fn parse_mempool_tx(
    db: &Arc<rocksdb::DB>,
    txid: &str,
    parents: &HashMap<String, ParsedMempoolTx>,
) -> Option<ParsedMempoolTx> {
    let raw = crate::api::helpers::rpc_call_json("getrawtransaction", serde_json::json!([txid, 0]))
        .await
        .ok()?;
    let raw_hex = raw.as_str()?.to_string();
    let raw_bytes = hex::decode(&raw_hex).ok()?;
    let size = raw_bytes.len();

    // Resolve parent outputs from the mempool view before touching the DB.
    let parent_outputs: HashMap<(String, u32), Vec<(String, i64)>> = parents
        .iter()
        .flat_map(|(ptxid, p)| {
            let mut grouped: HashMap<u32, Vec<(String, i64)>> = HashMap::new();
            for (addr, vout, val) in &p.credits {
                grouped.entry(*vout).or_default().push((addr.clone(), *val));
            }
            grouped
                .into_iter()
                .map(move |(vout, addrs)| ((ptxid.clone(), vout), addrs))
        })
        .collect();

    let db = Arc::clone(db);
    tokio::task::spawn_blocking(move || -> Option<ParsedMempoolTx> {
        let mut framed = Vec::with_capacity(4 + raw_bytes.len());
        framed.extend_from_slice(&[0u8; 4]);
        framed.extend_from_slice(&raw_bytes);
        let tx = crate::parser::deserialize_transaction_blocking(&framed).ok()?;

        let mut credits = Vec::new();
        let mut value_out = 0i64;
        for o in &tx.outputs {
            value_out += o.value;
            for addr in &o.address {
                credits.push((addr.clone(), o.index as u32, o.value));
            }
        }

        let cf_tx = db.cf_handle("transactions")?;
        let mut debits = Vec::new();
        let mut value_in = 0i64;
        let mut unresolved = false;
        for input in tx.inputs.iter().filter(|i| i.coinbase.is_none()) {
            let Some(prev) = input.prevout.as_ref() else {
                continue;
            };
            // Mempool parent first, then the confirmed index.
            if let Some(addrs) = parent_outputs.get(&(prev.hash.clone(), prev.n)) {
                if let Some((_, val)) = addrs.first() {
                    value_in += val;
                }
                for (addr, val) in addrs {
                    debits.push((addr.clone(), *val, prev.hash.clone(), prev.n));
                }
                continue;
            }
            let resolved = hex::decode(&prev.hash).ok().and_then(|ptx_bytes| {
                crate::api::transactions::read_valid_tx_record(&db, &cf_tx, &ptx_bytes)
                    .ok()
                    .flatten()
            });
            let Some(rec) = resolved else {
                unresolved = true;
                continue;
            };
            let mut pframed = Vec::with_capacity(4 + rec.len() - 8);
            pframed.extend_from_slice(&[0u8; 4]);
            pframed.extend_from_slice(&rec[8..]);
            let Ok(ptx) = crate::parser::deserialize_transaction_blocking(&pframed) else {
                unresolved = true;
                continue;
            };
            let Some(out) = ptx.outputs.iter().find(|o| o.index as u32 == prev.n) else {
                unresolved = true;
                continue;
            };
            value_in += out.value;
            for addr in &out.address {
                debits.push((addr.clone(), out.value, prev.hash.clone(), prev.n));
            }
        }

        let vb = tx
            .sapling_data
            .as_ref()
            .map(|s| s.value_balance)
            .unwrap_or(0);
        let fee_sats = if unresolved {
            None
        } else {
            Some((value_in + vb - value_out).max(0))
        };
        Some(ParsedMempoolTx {
            raw_hex,
            credits,
            debits,
            size,
            fee_sats,
        })
    })
    .await
    .ok()?
}

/// Rebuild the address view from the parsed set (full rebuild per poll; the
/// PIVX mempool is tiny and a rebuild cannot drift).
fn rebuild_index(parsed: &HashMap<String, ParsedMempoolTx>) -> MempoolAddressIndex {
    let mut idx = MempoolAddressIndex::default();
    // Sorted iteration: HashMap order made the pending txid lists flap
    // between polls, which paginating clients see as reordering.
    let mut ordered: Vec<(&String, &ParsedMempoolTx)> = parsed.iter().collect();
    ordered.sort_by(|a, b| a.0.cmp(b.0));
    for (txid, p) in ordered.iter().map(|(t, p)| (*t, *p)) {
        for (addr, _, val) in &p.credits {
            let e = idx.by_address.entry(addr.clone()).or_default();
            e.0 += val;
            if !e.1.contains(txid) {
                e.1.push(txid.clone());
            }
        }
        for (addr, val, ptx, pvout) in &p.debits {
            let e = idx.by_address.entry(addr.clone()).or_default();
            e.0 -= val;
            if !e.1.contains(txid) {
                e.1.push(txid.clone());
            }
            idx.spent_outpoints.insert((ptx.clone(), *pvout));
        }
    }
    // Mempool-created UTXOs, minus any consumed by another mempool tx.
    let mut ordered: Vec<(&String, &ParsedMempoolTx)> = parsed.iter().collect();
    ordered.sort_by(|a, b| a.0.cmp(b.0));
    for (txid, p) in ordered.iter().map(|(t, p)| (*t, *p)) {
        for (addr, vout, val) in &p.credits {
            if !idx.spent_outpoints.contains(&(txid.clone(), *vout)) {
                idx.created_utxos.entry(addr.clone()).or_default().push((
                    txid.clone(),
                    *vout,
                    *val,
                ));
            }
        }
    }
    idx
}

/// Monitor mempool for new transactions
pub async fn run_mempool_monitor(
    mempool_state: Arc<MempoolState>,
    db: Arc<rocksdb::DB>,
    broadcaster: Option<Arc<crate::websocket::EventBroadcaster>>,
    poll_interval_secs: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    // Initialize RPC client
    let config = get_global_config();
    let rpc_host = config.get_string("rpc.host")?;
    let rpc_user = config.get_string("rpc.user")?;
    let rpc_pass = config.get_string("rpc.pass")?;

    // Create RPC client in a completely separate OS thread
    // PivxRpcClient uses reqwest::blocking which creates its own runtime.
    // RETRY with backoff: pivxd is routinely not up yet when the explorer boots
    // (a host reboot starts both services). The old code returned Ok(()) on the
    // first failure — the task died permanently and the mempool served frozen-
    // empty for the process lifetime, indistinguishable from real data. Mirrors
    // the sync thread's boot retry loop.
    let rpc_client = loop {
        let (tx, rx) = std::sync::mpsc::channel();
        let rpc_host_clone = rpc_host.clone();
        let rpc_user_clone = rpc_user.clone();
        let rpc_pass_clone = rpc_pass.clone();

        std::thread::spawn(move || {
            let client = PivxRpcClient::new(
                rpc_host_clone,
                Some(rpc_user_clone),
                Some(rpc_pass_clone),
                3,
                10,
                1000,
            );

            // Test connection
            let result = match client.getblockcount() {
                Ok(_) => Ok(Arc::new(client)),
                Err(e) => Err(e),
            };
            let _ = tx.send(result);
        });

        match rx.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(Ok(client)) => break client,
            Ok(Err(e)) => {
                error!(error = ?e, retry_secs = 30, "Mempool RPC connection failed; retrying")
            }
            Err(_) => error!(
                retry_secs = 30,
                "Mempool RPC connection timed out; retrying"
            ),
        }
        tokio::time::sleep(Duration::from_secs(30)).await;
    };

    // Txids whose "added" event has fired. An event announces a tx only once
    // its parse landed (subscribers build the push from the parsed cache); a
    // tx whose parse failed this poll is announced on a later poll instead of
    // being dropped forever.
    let mut announced: std::collections::HashSet<String> = std::collections::HashSet::new();

    loop {
        tokio::time::sleep(Duration::from_secs(poll_interval_secs)).await;

        // Get raw mempool - must use separate thread
        let client = Arc::clone(&rpc_client);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = client.getrawmempool(false);
            let _ = tx.send(result);
        });

        let mempool_result = match rx.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(Ok(result)) => result,
            Ok(Err(e)) => {
                error!(error = ?e, "Failed to get mempool");
                continue;
            }
            Err(_) => {
                error!("Mempool RPC call timed out");
                continue;
            }
        };

        // Extract txids based on RawMemPool variant
        let txids: Vec<String> = match mempool_result {
            pivx_rpc_rs::RawMemPool::TxIds(txid_list) => txid_list,
            pivx_rpc_rs::RawMemPool::Verbose(_) => {
                warn!("Unexpected verbose mempool response");
                continue;
            }
        };

        let current_txids: std::collections::HashSet<String> = txids.iter().cloned().collect();
        let removed: Vec<String> = {
            let txs = mempool_state.transactions.read().await;
            txs.keys()
                .filter(|t| !current_txids.contains(*t))
                .cloned()
                .collect()
        };
        {
            let mut txs = mempool_state.transactions.write().await;
            // Remove confirmed transactions (keep only those still in mempool)
            txs.retain(|txid, _| current_txids.contains(txid));
            for txid in &txids {
                if !txs.contains_key(txid) {
                    txs.insert(
                        txid.clone(),
                        MempoolTransaction {
                            txid: txid.clone(),
                            size: None,
                            fee: None,
                            time: Some(
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or(std::time::Duration::from_secs(0))
                                    .as_secs(),
                            ),
                        },
                    );
                }
            }
        }

        // Maintain the parsed footprints: drop confirmed, parse new arrivals,
        // and retry any whose prevouts were unresolvable (mempool chains).
        {
            let mut parsed = mempool_state.parsed.write().await;
            parsed.retain(|txid, _| current_txids.contains(txid));
        }
        let need: Vec<String> = {
            let parsed = mempool_state.parsed.read().await;
            current_txids
                .iter()
                .filter(|t| parsed.get(*t).map(|p| p.fee_sats.is_none()).unwrap_or(true))
                .cloned()
                .collect()
        };
        // Two passes: a child can arrive in `need` before its mempool parent;
        // the second pass resolves it once the parent is parsed.
        for _ in 0..2 {
            let pass: Vec<String> = {
                let parsed = mempool_state.parsed.read().await;
                need.iter()
                    .filter(|t| parsed.get(*t).map(|p| p.fee_sats.is_none()).unwrap_or(true))
                    .cloned()
                    .collect()
            };
            if pass.is_empty() {
                break;
            }
            for txid in pass {
                let parents = mempool_state.parsed.read().await.clone();
                if let Some(p) = parse_mempool_tx(&db, &txid, &parents).await {
                    {
                        let mut txs = mempool_state.transactions.write().await;
                        if let Some(t) = txs.get_mut(&txid) {
                            t.size = Some(p.size);
                            t.fee = p.fee_sats.map(|f| f as f64 / 100_000_000.0);
                        }
                    }
                    mempool_state.parsed.write().await.insert(txid, p);
                }
            }
        }
        let idx = rebuild_index(&*mempool_state.parsed.read().await);
        *mempool_state.address_index.write().await = idx;

        // Events AFTER the parsed cache is ready, so websocket pushes can
        // build the pending tx from state.
        if let Some(bc) = &broadcaster {
            announced.retain(|t| current_txids.contains(t));
            {
                let parsed = mempool_state.parsed.read().await;
                for txid in &current_txids {
                    if parsed.contains_key(txid) && announced.insert(txid.clone()) {
                        bc.broadcast_mempool_update(txid.clone(), "added".to_string());
                    }
                }
            }
            for txid in &removed {
                bc.broadcast_mempool_update(txid.clone(), "removed".to_string());
            }
        }
    }
}
