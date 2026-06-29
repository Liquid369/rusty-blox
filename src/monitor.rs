use crate::metrics;
use pivx_rpc_rs::PivxRpcClient;
use rocksdb::DB;
use serde_json::Value;
use std::collections::HashSet;
/// Block Monitor Service - Real-time blockchain monitoring via RPC
///
/// Responsibilities:
/// - Poll RPC node for new blocks
/// - Detect chain tip changes
/// - Trigger block indexing
/// - Detect and handle reorgs
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, info_span, warn};

use crate::chain_state::set_network_height;
use crate::config::get_global_config;
use crate::reorg;
use crate::websocket::EventBroadcaster;

#[derive(Debug, Clone)]
pub struct ChainTip {
    pub height: i32,
    pub hash: String,
}

/// Hourly network snapshot (Tier-3 forward-only analytics series).
/// Stored as a bincode Vec under chain_state key `analytics_snapshots`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HourlySnapshot {
    pub ts: u64,
    pub mempool_txs: u64,
    pub mempool_bytes: u64,
    pub masternode_count: u64,
    pub shield_supply_piv: f64,
    pub transparent_supply_piv: f64,
}

/// Retain one year of hourly snapshots.
const SNAPSHOT_CAP: usize = 8760;

/// Hour (unix ts / 3600) of the last persisted snapshot, so restarts don't
/// double-write within the same hour. 0 when no series exists yet.
fn read_last_snapshot_hour(db: &Arc<DB>) -> u64 {
    db.cf_handle("chain_state")
        .and_then(|cf| db.get_cf(&cf, b"analytics_snapshots").ok().flatten())
        .and_then(|b| bincode::deserialize::<Vec<HourlySnapshot>>(&b).ok())
        .and_then(|v| v.last().map(|s| s.ts / 3600))
        .unwrap_or(0)
}

/// Collect and append one hourly snapshot (mempool, masternodes, supply) to
/// the chain_state `analytics_snapshots` series (load-append-store, capped).
async fn write_hourly_snapshot(
    db: &Arc<DB>,
    ts: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use crate::api::helpers::rpc_call_json;

    let mempool = rpc_call_json("getmempoolinfo", serde_json::json!([])).await?;
    let mn = rpc_call_json("getmasternodecount", serde_json::json!([])).await?;
    let supply = rpc_call_json("getsupplyinfo", serde_json::json!([false])).await?;

    let snapshot = HourlySnapshot {
        ts,
        mempool_txs: mempool.get("size").and_then(|v| v.as_u64()).unwrap_or(0),
        mempool_bytes: mempool.get("bytes").and_then(|v| v.as_u64()).unwrap_or(0),
        masternode_count: mn.get("total").and_then(|v| v.as_u64()).unwrap_or(0),
        shield_supply_piv: supply
            .get("shieldsupply")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        transparent_supply_piv: supply
            .get("transparentsupply")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
    };

    let cf_state = db
        .cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    // Load-append-store; an unreadable (old-schema) blob restarts the series.
    let mut series: Vec<HourlySnapshot> = db
        .get_cf(&cf_state, b"analytics_snapshots")?
        .and_then(|b| bincode::deserialize(&b).ok())
        .unwrap_or_default();
    series.push(snapshot);
    if series.len() > SNAPSHOT_CAP {
        let excess = series.len() - SNAPSHOT_CAP;
        series.drain(0..excess);
    }
    db.put_cf(
        &cf_state,
        b"analytics_snapshots",
        bincode::serialize(&series)?,
    )?;
    Ok(())
}

/// Structure to hold fetched block data for two-phase processing
#[derive(Debug, Clone)]
struct FetchedBlock {
    height: i32,
    block_hash: String,
    json_result: Value,
}

/// Get current chain tip from RPC node.
///
/// Fully async via the shared `rpc_call_json` helper — no more detached
/// `std::thread::spawn` + `recv_timeout` (which leaked an OS thread on every
/// node timeout). The helper carries its own hard HTTP timeout.
async fn get_rpc_chain_tip() -> Result<ChainTip, Box<dyn std::error::Error + Send + Sync>> {
    use crate::api::helpers::rpc_call_json;

    let timer = metrics::Timer::new();
    let height_i64 = rpc_call_json("getblockcount", serde_json::json!([]))
        .await?
        .as_i64()
        .ok_or("getblockcount returned non-integer")?;
    let height = height_i64 as i32;

    metrics::RPC_CALL_DURATION
        .with_label_values(&["getblockcount"])
        .observe(timer.elapsed_secs());

    let timer2 = metrics::Timer::new();
    let hash = rpc_call_json("getblockhash", serde_json::json!([height as i64]))
        .await?
        .as_str()
        .ok_or("getblockhash returned non-string")?
        .to_string();

    metrics::RPC_CALL_DURATION
        .with_label_values(&["getblockhash"])
        .observe(timer2.elapsed_secs());

    Ok(ChainTip { height, hash })
}

/// Get our current chain tip from database
fn get_db_chain_tip(db: &Arc<DB>) -> Result<ChainTip, Box<dyn std::error::Error>> {
    let cf_state = db
        .cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;

    // Get sync height
    let height = match db.get_cf(&cf_state, b"sync_height")? {
        Some(bytes) => i32::from_le_bytes(bytes.as_slice().try_into()?),
        None => {
            // Fallback: scan chain_metadata
            let cf_metadata = db
                .cf_handle("chain_metadata")
                .ok_or("chain_metadata CF not found")?;

            let mut h: i32 = 0;
            loop {
                let key = h.to_le_bytes().to_vec();
                match db.get_cf(&cf_metadata, &key)? {
                    Some(_) => h += 1,
                    None => break,
                }
                if h > 10_000_000 {
                    break;
                }
            }
            h - 1
        }
    };

    // Get block hash at this height
    let cf_metadata = db
        .cf_handle("chain_metadata")
        .ok_or("chain_metadata CF not found")?;

    let height_key = height.to_le_bytes().to_vec();
    let hash_bytes = db
        .get_cf(&cf_metadata, &height_key)?
        .ok_or("Block hash not found for current height")?;

    let hash = hex::encode(&hash_bytes);

    Ok(ChainTip { height, hash })
}

/// Phase 1a: Fetch block data from RPC (without indexing)
/// Returns parsed block JSON for later processing
async fn fetch_block_data(height: i32) -> Result<FetchedBlock, Box<dyn std::error::Error>> {
    use crate::api::helpers::rpc_call_json;

    let config = get_global_config();
    let url = config.get_string("rpc.host")?;
    let user = config.get_string("rpc.user")?;
    let pass = config.get_string("rpc.pass")?;

    let client = reqwest::Client::new();

    // Get block hash at this height. Async helper (carries its own timeout)
    // instead of a detached thread::spawn + recv_timeout that leaked a thread
    // on every node timeout.
    let timer = metrics::Timer::new();
    let block_hash = rpc_call_json("getblockhash", serde_json::json!([height as i64]))
        .await
        .map_err(|e| format!("RPC error getting block hash: {e}"))?
        .as_str()
        .ok_or("getblockhash returned non-string")?
        .to_string();
    metrics::RPC_CALL_DURATION
        .with_label_values(&["getblockhash"])
        .observe(timer.elapsed_secs());

    // Fetch block with full transaction data (verbosity=2)
    let timer = metrics::Timer::new();
    let response = client
        .post(&url)
        .basic_auth(&user, Some(&pass))
        .json(&serde_json::json!({
            "jsonrpc": "1.0",
            "id": "rustyblox",
            "method": "getblock",
            "params": [block_hash.clone(), 2]
        }))
        .send()
        .await?;

    let json: Value = response.json().await?;
    let result = json
        .get("result")
        .ok_or("No result in RPC response")?
        .clone();

    let elapsed = timer.elapsed_secs();
    metrics::RPC_CALL_DURATION
        .with_label_values(&["getblock"])
        .observe(elapsed);

    if elapsed > 5.0 {
        warn!(
            method = "getblock",
            height = height,
            duration_secs = elapsed,
            "Slow RPC call"
        );
    }

    Ok(FetchedBlock {
        height,
        block_hash,
        json_result: result,
    })
}

/// Phase 1b: Build complete spent set from fetched blocks
/// This scans all transactions in the blocks to identify which outputs are spent
fn build_spent_set_from_blocks(blocks: &[FetchedBlock]) -> HashSet<(Vec<u8>, u64)> {
    let mut spent_set = HashSet::new();

    for block in blocks {
        // Extract transactions from block JSON
        if let Some(tx_array) = block.json_result.get("tx").and_then(|t| t.as_array()) {
            for tx_val in tx_array {
                // Parse transaction to get inputs
                if let Some(tx_obj) = tx_val.as_object() {
                    // Get inputs array (vin)
                    if let Some(vin_array) = tx_obj.get("vin").and_then(|v| v.as_array()) {
                        for input in vin_array {
                            // Skip coinbase inputs (have "coinbase" field instead of "txid")
                            if input.get("coinbase").is_some() {
                                continue;
                            }

                            // Get previous output reference
                            if let (Some(txid_str), Some(vout)) = (
                                input.get("txid").and_then(|t| t.as_str()),
                                input.get("vout").and_then(|v| v.as_u64()),
                            ) {
                                // Convert txid from RPC (display format hex) to bytes
                                // RPC returns txid in display format, use it directly (matches DB keys)
                                if let Ok(txid_bytes) = hex::decode(txid_str) {
                                    spent_set.insert((txid_bytes, vout));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    spent_set
}

/// Fetch and index a single block from RPC
///
/// Parameters:
/// - spent_set: Optional pre-built set of spent outputs. If provided, spend detection
///   is done via HashSet lookup instead of on-demand RPC fetching. This ensures
///   100% accurate spend detection matching the initial sync two-pass algorithm.
async fn index_block_from_rpc(
    // Block hashes come from the async `rpc_call_json` helper (no blocking client).
    height: i32,
    db: &Arc<DB>,
    broadcaster: &Option<Arc<EventBroadcaster>>,
    spent_set: Option<&HashSet<(Vec<u8>, u64)>>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Check if address enrichment has completed and at what height
    // Only update address index for blocks AFTER enrichment height
    // (Enrichment already processed all historical blocks correctly)
    let cf_state = db
        .cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;

    let enrichment_height = db
        .get_cf(&cf_state, b"enrichment_height")?
        .and_then(|bytes| {
            if bytes.len() >= 4 {
                Some(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            } else {
                None
            }
        });

    // Only update address index if:
    // 1. Enrichment hasn't run yet (enrichment_height is None), OR
    // 2. This block is NEWER than enrichment height (arrived after enrichment)
    let should_update_address_index = match enrichment_height {
        None => true, // Enrichment hasn't run, update address index
        Some(enrich_height) => height > enrich_height, // Only for NEW blocks
    };

    // Get block hash at this height. Async helper (carries its own timeout)
    // instead of a detached thread::spawn + recv_timeout that leaked a thread
    // on every node timeout.
    let block_hash =
        crate::api::helpers::rpc_call_json("getblockhash", serde_json::json!([height as i64]))
            .await
            .map_err(|e| format!("RPC error getting block hash: {e}"))?
            .as_str()
            .ok_or("getblockhash returned non-string")?
            .to_string();

    // CRITICAL FIX: Atomic check-and-reserve to prevent race conditions
    // Step 1: Check if already indexed OR being processed
    let mut height_hash_key = vec![b'H'];
    height_hash_key.extend(&height.to_le_bytes());

    let mut processing_key = vec![b'P']; // P = "Processing" marker
    processing_key.extend(&height.to_le_bytes());

    // Use a transaction-like approach with RocksDB's write batch for atomicity
    if let Some(existing_hash) = db.get_cf(&cf_state, &height_hash_key)? {
        let existing_hash_str = String::from_utf8_lossy(&existing_hash);
        if existing_hash_str == block_hash {
            // Already indexed this exact block - skip silently
            return Ok(());
        } else {
            // Different block at same height - REORG detected!
            warn!(height = height, expected = %existing_hash_str, current = %block_hash, "REORG detected");
            // Delete processing marker if it exists (allow reindex)
            db.delete_cf(&cf_state, &processing_key).ok();
        }
    }

    // Step 2: Try to reserve this height atomically
    // If processing marker already exists, another task is working on it
    if db.get_cf(&cf_state, &processing_key)?.is_some() {
        // Another task is already processing this height - skip
        return Ok(());
    }

    // Step 3: Set processing marker to claim this height
    // Use a short TTL value as the marker (height as bytes)
    db.put_cf(&cf_state, &processing_key, height.to_le_bytes())?;

    // RAII guard to ensure processing marker is cleaned up even on error
    struct ProcessingGuard<'a> {
        db: &'a Arc<DB>,
        cf_state: &'a rocksdb::ColumnFamily,
        key: Vec<u8>,
    }

    impl<'a> Drop for ProcessingGuard<'a> {
        fn drop(&mut self) {
            self.db.delete_cf(self.cf_state, &self.key).ok();
        }
    }

    let _guard = ProcessingGuard {
        db,
        cf_state,
        key: processing_key.clone(),
    };

    // From this point forward, we own this height's processing
    // The guard will clean up the marker automatically when function exits

    // Make raw RPC call to get block with verbosity=2 (full transaction data)
    // We can't use the library's getblock because FullBlock deserialization
    // fails when the response has mixed string/object types in the tx array
    let config = get_global_config();
    let url = config.get_string("rpc.host")?;
    let user = config.get_string("rpc.user")?;
    let pass = config.get_string("rpc.pass")?;

    let client = reqwest::Client::new();

    // Use verbosity=2 for full block data with all transactions
    // This is more efficient than fetching each TX separately
    let response = client
        .post(&url)
        .basic_auth(&user, Some(&pass))
        .json(&serde_json::json!({
            "jsonrpc": "1.0",
            "id": "rustyblox",
            "method": "getblock",
            "params": [block_hash.clone(), 2]  // verbosity=2 includes full TX data
        }))
        .send()
        .await?;

    let json: Value = response.json().await?;
    let result = json.get("result").ok_or("No result in RPC response")?;

    // Extract block version and transactions
    let version = result.get("version").and_then(|v| v.as_i64()).unwrap_or(1) as i32;

    let tx_array = result
        .get("tx")
        .and_then(|t| t.as_array())
        .ok_or("No tx array in block")?;

    // Store block header data in chain_metadata
    let cf_metadata = db
        .cf_handle("chain_metadata")
        .ok_or("chain_metadata CF not found")?;

    let height_key = height.to_le_bytes().to_vec();
    let hash_bytes = hex::decode(&block_hash)?;

    // Store height -> hash mapping
    db.put_cf(&cf_metadata, &height_key, &hash_bytes)?;

    // Store hash -> height mapping for reverse lookups. The 'h' key MUST be the
    // INTERNAL (little-endian) hash to match the parse path (blocks.rs/offset_indexer)
    // and every 'h' reader (enrichment orphan classifier, reorg cleanup). Writing it
    // in display order (as this did previously) made live-monitored canonical blocks
    // invisible to those readers — they were miscounted as orphans, which corrupted
    // the orphan-rate / daily-series analytics. The forward height->hash map above
    // stays display order (consistent with blocks.rs).
    let internal_hash: Vec<u8> = hash_bytes.iter().rev().cloned().collect();
    let mut hash_key = vec![b'h'];
    hash_key.extend_from_slice(&internal_hash);
    let height_bytes = height.to_le_bytes().to_vec();
    db.put_cf(&cf_metadata, &hash_key, &height_bytes)?;

    // Store block header in blocks CF
    // Create a minimal header with the data we have from RPC
    let cf_blocks = db.cf_handle("blocks").ok_or("blocks CF not found")?;

    // Parse block header fields from RPC result
    let time = result.get("time").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let nonce = result.get("nonce").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let bits = result
        .get("bits")
        .and_then(|v| v.as_str())
        .unwrap_or("00000000");
    let merkleroot = result
        .get("merkleroot")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let previousblockhash = result
        .get("previousblockhash")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Build a minimal 80-byte block header
    let mut header = Vec::with_capacity(80);
    header.extend(&version.to_le_bytes()); // 4 bytes: version

    // 32 bytes: previous block hash
    if !previousblockhash.is_empty() {
        if let Ok(prev_hash) = hex::decode(previousblockhash) {
            let prev_internal: Vec<u8> = prev_hash.iter().rev().cloned().collect();
            header.extend(&prev_internal);
        } else {
            header.extend(&[0u8; 32]);
        }
    } else {
        header.extend(&[0u8; 32]); // Genesis block
    }

    // 32 bytes: merkle root
    if !merkleroot.is_empty() {
        if let Ok(merkle) = hex::decode(merkleroot) {
            let merkle_internal: Vec<u8> = merkle.iter().rev().cloned().collect();
            header.extend(&merkle_internal);
        } else {
            header.extend(&[0u8; 32]);
        }
    } else {
        header.extend(&[0u8; 32]);
    }

    header.extend(&time.to_le_bytes()); // 4 bytes: time

    // 4 bytes: bits
    if let Ok(bits_val) = u32::from_str_radix(bits, 16) {
        header.extend(&bits_val.to_le_bytes());
    } else {
        header.extend(&[0u8; 4]);
    }

    header.extend(&nonce.to_le_bytes()); // 4 bytes: nonce

    // Store the header (key is internal format hash, value is header)
    let internal_hash: Vec<u8> = hash_bytes.iter().rev().cloned().collect();
    db.put_cf(&cf_blocks, &internal_hash, &header)?;

    // Index all transactions from this block
    let cf_transactions = db
        .cf_handle("transactions")
        .ok_or("transactions CF not found")?;

    let mut tx_count = 0;
    let mut tx_errors = 0;
    // Address-index undo for this block, captured INLINE as we apply each r/s/a/t
    // write below. Recording exactly what we write (instead of recomputing from the
    // block afterwards) reverses r/s exactly and restores the spent UTXOs' fields
    // (set/value-exact; 'a'/'t' insertion order may differ from a fresh enrich after a
    // reorg, harmless) — and adds zero extra DB reads/deserializes. Stored
    // after the loop, only for live blocks past the enrichment watermark. See
    // address_rollback::reverse_address_block for the consumer.
    let mut block_undo = crate::address_rollback::AddressBlockUndo::new(height);

    for (tx_index, tx_val) in tx_array.iter().enumerate() {
        // With verbosity=2, tx_val could be:
        // - A string (just txid) - older PIVX versions
        // - An object (full transaction data) - newer versions

        let txid = if let Some(txid_str) = tx_val.as_str() {
            // Old format: just a txid string
            txid_str.to_string()
        } else if let Some(tx_obj) = tx_val.as_object() {
            // New format: full transaction object
            tx_obj
                .get("txid")
                .and_then(|t| t.as_str())
                .ok_or("Missing txid in transaction object")?
                .to_string()
        } else {
            warn!(
                tx_index = tx_index,
                height = height,
                "Skipping invalid transaction"
            );
            tx_errors += 1;
            continue;
        };

        let txid_bytes = match hex::decode(&txid) {
            Ok(bytes) => bytes,
            Err(_) => {
                warn!(tx_index = tx_index, height = height, txid = %txid, "Invalid txid hex");
                tx_errors += 1;
                continue;
            }
        };

        // Attempt to get raw transaction data
        let raw_tx_bytes = if let Some(tx_obj) = tx_val.as_object() {
            // Try to get 'hex' field from transaction object (verbosity=2)
            if let Some(hex_str) = tx_obj.get("hex").and_then(|h| h.as_str()) {
                match hex::decode(hex_str) {
                    Ok(bytes) => Some(bytes),
                    Err(_) => {
                        warn!(txid = %txid, "Failed to decode hex for transaction");
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        // If we don't have raw bytes from verbosity=2, fetch individually
        let raw_tx_bytes = if let Some(bytes) = raw_tx_bytes {
            bytes
        } else {
            // Fallback: Fetch individual transaction
            match client
                .post(&url)
                .basic_auth(&user, Some(&pass))
                .json(&serde_json::json!({
                    "jsonrpc": "1.0",
                    "id": "rustyblox",
                    "method": "getrawtransaction",
                    "params": [&txid, 0]  // 0 = raw hex, not JSON
                }))
                .send()
                .await
            {
                Ok(tx_resp) => match tx_resp.json::<Value>().await {
                    Ok(tx_json) => {
                        if let Some(raw_hex) = tx_json.get("result").and_then(|r| r.as_str()) {
                            match hex::decode(raw_hex) {
                                Ok(bytes) => bytes,
                                Err(_) => {
                                    warn!(txid = %txid, "Failed to decode getrawtransaction result");
                                    tx_errors += 1;
                                    continue;
                                }
                            }
                        } else {
                            warn!(txid = %txid, "No result in getrawtransaction response");
                            tx_errors += 1;
                            continue;
                        }
                    }
                    Err(e) => {
                        warn!(txid = %txid, error = %e, "Failed to parse getrawtransaction response");
                        tx_errors += 1;
                        continue;
                    }
                },
                Err(e) => {
                    warn!(txid = %txid, error = %e, "Failed to fetch transaction via RPC");
                    tx_errors += 1;
                    continue;
                }
            }
        };

        // 1. Store full transaction: 't' + txid → (tx_version + height + raw_tx)
        // Database uses DISPLAY format for txid keys (same as Bitcoin Core's internal format)
        // RPC returns txid in display format, use it directly (do NOT reverse)
        let mut tx_key = vec![b't'];
        tx_key.extend_from_slice(&txid_bytes);

        // CRITICAL FIX: Check if transaction already exists and update height if needed
        // This handles the case where a transaction was added to mempool (height=-1)
        // and later confirmed in a block (needs height update)
        let existing_tx_data = db.get_cf(&cf_transactions, &tx_key)?;

        let needs_update = if let Some(existing_data) = &existing_tx_data {
            if existing_data.len() >= 8 {
                let existing_height = i32::from_le_bytes([
                    existing_data[4],
                    existing_data[5],
                    existing_data[6],
                    existing_data[7],
                ]);
                // Update if existing is unconfirmed (-1) or unresolved (-2)
                existing_height < 0
            } else {
                // Invalid existing data, overwrite
                true
            }
        } else {
            // New transaction
            true
        };

        // Extract transaction version from raw_tx_bytes (first 4 bytes)
        let tx_version_bytes = if raw_tx_bytes.len() >= 4 {
            &raw_tx_bytes[0..4]
        } else {
            warn!(txid = %txid, "Transaction has invalid size (< 4 bytes), using default version");
            &[1u8, 0, 0, 0] // Default to version 1
        };

        let mut full_data = tx_version_bytes.to_vec();
        full_data.extend(&height.to_le_bytes());
        full_data.extend(&raw_tx_bytes);

        if needs_update {
            if let Err(e) = db.put_cf(&cf_transactions, &tx_key, &full_data) {
                warn!(txid = %txid, error = %e, "Failed to store transaction");
                tx_errors += 1;
                continue;
            }

            if existing_tx_data.is_some() {
                debug!(txid = %txid, height = height, "Promoted mempool tx to confirmed");
            }
        }

        // 2. Block transaction index: 'B' + height + tx_index → txid
        let mut block_tx_key = vec![b'B'];
        block_tx_key.extend(&height.to_le_bytes());
        block_tx_key.extend(&(tx_index as u64).to_le_bytes());

        // Store the txid in display format
        if let Err(e) = db.put_cf(&cf_transactions, &block_tx_key, txid.as_bytes()) {
            warn!(txid = %txid, error = %e, "Failed to store block TX index");
            tx_errors += 1;
            continue;
        }

        // 3. Parse transaction and index addresses/UTXOs
        // Validate transaction size before allocation
        if raw_tx_bytes.len() > 10_000_000 {
            warn!(txid = %txid, bytes = raw_tx_bytes.len(), "Transaction too large, skipping");
            tx_errors += 1;
            continue;
        }

        if raw_tx_bytes.is_empty() {
            warn!(txid = %txid, "Transaction has empty data, skipping");
            tx_errors += 1;
            continue;
        }

        // Prepend dummy block_version for parser compatibility
        // Use safe checked addition to prevent overflow
        let total_size = match 4_usize.checked_add(raw_tx_bytes.len()) {
            Some(size) if size <= 10_000_004 => size,
            _ => {
                warn!(txid = %txid, "Transaction size calculation overflow, skipping");
                tx_errors += 1;
                continue;
            }
        };
        let mut tx_data_with_header = Vec::with_capacity(total_size);
        tx_data_with_header.extend_from_slice(&[0u8; 4]);
        tx_data_with_header.extend_from_slice(&raw_tx_bytes);

        // Parse the transaction
        use crate::parser::deserialize_transaction;

        let parsed_tx = match deserialize_transaction(&tx_data_with_header).await {
            Ok(tx) => tx,
            Err(e) => {
                warn!(txid = %txid, error = %e, "Failed to parse transaction");
                tx_errors += 1;
                continue;
            }
        };

        // Track Sapling transactions for metrics
        if parsed_tx.sapling_data.is_some() {
            metrics::increment_sapling_transactions(1);
        }

        // Index addresses from outputs
        // CRITICAL FIX: Only modify address index for NEW blocks (after enrichment height)
        // Blocks processed by enrichment already have correct address index data
        let cf_addr_index = db
            .cf_handle("addr_index")
            .ok_or("addr_index CF not found")?;

        // txid_bytes is already in display format (no reversal needed)
        // Database keys and UTXO storage use display format consistently

        // Track which addresses are involved in this transaction (for tx history)
        let mut involved_addresses = std::collections::HashSet::new();

        // Add outputs as UTXOs ONLY for new blocks
        if should_update_address_index {
            // Source-tx type once per tx — the SAME byte enrich stores as packed.ty,
            // so catch-up 'a' records are byte-identical to enrich.
            let tx_kind =
                crate::tx_type::ty_to_u8(crate::tx_type::detect_transaction_type(&parsed_tx));
            for (output_idx, output) in parsed_tx.outputs.iter().enumerate() {
                for address in &output.address {
                    if address.is_empty() {
                        continue;
                    }

                    // Track address for transaction history
                    involved_addresses.insert(address.clone());

                    // TWO-PHASE OPTIMIZATION: Skip outputs that are spent within this batch
                    // These are "born and die" outputs that should never appear in UTXO set
                    if let Some(spent) = spent_set {
                        if spent.contains(&(txid_bytes.clone(), output_idx as u64)) {
                            // Output is spent in this batch - don't add it to UTXO set
                            continue;
                        }
                    }

                    // Key format: 'a' + address (UTXOs)
                    let mut addr_key = vec![b'a'];
                    addr_key.extend_from_slice(address.as_bytes());

                    // No size cap: enrich writes 'a' UNCAPPED, so a >100k-UTXO address
                    // (exchange/treasury) is legitimate and must round-trip. On an
                    // unreadable blob (legacy/corrupt — never a v2 blob), SKIP this
                    // address: never overwrite a non-empty-but-unreadable record with a
                    // one-entry list, which would destroy the entire UTXO set (MBX-1).
                    let mut existing_utxos = match db.get_cf(&cf_addr_index, &addr_key)? {
                        Some(data) => match crate::parser::deserialize_addr_utxos(&data).await {
                            Ok(u) => u,
                            Err(e) => {
                                warn!(address = %address, error = %e, "Invalid 'a' record; skipping (not overwriting)");
                                continue;
                            }
                        },
                        None => Vec::new(),
                    };

                    // CRITICAL FIX: Check if UTXO already exists (idempotent)
                    let already_exists = existing_utxos
                        .iter()
                        .any(|(t, i, _, _)| t == &txid_bytes && *i == output_idx as u64);

                    if already_exists {
                        // Already indexed - skip (prevents duplicates from reorg/reindex)
                        #[cfg(feature = "debug-address-index")]
                        debug!(txid = %hex::encode(&txid_bytes), vout = output_idx, %address,
                               "Duplicate UTXO detected (skipped)");
                        continue;
                    }

                    // Add the new v2 49-byte UTXO record (txid+vout+value+kind).
                    existing_utxos.push((
                        txid_bytes.clone(),
                        output_idx as u64,
                        output.value,
                        tx_kind,
                    ));
                    // Reorg-undo capture records the created (txid,vout) only.
                    block_undo.add_utxo_created(
                        address.clone(),
                        txid_bytes.clone(),
                        output_idx as u64,
                    );

                    // Store updated UTXO list
                    let serialized = crate::parser::serialize_addr_utxos(&existing_utxos).await;
                    if let Err(e) = db.put_cf(&cf_addr_index, &addr_key, &serialized) {
                        warn!(address = %address, txid = %txid, error = %e, "Failed to index address for tx");
                    }
                }
            }
        }

        // Process inputs: Remove spent UTXOs from address index
        // TWO-PHASE OPTIMIZATION: If spent_set is provided (from Phase 1),
        // we can skip spend removal for outputs that aren't in the spent set.
        // This is much faster and more reliable than on-demand RPC fetching.
        for input in &parsed_tx.inputs {
            // Skip coinbase transactions (no prevout)
            let prevout = match &input.prevout {
                Some(p) => p,
                None => continue,
            };

            // Skip if coinbase (indicated by coinbase field)
            if input.coinbase.is_some() {
                continue;
            }

            // Get previous txid and output index
            let prev_txid_hex = &prevout.hash;
            let prev_output_idx = prevout.n;

            // Decode the previous txid from hex string to bytes (display format)
            let prev_txid_bytes = match hex::decode(prev_txid_hex) {
                Ok(bytes) => bytes,
                Err(_) => {
                    warn!(prev_txid = %prev_txid_hex, "Invalid prev txid hex");
                    continue;
                }
            };

            // Get the previous transaction (prevout.hash is already in display format)
            let mut prev_tx_key = vec![b't'];
            prev_tx_key.extend_from_slice(&prev_txid_bytes);

            // Try to get from database first
            let prev_tx_data_opt = db.get_cf(&cf_transactions, &prev_tx_key)?;

            // If not in database, fetch from RPC and store it
            let prev_tx_data = if let Some(data) = prev_tx_data_opt {
                data
            } else {
                // Previous transaction not in DB - need to fetch it
                // TWO-PHASE NOTE: This should be rare if blocks are processed in order
                // Debug logging removed for performance - only log on errors
                match client
                    .post(&url)
                    .basic_auth(&user, Some(&pass))
                    .json(&serde_json::json!({
                        "jsonrpc": "1.0",
                        "id": "rustyblox",
                        "method": "getrawtransaction",
                        "params": [prev_txid_hex, 1]  // 1 = verbose (includes blockhash)
                    }))
                    .send()
                    .await
                {
                    Ok(resp) => {
                        match resp.json::<Value>().await {
                            Ok(json) => {
                                if let Some(result) = json.get("result") {
                                    // Extract hex and blockhash
                                    let raw_hex = result.get("hex").and_then(|h| h.as_str());
                                    let blockhash =
                                        result.get("blockhash").and_then(|h| h.as_str());

                                    if let (Some(hex_str), Some(block_hash)) = (raw_hex, blockhash)
                                    {
                                        match hex::decode(hex_str) {
                                            Ok(raw_bytes) => {
                                                // Fetch block height for this blockhash
                                                let prev_height = match client
                                                    .post(&url)
                                                    .basic_auth(&user, Some(&pass))
                                                    .json(&serde_json::json!({
                                                        "jsonrpc": "1.0",
                                                        "id": "rustyblox",
                                                        "method": "getblock",
                                                        "params": [block_hash, 1]
                                                    }))
                                                    .send()
                                                    .await
                                                {
                                                    Ok(block_resp) => block_resp
                                                        .json::<Value>()
                                                        .await
                                                        .ok()
                                                        .and_then(|j| {
                                                            j.get("result")
                                                                .and_then(|r| r.get("height"))
                                                                .and_then(|h| h.as_i64())
                                                        })
                                                        .unwrap_or(0)
                                                        as i32,
                                                    Err(_) => 0,
                                                };

                                                // Store this transaction with proper height
                                                // Extract tx version from raw bytes (first 4 bytes)
                                                let tx_version_bytes = if raw_bytes.len() >= 4 {
                                                    &raw_bytes[0..4]
                                                } else {
                                                    &[1u8, 0, 0, 0] // Default to version 1
                                                };

                                                let mut full_data = tx_version_bytes.to_vec();
                                                full_data.extend(&prev_height.to_le_bytes());
                                                full_data.extend(&raw_bytes);

                                                if let Err(e) = db.put_cf(
                                                    &cf_transactions,
                                                    &prev_tx_key,
                                                    &full_data,
                                                ) {
                                                    warn!(prev_txid = %prev_txid_hex, error = %e, "Failed to cache previous tx");
                                                }
                                                // Successfully cached - debug logging removed for performance

                                                // Also store the 'B' index entry for this transaction
                                                if prev_height > 0 {
                                                    // We don't know the tx_index within the block, so skip 'B' entry
                                                    // The transaction will still be queryable via 't' prefix
                                                }

                                                full_data
                                            }
                                            Err(_) => {
                                                warn!(prev_txid = %prev_txid_hex, "Failed to decode previous tx hex");
                                                continue;
                                            }
                                        }
                                    } else {
                                        warn!(prev_txid = %prev_txid_hex, "Missing hex or blockhash for previous tx");
                                        continue;
                                    }
                                } else {
                                    warn!(prev_txid = %prev_txid_hex, "No result for previous tx in RPC response");
                                    continue;
                                }
                            }
                            Err(_) => {
                                warn!(prev_txid = %prev_txid_hex, "Failed to parse RPC response for previous tx");
                                continue;
                            }
                        }
                    }
                    Err(_) => {
                        // RPC fetch failed - can't process this input
                        // This is non-fatal, just skip removing from UTXO set
                        debug!(prev_txid = %prev_txid_hex, "RPC request failed for previous tx (non-fatal)");
                        continue;
                    }
                }
            };

            // Now process the previous transaction data
            if prev_tx_data.len() < 8 {
                warn!(prev_txid = %prev_txid_hex, "Previous transaction data too short");
                continue;
            }

            if prev_tx_data.len() > 10_000_008 {
                // 10MB + 8 byte header
                warn!(prev_txid = %prev_txid_hex, bytes = prev_tx_data.len(), "Previous transaction too large");
                continue;
            }

            // Format: version (4) + height (4) + raw_tx
            // We need to prepend 4-byte dummy header for parser, so extract raw tx part
            let raw_prev_tx = &prev_tx_data[8..];

            if raw_prev_tx.is_empty() {
                warn!(prev_txid = %prev_txid_hex, "Previous transaction has empty data");
                continue;
            }

            // Additional safety check: ensure raw_prev_tx length is reasonable
            if raw_prev_tx.len() > 10_000_000 {
                warn!(prev_txid = %prev_txid_hex, bytes = raw_prev_tx.len(), "Previous transaction raw data too large");
                continue;
            }

            // Prepend dummy block_version for parser compatibility
            // Use safe checked addition to prevent overflow
            let total_size = match 4_usize.checked_add(raw_prev_tx.len()) {
                Some(size) if size <= 10_000_004 => size,
                _ => {
                    warn!(prev_txid = %prev_txid_hex, "Previous transaction total size overflow or too large");
                    continue;
                }
            };
            let mut prev_tx_with_header = Vec::with_capacity(total_size);
            prev_tx_with_header.extend_from_slice(&[0u8; 4]);
            prev_tx_with_header.extend_from_slice(raw_prev_tx);

            // Parse previous transaction to find addresses of the spent output
            if let Ok(prev_tx) = deserialize_transaction(&prev_tx_with_header).await {
                // Get the output at this index
                if let Some(output) = prev_tx.outputs.get(prev_output_idx as usize) {
                    // Remove from address index for each address in this output
                    for address in &output.address {
                        if address.is_empty() {
                            continue;
                        }

                        // Track address for transaction history (inputs spend from this address)
                        involved_addresses.insert(address.clone());

                        let mut addr_key = vec![b'a'];
                        addr_key.extend_from_slice(address.as_bytes());

                        // Get existing UTXOs. No size cap (enrich is uncapped). On an
                        // unreadable blob, SKIP this spend — never delete_cf / overwrite
                        // a record that failed to read (MBX-1: that would wipe the set).
                        let existing_utxos = match db.get_cf(&cf_addr_index, &addr_key)? {
                            Some(data) => {
                                match crate::parser::deserialize_addr_utxos(&data).await {
                                    Ok(u) => u,
                                    Err(e) => {
                                        warn!(address = %address, error = %e, "Invalid 'a' record; skipping (not deleting)");
                                        continue;
                                    }
                                }
                            }
                            None => Vec::new(),
                        };

                        // Reorg-undo capture: only record the spend if the UTXO was
                        // actually present in 'a' (a born-and-die output never entered
                        // it). Capture value+kind from the matched 49-byte record so a
                        // reorg disconnect restores it byte-exactly.
                        if let Some((_, _, value, kind)) =
                            existing_utxos
                                .iter()
                                .find(|(stored_txid, stored_idx, _, _)| {
                                    stored_txid == &prev_txid_bytes
                                        && *stored_idx == prev_output_idx as u64
                                })
                        {
                            block_undo.add_utxo_spent(
                                address.clone(),
                                prev_txid_bytes.clone(),
                                prev_output_idx as u64,
                                *value,
                                *kind,
                            );
                        }

                        // Remove the spent UTXO (match by txid and index)
                        let updated_utxos: Vec<_> = existing_utxos
                            .into_iter()
                            .filter(|(stored_txid, stored_idx, _, _)| {
                                !(stored_txid == &prev_txid_bytes
                                    && *stored_idx == prev_output_idx as u64)
                            })
                            .collect();

                        // Update or delete
                        if !updated_utxos.is_empty() {
                            let serialized =
                                crate::parser::serialize_addr_utxos(&updated_utxos).await;
                            let _ = db.put_cf(&cf_addr_index, &addr_key, &serialized);
                        } else {
                            let _ = db.delete_cf(&cf_addr_index, &addr_key);
                        }
                    }
                }
            }
        }

        // Add this transaction to all involved addresses' transaction lists
        // Only for new blocks to avoid duplicates
        if should_update_address_index {
            // Key format: 't' + address for transaction history
            for address in &involved_addresses {
                let mut tx_list_key = vec![b't'];
                tx_list_key.extend_from_slice(address.as_bytes());

                // Get existing transaction list
                let mut tx_list = match db.get_cf(&cf_addr_index, &tx_list_key)? {
                    Some(data) => match crate::parser::deserialize_addr_txs(&data).await {
                        Ok(list) => list,
                        Err(e) => {
                            // MBX-1 (same as the 'a' paths): an unreadable 't' blob must
                            // NOT be overwritten with a one-entry list — 't' is the
                            // non-reconstructable tx history. Skip this address.
                            warn!(address = %address, error = %e, "Invalid 't' record; skipping (not overwriting)");
                            continue;
                        }
                    },
                    None => Vec::new(),
                };

                // Add this transaction if not already present. Compare the 32-byte txid
                // against each 36-byte record's txid field (a 36B record can never equal
                // a 32B txid — the dedup trap that would otherwise append unboundedly).
                if !tx_list.iter().any(|(t, _)| t == &txid_bytes) {
                    tx_list.push((txid_bytes.clone(), height));
                    // Reorg-undo capture: record exactly the txid we add to 't'.
                    block_undo.add_tx(address.clone(), txid_bytes.clone());

                    // Serialize the v2 36-byte 't' records (txid + inline height).
                    let serialized = crate::parser::serialize_addr_txs(&tx_list).await;

                    if let Err(e) = db.put_cf(&cf_addr_index, &tx_list_key, &serialized) {
                        warn!(address = %address, error = %e, "Failed to update tx list for address");
                    }
                }
            }

            // CRITICAL FIX: Update total_received and total_sent for involved addresses
            // This ensures balances stay correct during RPC catchup
            for address in &involved_addresses {
                // Calculate received amount (outputs to this address)
                let mut received_delta: i64 = 0;
                for output in &parsed_tx.outputs {
                    if output.address.contains(address) {
                        received_delta += output.value;
                    }
                }

                // Update total_received ('r' + address)
                if received_delta > 0 {
                    let mut key_r = vec![b'r'];
                    key_r.extend_from_slice(address.as_bytes());

                    let current_total = db
                        .get_cf(&cf_addr_index, &key_r)?
                        .and_then(|bytes| {
                            if bytes.len() == 8 {
                                Some(i64::from_le_bytes([
                                    bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5],
                                    bytes[6], bytes[7],
                                ]))
                            } else {
                                None
                            }
                        })
                        .unwrap_or(0);

                    let new_total = current_total + received_delta;
                    db.put_cf(&cf_addr_index, &key_r, new_total.to_le_bytes())?;
                    // Reorg-undo capture: record exactly the amount we add to 'r'.
                    block_undo.add_received(address.clone(), received_delta);
                }

                // Calculate sent amount (inputs spending from this address)
                let mut sent_delta: i64 = 0;
                for input in &parsed_tx.inputs {
                    if input.coinbase.is_some() {
                        continue;
                    }
                    if let Some(prevout) = &input.prevout {
                        let prev_txid_hex = &prevout.hash;
                        let prev_output_idx = prevout.n;

                        // Decode previous txid
                        if let Ok(prev_txid_bytes) = hex::decode(prev_txid_hex) {
                            let mut prev_tx_key = vec![b't'];
                            prev_tx_key.extend_from_slice(&prev_txid_bytes);

                            // Get previous transaction
                            if let Some(prev_tx_data) = db.get_cf(&cf_transactions, &prev_tx_key)? {
                                if prev_tx_data.len() >= 8 {
                                    let prev_raw_tx = &prev_tx_data[8..];
                                    let mut prev_tx_with_header =
                                        Vec::with_capacity(4 + prev_raw_tx.len());
                                    prev_tx_with_header.extend_from_slice(&[0u8; 4]);
                                    prev_tx_with_header.extend_from_slice(prev_raw_tx);

                                    if let Ok(prev_tx) =
                                        deserialize_transaction(&prev_tx_with_header).await
                                    {
                                        if let Some(prev_output) =
                                            prev_tx.outputs.get(prev_output_idx as usize)
                                        {
                                            if prev_output.address.contains(address) {
                                                sent_delta += prev_output.value;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Update total_sent ('s' + address)
                if sent_delta > 0 {
                    let mut key_s = vec![b's'];
                    key_s.extend_from_slice(address.as_bytes());

                    let current_total = db
                        .get_cf(&cf_addr_index, &key_s)?
                        .and_then(|bytes| {
                            if bytes.len() == 8 {
                                Some(i64::from_le_bytes([
                                    bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5],
                                    bytes[6], bytes[7],
                                ]))
                            } else {
                                None
                            }
                        })
                        .unwrap_or(0);

                    let new_total = current_total + sent_delta;
                    db.put_cf(&cf_addr_index, &key_s, new_total.to_le_bytes())?;
                    // Reorg-undo capture: record exactly the amount we add to 's'.
                    block_undo.add_sent(address.clone(), sent_delta);
                }
            }
        }

        tx_count += 1;
    }

    // Report any errors
    if tx_errors > 0 {
        warn!(
            height = height,
            indexed = tx_count,
            total = tx_array.len(),
            errors = tx_errors,
            "Block indexed with errors"
        );
    } else if height % 1000 == 0 {
        debug!(height = height, tx_count = tx_count, "Block indexed");
    }
    // Debug logging reduced for performance - only log every 1000 blocks or on errors

    // Persist the inline-captured address-index undo so a future chain reorg can
    // reverse r/s/a/t exactly (rollback_address_index -> reverse_address_block).
    // Only for blocks this monitor actually indexed (live blocks past the enrichment
    // watermark); enrichment-owned blocks are repaired by re-enrichment, not reorg
    // undo, and reorgs only ever hit the recent live tip. Stored before the
    // sync_height/H-marker writes so a block marked done always has its undo.
    if should_update_address_index {
        if let Err(e) = crate::address_rollback::store_address_undo(db.clone(), &block_undo).await {
            warn!(height, error = %e,
                "Failed to store address undo; reorg r/s reversal degraded for this block");
        }
        // Prune undo older than the reorg window so chain_state does not grow
        // unbounded. PIVX PoS reorgs are shallow; 200 blocks is a generous margin.
        // Run unconditionally for live heights (not gated on this block having any
        // captured deltas) so a coinbase-only/empty block can't leak its
        // predecessor's record.
        // Single source of truth shared with the analytics recompute's reorg gate,
        // so the prune horizon and the "index may be reorg-stale" threshold can't drift.
        let prune_below = height - crate::analytics_recompute::ADDR_INDEX_UNDO_WINDOW;
        if prune_below > 0 {
            if let Err(e) =
                crate::address_rollback::delete_address_undo(db.clone(), prune_below).await
            {
                debug!(height = prune_below, error = %e, "Failed to prune old address undo");
            }
        }
    }

    // Update sync height
    let cf_state = db
        .cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;

    db.put_cf(&cf_state, b"sync_height", height.to_le_bytes())?;

    // CRITICAL FIX: Store block hash for deduplication
    let mut height_hash_key = vec![b'H'];
    height_hash_key.extend(&height.to_le_bytes());
    db.put_cf(&cf_state, &height_hash_key, block_hash.as_bytes())?;

    // Update indexed height metric
    metrics::set_indexed_height("rpc_monitor", height as i64);

    // Processing marker will be cleaned up automatically by the guard's Drop impl

    // Broadcast new block event if broadcaster is available
    if let Some(bc) = broadcaster {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or(std::time::Duration::from_secs(0))
            .as_secs();
        bc.broadcast_block(height, block_hash, timestamp, tx_count);
    }

    Ok(())
}

/// Update aggregate database metrics (address count, UTXO count, Sapling tx count)
/// Should be called periodically during monitoring to keep metrics up to date
fn update_aggregate_metrics(db: &Arc<DB>) -> Result<(), Box<dyn std::error::Error>> {
    let cf_addr_index = db
        .cf_handle("addr_index")
        .ok_or("addr_index CF not found")?;

    // Count unique addresses and their unspent UTXOs
    // In addr_index CF:
    //   - Keys starting with 'a' contain UTXO lists for addresses
    //   - Value format: repeating (txid(32) + vout(8)) for each UTXO
    let mut address_count: u64 = 0;
    let mut utxo_count: u64 = 0;
    let mut sample_checked = 0;

    let iter = db.iterator_cf(&cf_addr_index, rocksdb::IteratorMode::Start);
    for item in iter {
        if let Ok((key, value)) = item {
            if !key.is_empty() {
                let prefix = key[0];
                // Count 'a' (address) keys - these contain UTXO lists
                if prefix == b'a' {
                    address_count += 1;

                    // Count UTXOs for this address
                    // Each v2 'a' record is 49 bytes: txid(32)+vout(8)+value(8)+kind(1)
                    if !value.is_empty() {
                        if value.len() % crate::parser::ADDR_UTXO_STRIDE == 0 {
                            let utxos_for_address = value.len() / crate::parser::ADDR_UTXO_STRIDE;
                            utxo_count += utxos_for_address as u64;
                        } else if sample_checked < 5 {
                            // Log first few addresses with unexpected format for debugging
                            debug!(
                                address_key_len = key.len(),
                                value_len = value.len(),
                                "Address value length not multiple of 49"
                            );
                            sample_checked += 1;
                        }
                    }
                }
            }
        }
    }

    debug!(
        addresses = address_count,
        utxos = utxo_count,
        "Aggregate metrics updated from database scan"
    );

    // Update metrics
    metrics::set_total_addresses_indexed(address_count);
    metrics::set_total_utxos_tracked(utxo_count);

    // NOTE: We don't update SAPLING_TRANSACTIONS_COUNT here because:
    // 1. SAPLING_TRANSACTIONS_TOTAL (counter) is incremented as new txs are processed
    // 2. SAPLING_TRANSACTIONS_COUNT (gauge) is set during enrichment phase
    // 3. Counting from DB requires full parsing which is too expensive for periodic updates
    // The counter (TOTAL) gives us the cumulative count which is what we need for monitoring

    Ok(())
}

/// Detect if reorg occurred
fn detect_reorg(
    db_tip: &ChainTip,
    rpc_tip: &ChainTip,
) -> Result<Option<i32>, Box<dyn std::error::Error>> {
    // If RPC height is less than ours, definite reorg
    if rpc_tip.height < db_tip.height {
        warn!(
            rpc_height = rpc_tip.height,
            db_height = db_tip.height,
            "REORG DETECTED: RPC height less than DB height"
        );
        return Ok(Some(rpc_tip.height));
    }

    // If heights are same, check hashes
    if rpc_tip.height == db_tip.height && rpc_tip.hash != db_tip.hash {
        warn!(
            height = rpc_tip.height,
            "REORG DETECTED: Hash mismatch at tip"
        );
        return Ok(Some(rpc_tip.height - 1));
    }

    // TODO: More sophisticated reorg detection
    // - Compare hashes at previous heights
    // - Find common ancestor

    Ok(None)
}

/// One-shot startup self-heal: a leftover 'P' (processing) marker means a previous
/// run died mid-block-connect (a hard crash skips the RAII guard that clears it).
/// Re-apply each such block (the 't'/'a' indexes are idempotent) and recompute r/s
/// for its addresses, since the r/s totals are NOT idempotent and re-applying would
/// otherwise leave them double-counted. No-op on a clean shutdown (no markers).
/// After crash recovery, never let `sync_height` regress below where it was before
/// recovery ran. A stale (below-tip) `'P'` marker re-applied via `index_block_from_rpc`
/// rewrites `sync_height` to that lower block height; left as-is, the monitor would
/// then re-catch-up — and on any bulk-synced range re-apply, double-counting r/s —
/// thousands of already-indexed blocks. The recovered blocks are all <= the
/// pre-recovery tip, so lifting the watermark back to `saved` is always safe.
/// Returns true if it restored.
fn restore_sync_height_if_regressed(db: &Arc<DB>, saved: i32) -> bool {
    let current = crate::chain_state::get_sync_height(db).unwrap_or(0);
    if current < saved {
        match crate::chain_state::set_sync_height(db, saved) {
            Ok(()) => true,
            Err(e) => {
                // Report the restore truthfully: if the write itself failed, the
                // watermark is still regressed (the caller must not log "restored").
                error!(error = %e, saved, "Failed to restore sync_height after crash recovery");
                false
            }
        }
    } else {
        false
    }
}

async fn recover_crashed_blocks(db: &Arc<DB>, broadcaster: &Option<Arc<EventBroadcaster>>) {
    let dirty = crate::crash_recovery::scan_processing_markers(db);
    if dirty.is_empty() {
        return;
    }
    warn!(count = dirty.len(), heights = ?dirty,
        "Leftover processing markers from a previous crash; recovering r/s for affected blocks");

    // Snapshot the committed watermark before recovery: re-applying a stale below-tip
    // marker rewrites sync_height to that lower height (index_block_from_rpc writes it
    // unconditionally), and that regression must be undone afterwards.
    let saved_height = crate::chain_state::get_sync_height(db).unwrap_or(0);

    let cf_state = match db.cf_handle("chain_state") {
        Some(cf) => cf,
        None => return,
    };
    for height in dirty {
        // Clear the stale marker so index_block_from_rpc's reservation check does not
        // skip the re-apply. (It sets and clears its own marker around processing.)
        let mut pkey = vec![b'P'];
        pkey.extend(&height.to_le_bytes());
        let _ = db.delete_cf(&cf_state, &pkey);

        // Re-apply to complete t/a/undo (double-counts the non-idempotent r/s). If
        // the block already finished before the crash (H-marker present), this is a
        // no-op skip — the recompute below is still idempotent and harmless.
        if let Err(e) = index_block_from_rpc(height, db, broadcaster, None).await {
            warn!(height, error = %e,
                "Failed to re-apply crashed block during recovery; leaving for normal catchup/reorg");
            continue;
        }

        // Recompute r/s for the block's addresses from the authoritative t/a indexes,
        // correcting any double-count.
        match crate::crash_recovery::repair_block_addresses_rs(db, height).await {
            Ok(n) => info!(
                height,
                addresses = n,
                "Recovered r/s after crash-interrupted block"
            ),
            Err(e) => {
                warn!(height, error = %e, "Failed to repair r/s after re-applying crashed block")
            }
        }
    }

    // Undo any watermark regression caused by re-applying a below-tip marker, so the
    // monitor doesn't re-catch-up an already-indexed range from the rolled-back height.
    if restore_sync_height_if_regressed(db, saved_height) {
        warn!(
            restored = saved_height,
            "Crash recovery re-applied a below-tip block; restored sync_height to the pre-recovery tip"
        );
    }
}

/// Main block monitoring loop
/// Backfill catch-up blocks that were stored heightless (tx height = -1).
///
/// When the blk-scan catch-up parses blocks newer than the canonical-metadata
/// (Core leveldb) refresh — because the freshly-(re)started node's on-disk block
/// index lagged its blk-file/RPC tip — those transactions are written heightless and
/// then orphaned out of the address index (0-tx block-detail, missing UTXOs, balances
/// behind by the gap). This re-processes `[from..=to]` through `index_block_from_rpc`,
/// which for each block, from the AUTHORITATIVE RPC block height:
///   1. writes the canonical 4-byte `height→hash` key into chain_metadata (~line 444) —
///      so the re-enrich's height resolver finds the height in `height_to_blockhash`
///      (`contains_key` true) and KEEPS it rather than orphaning it (height_resolver.rs
///      ~270); this is why the gap survives the re-enrich's height re-validation;
///   2. rewrites each tx's height via its `needs_update` path (existing height < 0) and
///      the `'B'` block→tx index.
/// The blk-parse never writes the `'H'` "already-indexed" marker, so the idempotency
/// early-return (~line 354) does not fire for these heightless blocks and the
/// chain_metadata write is reached. Run it BEFORE the catch-up re-enrich so the
/// re-enrich then includes the now-resolved, chain_metadata-backed range.
///
/// Durability: this retries IN PLACE until every height in the range has its 4-byte
/// chain_metadata key (re-processing only the still-missing ones each pass — RPC heights
/// are authoritative and `index_block_from_rpc` is idempotent, so this converges
/// regardless of how stale Core's leveldb is). It returns `Ok` ONLY when the whole range
/// is verified keyed, and `Err` after `MAX_ATTEMPTS`. The caller just `?`s and, on `Err`,
/// aborts the catch-up WITHOUT clearing completion markers — leaving the prior (correct,
/// slightly-behind) index served and the gap to be re-detected on retry — rather than
/// clearing markers, which would re-orphan the gap against the still-stale chain_metadata
/// and re-mark it complete (the broken recovery the prior FE-1 attempt had).
///
/// Cost: one `getblockhash` + `getblock` round-trip per still-missing block.
pub(crate) async fn backfill_heightless_catchup_range(
    db: &Arc<DB>,
    broadcaster: &Option<Arc<EventBroadcaster>>,
    from: i32,
    to: i32,
) -> Result<(), Box<dyn std::error::Error>> {
    if from > to {
        return Ok(());
    }
    // Clear any stale 'P'+height processing markers in the range (left by a prior hard
    // kill between claiming a height at ~:376 and finishing it). Without this,
    // index_block_from_rpc's idempotency early-return (~:369) would skip the block BEFORE
    // writing its 4-byte chain_metadata height key. Safe: the backfill is single-threaded.
    let cf_state = db
        .cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    for h in from..=to {
        let mut pkey = vec![b'P'];
        pkey.extend_from_slice(&h.to_le_bytes());
        db.delete_cf(&cf_state, &pkey).ok();
    }
    let cf_meta = db
        .cf_handle("chain_metadata")
        .ok_or("chain_metadata CF not found")?;
    info!(
        from,
        to,
        blocks = to - from + 1,
        "Backfilling heightless catch-up blocks via RPC (canonical metadata lagged the blk-file tip)"
    );
    const MAX_ATTEMPTS: u32 = 5;
    for attempt in 1..=MAX_ATTEMPTS {
        // Process only heights that still lack their canonical 4-byte chain_metadata key.
        for h in from..=to {
            if db.get_cf(&cf_meta, h.to_le_bytes())?.is_some() {
                continue; // already keyed — idempotent re-run skips it
            }
            if let Err(e) = index_block_from_rpc(h, db, broadcaster, None).await {
                warn!(height = h, attempt, error = %e, "Backfill: block re-resolve failed, will retry");
            }
        }
        // Verify the OUTCOME (not Ok-counts): every height must now have its key.
        let mut missing = 0u32;
        for h in from..=to {
            if db.get_cf(&cf_meta, h.to_le_bytes())?.is_none() {
                missing += 1;
            }
        }
        if missing == 0 {
            info!(attempt, "Heightless catch-up backfill complete + verified");
            return Ok(());
        }
        warn!(
            attempt,
            missing, "Backfill pass incomplete; retrying the still-missing heights"
        );
    }
    Err(format!(
        "catch-up backfill could not re-key heightless range [{from}..={to}] after \
         {MAX_ATTEMPTS} attempts (RPC unreachable?); aborting the catch-up to retry"
    )
    .into())
}

pub async fn run_block_monitor(
    db: Arc<DB>,
    poll_interval_secs: u64,
    broadcaster: Option<Arc<EventBroadcaster>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let _span = info_span!("block_monitor", poll_interval_secs = poll_interval_secs).entered();
    info!("Starting block monitor");

    // Initialize RPC client
    let config = get_global_config();
    let rpc_host = config.get_string("rpc.host")?;
    let rpc_user = config.get_string("rpc.user")?;
    let rpc_pass = config.get_string("rpc.pass")?;

    // Construct the blocking RPC client + test connection on tokio's managed
    // blocking pool. PivxRpcClient uses reqwest::blocking (own runtime), so it
    // must not run on a tokio worker thread. spawn_blocking is bounded and
    // awaited (joined) — unlike the previous detached std::thread::spawn +
    // recv_timeout, which leaked the OS thread whenever the node was slow.
    let rpc_host_clone = rpc_host.clone();
    let connect = tokio::task::spawn_blocking(move || {
        let client =
            PivxRpcClient::new(rpc_host_clone, Some(rpc_user), Some(rpc_pass), 3, 10, 30000);

        // Test connection
        match client.getblockcount() {
            Ok(height) => Ok((Arc::new(client), height)),
            Err(e) => Err(e),
        }
    });

    let rpc_client = match tokio::time::timeout(std::time::Duration::from_secs(10), connect).await {
        Ok(Ok(Ok((client, height)))) => {
            info!(rpc_height = height, "RPC connection established");
            metrics::set_rpc_connected(true);

            // Connected successfully - store initial network height
            if let Err(e) = set_network_height(&db, height as i32) {
                error!(error = %e, "Failed to set initial network height");
            }
            client
        }
        Ok(Ok(Err(e))) => {
            error!(error = %e, "RPC connection failed - ensure PIVX node is running with RPC enabled");
            metrics::set_rpc_connected(false);
            // Return (instead of parking in a no-op sleep loop) so the sync-thread retry
            // loop (main.rs) re-drives the whole sync — re-detect, re-catch-up, reconnect —
            // with backoff, and the tip self-heals once RPC recovers rather than freezing.
            return Err(format!("RPC connection failed at monitor startup: {e}").into());
        }
        Ok(Err(e)) => {
            // spawn_blocking task panicked/was cancelled.
            error!(error = %e, "RPC connection task failed");
            metrics::set_rpc_connected(false);
            return Err(format!("RPC connection task failed at monitor startup: {e}").into());
        }
        Err(_) => {
            error!("RPC connection timed out");
            metrics::set_rpc_connected(false);
            return Err("RPC connection timed out at monitor startup".into());
        }
    };

    // One-shot crash recovery before any new work: repair r/s for a block that a
    // previous run died partway through applying (detected via a leftover 'P'
    // marker). Runs only when such a marker exists.
    recover_crashed_blocks(&db, &broadcaster).await;

    // Surface the live daily-analytics gate state at startup. It activates only
    // after a FULL enrich flips the ready gate; a degraded restart rebuild (zeroed
    // join fields) keeps it dark until a full re-enrich.
    if crate::analytics_live::is_enabled() {
        info!(
            shadow = crate::analytics_live::shadow_mode(),
            ready = crate::analytics_live::is_ready(&db),
            "Live daily-analytics enabled (active once the ready gate is set by a full enrich)"
        );
    }

    // One-shot Phase-0 shadow validator: if `sync.live_analytics_shadow_validate_days`
    // > 0, wait for the daily series to be built, then re-run Lane I/R over that many
    // recent days into the shadow keyspace and diff each complete day vs the full
    // enrich, logging the report. (Join-field mismatches vs a DEGRADED rebuild are
    // expected — run a FULL re-enrich for a true join comparison.)
    let validate_days = config
        .get_int("sync.live_analytics_shadow_validate_days")
        .unwrap_or(0);
    if validate_days > 0 {
        let db_v = Arc::clone(&db);
        // std::thread + current-thread runtime: shadow_validate holds a RocksDB
        // iterator across .await (so it is !Send), the same reason the daily-series
        // pass is detached this way.
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    warn!(error = %e, "shadow_validate: failed to build runtime");
                    return;
                }
            };
            rt.block_on(async move {
                for _ in 0..1440 {
                    let done = db_v
                        .cf_handle("chain_state")
                        .and_then(|cf| db_v.get_cf(&cf, b"analytics_complete").ok().flatten())
                        .map(|v| v.first() == Some(&1u8))
                        .unwrap_or(false);
                    if done {
                        break;
                    }
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
                match crate::analytics_live::shadow_validate(&db_v, validate_days).await {
                    Ok(report) => info!("LIVE-ANALYTICS SHADOW VALIDATE:\n{report}"),
                    Err(e) => warn!(error = %e, "live-analytics shadow_validate failed"),
                }
            });
        });
    }

    // Tier-3 hourly snapshot tracking (resumes from the persisted series).
    let mut last_snapshot_hour: u64 = read_last_snapshot_hour(&db);
    let mut snapshot_failure_warned = false;

    loop {
        // Hourly forward-only snapshot (mempool / masternodes / supply).
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let hour = now_secs / 3600;
        if now_secs > 0 && hour > last_snapshot_hour {
            match write_hourly_snapshot(&db, now_secs).await {
                Ok(()) => {
                    last_snapshot_hour = hour;
                    debug!(ts = now_secs, "Hourly analytics snapshot stored");
                }
                Err(e) => {
                    // Skip this hour's snapshot; warn only once to avoid spam.
                    if !snapshot_failure_warned {
                        warn!(error = %e, "Hourly analytics snapshot failed (skipping silently from now on)");
                        snapshot_failure_warned = true;
                    }
                    last_snapshot_hour = hour;
                }
            }
        }

        // Get current tips
        let rpc_tip = match get_rpc_chain_tip().await {
            Ok(tip) => {
                info!(
                    rpc_height = tip.height,
                    rpc_hash = %crate::telemetry::truncate_hex(&tip.hash, 16),
                    "RPC chain tip detected"
                );
                metrics::set_chain_tip_height("rpc", tip.height as i64);
                tip
            }
            Err(e) => {
                error!(error = %e, "Failed to get RPC tip");
                metrics::increment_rpc_errors("getblockcount", "connection");
                metrics::set_rpc_connected(false);
                tokio::time::sleep(Duration::from_secs(poll_interval_secs)).await;
                continue;
            }
        };

        // Update network height in database
        if let Err(e) = set_network_height(&db, rpc_tip.height) {
            error!(error = %e, "Failed to update network height");
        }

        let db_tip = match get_db_chain_tip(&db) {
            Ok(tip) => tip,
            Err(e) => {
                error!(error = %e, "Failed to get DB tip");
                tokio::time::sleep(Duration::from_secs(poll_interval_secs)).await;
                continue;
            }
        };

        // Check for reorg
        if let Some(_reorg_height) = detect_reorg(&db_tip, &rpc_tip)? {
            warn!("BLOCKCHAIN REORGANIZATION DETECTED");

            // Handle the reorg using our reorg module
            match reorg::handle_reorg(db.clone(), &rpc_client, db_tip.height, rpc_tip.height).await
            {
                Ok(reorg_info) => {
                    info!(
                        orphaned = reorg_info.orphaned_blocks,
                        fork_height = reorg_info.fork_height,
                        "Reorg handled successfully"
                    );

                    // Keep live daily-analytics correct across the rollback: clear
                    // the affected (>= fork date) day blobs + reset the watermark so
                    // the subsequent ticks rebuild them as the new chain is indexed.
                    crate::analytics_live::on_reorg(
                        &db,
                        reorg_info.fork_height,
                        reorg_info.orphaned_blocks,
                    );

                    // Pause the richlist/wealth recompute if this reorg is deeper
                    // than the addr_index undo window (r/s/a may be left
                    // un-reversed); a full re-enrich rebuilds the index + clears it.
                    crate::analytics_recompute::on_reorg(&db, reorg_info.orphaned_blocks);

                    // Continue to re-index from the rollback point
                    // The normal sync logic below will pick up from the new chain tip
                }
                Err(e) => {
                    error!(error = %e, retry_secs = poll_interval_secs, "Failed to handle reorg");
                    tokio::time::sleep(Duration::from_secs(poll_interval_secs)).await;
                    continue;
                }
            }

            // After reorg, immediately check for new blocks on the canonical chain
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }

        // Check if we're behind
        if rpc_tip.height > db_tip.height {
            let blocks_behind = rpc_tip.height - db_tip.height;

            let _catchup_span = info_span!(
                "rpc_catchup",
                current_height = db_tip.height,
                target_height = rpc_tip.height,
                blocks_behind = blocks_behind
            )
            .entered();

            info!(
                blocks_behind = blocks_behind,
                current_height = db_tip.height,
                target_height = rpc_tip.height,
                "RPC catchup needed"
            );
            metrics::set_blocks_behind_tip(blocks_behind as i64);

            info!(
                blocks_behind = blocks_behind,
                start = db_tip.height + 1,
                end = rpc_tip.height,
                "RPC catchup starting"
            );

            // TWO-PHASE RPC CATCHUP (matches initial sync two-pass algorithm)
            // This ensures 100% accurate spend detection without network dependency

            let start_height = db_tip.height + 1;
            let end_height = rpc_tip.height;

            // === PHASE 1: Fetch blocks and build complete spent set ===
            debug!(
                blocks = blocks_behind,
                "Phase 1: Fetching blocks and building spent set"
            );

            let mut fetched_blocks = Vec::new();
            let mut fetch_errors = 0;

            // Fetch blocks in parallel for network efficiency
            // Use semaphore to limit concurrency
            use std::sync::Arc as StdArc;
            use tokio::sync::Semaphore;

            const MAX_CONCURRENT_FETCH: usize = 10;
            let fetch_semaphore = StdArc::new(Semaphore::new(MAX_CONCURRENT_FETCH));

            let mut fetch_futures = Vec::new();
            for height in start_height..=end_height {
                let sem = fetch_semaphore.clone();

                fetch_futures.push(async move {
                    let _permit = sem.acquire().await.unwrap();
                    (height, fetch_block_data(height).await)
                });
            }

            // Wait for all fetches to complete
            let fetch_results = futures::future::join_all(fetch_futures).await;

            // Collect successfully fetched blocks
            for (height, result) in fetch_results {
                match result {
                    Ok(block) => fetched_blocks.push(block),
                    Err(e) => {
                        error!(height = height, error = %e, "Failed to fetch block");
                        metrics::increment_rpc_errors("getblock", "timeout");
                        fetch_errors += 1;
                    }
                }
            }

            if fetch_errors > 0 {
                warn!(
                    fetch_errors = fetch_errors,
                    retry_secs = poll_interval_secs,
                    "Fetch errors occurred, retrying"
                );
                tokio::time::sleep(Duration::from_secs(poll_interval_secs)).await;
                continue;
            }

            // Build complete spent set from all fetched blocks
            debug!(
                blocks = fetched_blocks.len(),
                "Building spent set from fetched blocks"
            );
            let spent_set = build_spent_set_from_blocks(&fetched_blocks);
            debug!(
                spent_outputs = spent_set.len(),
                "Phase 1 complete: spent set built"
            );

            // === PHASE 2: Sequential indexing with complete spent knowledge ===
            debug!("Phase 2: Indexing blocks with complete spent set");

            let mut indexed = 0;
            let mut index_errors = 0;

            // Process blocks sequentially in height order
            // This ensures:
            // - No race conditions
            // - Height N indexed before N+1
            // - Spend removal has complete knowledge
            for block in fetched_blocks {
                // Add canonical hash validation
                let cf_metadata = db
                    .cf_handle("chain_metadata")
                    .ok_or("chain_metadata CF not found")?;

                let height_key = block.height.to_le_bytes();

                // Check if we already have a canonical hash for this height
                if let Some(stored_hash) = db.get_cf(&cf_metadata, height_key)? {
                    let stored_hash_hex = hex::encode(&stored_hash);

                    if stored_hash_hex != block.block_hash {
                        warn!(height = block.height, db_hash = %stored_hash_hex, rpc_hash = %block.block_hash,
                              "REORG detected during catchup - aborting to trigger reorg handler");
                        break;
                    }
                }

                // Index this block with spent set available
                match index_block_from_rpc(block.height, &db, &broadcaster, Some(&spent_set)).await
                {
                    Ok(_) => {
                        indexed += 1;
                        if indexed % 100 == 0 {
                            let progress = (indexed as f64 / blocks_behind as f64) * 100.0;
                            debug!(
                                indexed = indexed,
                                total = blocks_behind,
                                progress_pct = format!("{:.1}", progress),
                                "Catchup progress"
                            );
                        }
                    }
                    Err(e) => {
                        error!(height = block.height, error = %e, "Failed to index block");
                        index_errors += 1;
                    }
                }
            }

            info!(
                indexed = indexed,
                total = blocks_behind,
                errors = index_errors,
                "Phase 2 complete: blocks indexed"
            );

            if index_errors > 0 {
                warn!(
                    errors = index_errors,
                    retry_secs = poll_interval_secs,
                    "Some blocks failed to index"
                );
            }

            // Check if we successfully caught up
            let new_db_tip = match get_db_chain_tip(&db) {
                Ok(tip) => tip,
                Err(_) => db_tip.clone(),
            };

            if new_db_tip.height >= rpc_tip.height {
                info!(
                    current_height = new_db_tip.height,
                    network_height = rpc_tip.height,
                    "RPC catchup complete - fully synced"
                );
                metrics::set_blocks_behind_tip(0);
                metrics::RPC_CATCHUP_BLOCKS.inc_by(blocks_behind as u64);

                // Update aggregate metrics after catchup completes
                if let Err(e) = update_aggregate_metrics(&db) {
                    warn!(error = %e, "Failed to update aggregate metrics");
                } else {
                    // Persist metrics to database for restart durability
                    if let Err(e) = metrics::save_metrics_to_db(&db) {
                        warn!(error = %e, "Failed to persist metrics to database");
                    } else {
                        debug!("Metrics persisted to database");
                    }
                }
            }
        } else {
            // We're caught up - update aggregate metrics periodically
            // Only update every 10 poll intervals to reduce overhead
            static POLL_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let count = POLL_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

            if count % 10 == 0 {
                if let Err(e) = update_aggregate_metrics(&db) {
                    warn!(error = %e, "Failed to update aggregate metrics");
                } else {
                    // Persist metrics to database periodically
                    if let Err(e) = metrics::save_metrics_to_db(&db) {
                        warn!(error = %e, "Failed to persist metrics to database");
                    }
                }
            }

            // We're caught up - sleep before checking again
            tokio::time::sleep(Duration::from_secs(poll_interval_secs)).await;
        }

        // Live daily-analytics: drive Lane I over (watermark, sync_height] + the
        // cadence Lane-R recompute. No-ops unless `sync.live_analytics` is on AND a
        // full enrich has set the live-ready gate. Steady-state only (this is the
        // post-enrich monitor; the pre-enrich bulk sync lives in sync.rs).
        crate::analytics_live::tick(&db).await;

        // Periodic richlist/wealth recompute from the live addr_index. Opt-in
        // (no-ops unless sync.analytics_recompute_enabled); decoupled from the
        // heavy full enrich so the frozen snapshots stay fresh between enriches.
        // Detaches its scan onto a blocking worker, so this returns immediately.
        crate::analytics_recompute::maybe_recompute(&db);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rocksdb::{Options, DB};
    use std::sync::Arc;

    fn open_db() -> (tempfile::TempDir, Arc<DB>) {
        let temp = tempfile::TempDir::new().unwrap();
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db = Arc::new(DB::open_cf(&opts, temp.path(), &["chain_state"]).unwrap());
        (temp, db)
    }

    #[test]
    fn restore_sync_height_lifts_a_regressed_watermark() {
        // Crash recovery re-applied a stale below-tip marker, rolling sync_height
        // back; restore it to the pre-recovery tip.
        let (_t, db) = open_db();
        crate::chain_state::set_sync_height(&db, 5463188).unwrap();
        assert!(restore_sync_height_if_regressed(&db, 5474276));
        assert_eq!(crate::chain_state::get_sync_height(&db).unwrap(), 5474276);
    }

    #[test]
    fn restore_sync_height_leaves_an_advanced_watermark() {
        // A legitimate tip-block recovery advanced the watermark past `saved`; leave it.
        let (_t, db) = open_db();
        crate::chain_state::set_sync_height(&db, 5474300).unwrap();
        assert!(!restore_sync_height_if_regressed(&db, 5474276));
        assert_eq!(crate::chain_state::get_sync_height(&db).unwrap(), 5474300);
    }
}
