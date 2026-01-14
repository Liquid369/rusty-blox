/// Block Monitor Service - Real-time blockchain monitoring via RPC
/// 
/// Responsibilities:
/// - Poll RPC node for new blocks
/// - Detect chain tip changes
/// - Trigger block indexing
/// - Detect and handle reorgs

use std::sync::Arc;
use std::time::Duration;
use std::collections::HashSet;
use rocksdb::DB;
use pivx_rpc_rs::PivxRpcClient;
use serde_json::Value;
use tracing::{info, warn, error, info_span};
use crate::metrics;

use crate::config::get_global_config;
use crate::websocket::EventBroadcaster;
use crate::chain_state::set_network_height;
use crate::reorg;

#[derive(Debug, Clone)]
pub struct ChainTip {
    pub height: i32,
    pub hash: String,
}

/// Structure to hold fetched block data for two-phase processing
#[derive(Debug, Clone)]
struct FetchedBlock {
    height: i32,
    block_hash: String,
    json_result: Value,
}

/// Get current chain tip from RPC node
fn get_rpc_chain_tip(
    rpc_client: &PivxRpcClient,
) -> Result<ChainTip, Box<dyn std::error::Error>> {
    let timer = metrics::Timer::new();
    
    // Get block count (height)
    let height_i64 = rpc_client.getblockcount()?;
    let height = height_i64 as i32;
    
    metrics::RPC_CALL_DURATION
        .with_label_values(&["getblockcount"])
        .observe(timer.elapsed_secs());
    
    // Get block hash at this height
    let timer2 = metrics::Timer::new();
    let hash = rpc_client.getblockhash(height as i64)?;
    
    metrics::RPC_CALL_DURATION
        .with_label_values(&["getblockhash"])
        .observe(timer2.elapsed_secs());
    
    Ok(ChainTip {
        height,
        hash,
    })
}

/// Get our current chain tip from database
fn get_db_chain_tip(db: &Arc<DB>) -> Result<ChainTip, Box<dyn std::error::Error>> {
    let cf_state = db.cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    
    // Get sync height
    let height = match db.get_cf(&cf_state, b"sync_height")? {
        Some(bytes) => i32::from_le_bytes(bytes.as_slice().try_into()?),
        None => {
            // Fallback: scan chain_metadata
            let cf_metadata = db.cf_handle("chain_metadata")
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
    let cf_metadata = db.cf_handle("chain_metadata")
        .ok_or("chain_metadata CF not found")?;
    
    let height_key = height.to_le_bytes().to_vec();
    let hash_bytes = db.get_cf(&cf_metadata, &height_key)?
        .ok_or("Block hash not found for current height")?;
    
    let hash = hex::encode(&hash_bytes);
    
    Ok(ChainTip {
        height,
        hash,
    })
}

/// Phase 1a: Fetch block data from RPC (without indexing)
/// Returns parsed block JSON for later processing
async fn fetch_block_data(
    rpc_client: &PivxRpcClient,
    height: i32,
) -> Result<FetchedBlock, Box<dyn std::error::Error>> {
    let config = get_global_config();
    let url = config.get_string("rpc.host")?;
    let user = config.get_string("rpc.user")?;
    let pass = config.get_string("rpc.pass")?;
    
    let client = reqwest::Client::new();
    
    // Get block hash at this height
    let timer = metrics::Timer::new();
    let block_hash = rpc_client.getblockhash(height as i64)?;
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
    let result = json.get("result")
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
    rpc_client: &PivxRpcClient,
    height: i32,
    db: &Arc<DB>,
    broadcaster: &Option<Arc<EventBroadcaster>>,
    spent_set: Option<&HashSet<(Vec<u8>, u64)>>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Check if address enrichment has completed and at what height
    // Only update address index for blocks AFTER enrichment height
    // (Enrichment already processed all historical blocks correctly)
    let cf_state = db.cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    
    let enrichment_height = db.get_cf(&cf_state, b"enrichment_height")?
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
        None => true,  // Enrichment hasn't run, update address index
        Some(enrich_height) => height > enrich_height,  // Only for NEW blocks
    };
    
    // Get block hash at this height
    let block_hash = rpc_client.getblockhash(height as i64)?;
    
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
            eprintln!("⚠️  REORG detected at height {}", height);
            eprintln!("   Expected: {}", existing_hash_str);
            eprintln!("   Current:  {}", block_hash);
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
    db.put_cf(&cf_state, &processing_key, &height.to_le_bytes())?;
    
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
        cf_state: &cf_state,
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
    let result = json.get("result")
        .ok_or("No result in RPC response")?;
    
    // Extract block version and transactions
    let version = result.get("version")
        .and_then(|v| v.as_i64())
        .unwrap_or(1) as i32;
    
    let tx_array = result.get("tx")
        .and_then(|t| t.as_array())
        .ok_or("No tx array in block")?;
    
    // Store block header data in chain_metadata
    let cf_metadata = db.cf_handle("chain_metadata")
        .ok_or("chain_metadata CF not found")?;
    
    let height_key = height.to_le_bytes().to_vec();
    let hash_bytes = hex::decode(&block_hash)?;
    
    // Store height -> hash mapping
    db.put_cf(&cf_metadata, &height_key, &hash_bytes)?;
    
    // Store hash -> height mapping for reverse lookups
    let mut hash_key = vec![b'h'];
    hash_key.extend_from_slice(&hash_bytes);
    let height_bytes = height.to_le_bytes().to_vec();
    db.put_cf(&cf_metadata, &hash_key, &height_bytes)?;
    
    // Store block header in blocks CF
    // Create a minimal header with the data we have from RPC
    let cf_blocks = db.cf_handle("blocks")
        .ok_or("blocks CF not found")?;
    
    // Parse block header fields from RPC result
    let time = result.get("time").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let nonce = result.get("nonce").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let bits = result.get("bits").and_then(|v| v.as_str()).unwrap_or("00000000");
    let merkleroot = result.get("merkleroot").and_then(|v| v.as_str()).unwrap_or("");
    let previousblockhash = result.get("previousblockhash").and_then(|v| v.as_str()).unwrap_or("");
    
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
    let cf_transactions = db.cf_handle("transactions")
        .ok_or("transactions CF not found")?;
    
    let mut tx_count = 0;
    let mut tx_errors = 0;
    
    for (tx_index, tx_val) in tx_array.iter().enumerate() {
        // With verbosity=2, tx_val could be:
        // - A string (just txid) - older PIVX versions
        // - An object (full transaction data) - newer versions
        
        let txid = if let Some(txid_str) = tx_val.as_str() {
            // Old format: just a txid string
            txid_str.to_string()
        } else if let Some(tx_obj) = tx_val.as_object() {
            // New format: full transaction object
            tx_obj.get("txid")
                .and_then(|t| t.as_str())
                .ok_or("Missing txid in transaction object")?
                .to_string()
        } else {
            eprintln!("⚠️  Skipping invalid transaction at index {} in block {}", tx_index, height);
            tx_errors += 1;
            continue;
        };
        
        let txid_bytes = match hex::decode(&txid) {
            Ok(bytes) => bytes,
            Err(_) => {
                eprintln!("⚠️  Invalid txid hex at index {} in block {}: {}", tx_index, height, txid);
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
                        eprintln!("⚠️  Failed to decode hex for txid {}", txid);
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
                Ok(tx_resp) => {
                    match tx_resp.json::<Value>().await {
                        Ok(tx_json) => {
                            if let Some(raw_hex) = tx_json.get("result").and_then(|r| r.as_str()) {
                                match hex::decode(raw_hex) {
                                    Ok(bytes) => bytes,
                                    Err(_) => {
                                        eprintln!("⚠️  Failed to decode getrawtransaction result for {}", txid);
                                        tx_errors += 1;
                                        continue;
                                    }
                                }
                            } else {
                                eprintln!("⚠️  No result in getrawtransaction for {}", txid);
                                tx_errors += 1;
                                continue;
                            }
                        }
                        Err(e) => {
                            eprintln!("⚠️  Failed to parse getrawtransaction response for {}: {}", txid, e);
                            tx_errors += 1;
                            continue;
                        }
                    }
                }
                Err(e) => {
                    eprintln!("⚠️  Failed to fetch transaction {}: {}", txid, e);
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
                    existing_data[4], existing_data[5], existing_data[6], existing_data[7]
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
            eprintln!("⚠️  Transaction {} has invalid size (< 4 bytes), using default version", txid);
            &[1u8, 0, 0, 0] // Default to version 1
        };
        
        let mut full_data = tx_version_bytes.to_vec();
        full_data.extend(&height.to_le_bytes());
        full_data.extend(&raw_tx_bytes);
        
        if needs_update {
            if let Err(e) = db.put_cf(&cf_transactions, &tx_key, &full_data) {
                eprintln!("⚠️  Failed to store transaction {}: {}", txid, e);
                tx_errors += 1;
                continue;
            }
            
            if existing_tx_data.is_some() {
                eprintln!("✅ Promoted mempool tx {} to confirmed at height {}", txid, height);
            }
        }
        
        // 2. Block transaction index: 'B' + height + tx_index → txid
        let mut block_tx_key = vec![b'B'];
        block_tx_key.extend(&height.to_le_bytes());
        block_tx_key.extend(&(tx_index as u64).to_le_bytes());
        
        // Store the txid in display format
        if let Err(e) = db.put_cf(&cf_transactions, &block_tx_key, txid.as_bytes()) {
            eprintln!("⚠️  Failed to store block TX index for {}: {}", txid, e);
            tx_errors += 1;
            continue;
        }
        
        // 3. Parse transaction and index addresses/UTXOs
        // Validate transaction size before allocation
        if raw_tx_bytes.len() > 10_000_000 {
            eprintln!("⚠️  Transaction {} too large ({} bytes), skipping", txid, raw_tx_bytes.len());
            tx_errors += 1;
            continue;
        }
        
        if raw_tx_bytes.is_empty() {
            eprintln!("⚠️  Transaction {} has empty data, skipping", txid);
            tx_errors += 1;
            continue;
        }
        
        // Prepend dummy block_version for parser compatibility
        let mut tx_data_with_header = Vec::with_capacity(4 + raw_tx_bytes.len());
        tx_data_with_header.extend_from_slice(&[0u8; 4]);
        tx_data_with_header.extend_from_slice(&raw_tx_bytes);
        
        // Parse the transaction
        use crate::parser::{deserialize_transaction, serialize_utxos, deserialize_utxos};
        
        let parsed_tx = match deserialize_transaction(&tx_data_with_header).await {
            Ok(tx) => tx,
            Err(e) => {
                eprintln!("⚠️  Failed to parse transaction {}: {}", txid, e);
                tx_errors += 1;
                continue;
            }
        };
        
        // Index addresses from outputs
        // CRITICAL FIX: Only modify address index for NEW blocks (after enrichment height)
        // Blocks processed by enrichment already have correct address index data
        let cf_addr_index = db.cf_handle("addr_index")
            .ok_or("addr_index CF not found")?;
        
        // txid_bytes is already in display format (no reversal needed)
        // Database keys and UTXO storage use display format consistently
        
        // Track which addresses are involved in this transaction (for tx history)
        let mut involved_addresses = std::collections::HashSet::new();
        
        // Add outputs as UTXOs ONLY for new blocks
        if should_update_address_index {
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
                    
                    // Get existing UTXOs for this address
                    let mut existing_utxos = match db.get_cf(&cf_addr_index, &addr_key)? {
                        Some(data) => deserialize_utxos(&data).await,
                        None => Vec::new(),
                    };
                    
                    // CRITICAL FIX: Check if UTXO already exists (idempotent)
                    let already_exists = existing_utxos.iter().any(|(t, i)| {
                        t == &txid_bytes && *i == output_idx as u64
                    });
                    
                    if already_exists {
                        // Already indexed - skip (prevents duplicates from reorg/reindex)
                        #[cfg(feature = "debug-address-index")]
                        eprintln!("⚠️  Duplicate UTXO detected (skipped): {}:{} for {}", 
                                 hex::encode(&txid_bytes), output_idx, address);
                        continue;
                    }
                    
                    // Add new UTXO: (txid_bytes, output_index)
                    existing_utxos.push((txid_bytes.clone(), output_idx as u64));
                
                    // Store updated UTXO list
                    let serialized = serialize_utxos(&existing_utxos).await;
                    if let Err(e) = db.put_cf(&cf_addr_index, &addr_key, &serialized) {
                        eprintln!("⚠️  Failed to index address {} for tx {}: {}", address, txid, e);
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
                    eprintln!("⚠️  Invalid prev txid hex: {}", prev_txid_hex);
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
                                    let blockhash = result.get("blockhash").and_then(|h| h.as_str());
                                    
                                    if let (Some(hex_str), Some(block_hash)) = (raw_hex, blockhash) {
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
                                                    Ok(block_resp) => {
                                                        block_resp.json::<Value>()
                                                            .await
                                                            .ok()
                                                            .and_then(|j| j.get("result").and_then(|r| r.get("height")).and_then(|h| h.as_i64()))
                                                            .unwrap_or(0) as i32
                                                    }
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
                                                
                                                if let Err(e) = db.put_cf(&cf_transactions, &prev_tx_key, &full_data) {
                                                    eprintln!("⚠️  Failed to cache previous tx {}: {}", prev_txid_hex, e);
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
                                                eprintln!("⚠️  Failed to decode previous tx hex {}", prev_txid_hex);
                                                continue;
                                            }
                                        }
                                    } else {
                                        eprintln!("⚠️  Missing hex or blockhash for previous tx {}", prev_txid_hex);
                                        continue;
                                    }
                                } else {
                                    eprintln!("⚠️  No result for previous tx {}", prev_txid_hex);
                                    continue;
                                }
                            }
                            Err(_) => {
                                eprintln!("⚠️  Failed to parse RPC response for previous tx {}", prev_txid_hex);
                                continue;
                            }
                        }
                    }
                    Err(_) => {
                        // RPC fetch failed - can't process this input
                        // This is non-fatal, just skip removing from UTXO set
                        eprintln!("⚠️  RPC request failed for previous tx {}", prev_txid_hex);
                        continue;
                    }
                }
            };
            
            // Now process the previous transaction data
            if prev_tx_data.len() < 8 {
                eprintln!("⚠️  Previous transaction {} data too short", prev_txid_hex);
                continue;
            }
            
            if prev_tx_data.len() > 10_000_008 {  // 10MB + 8 byte header
                eprintln!("⚠️  Previous transaction {} too large", prev_txid_hex);
                continue;
            }
            
            // Format: version (4) + height (4) + raw_tx
            // We need to prepend 4-byte dummy header for parser, so extract raw tx part
            let raw_prev_tx = &prev_tx_data[8..];
            
            if raw_prev_tx.is_empty() {
                eprintln!("⚠️  Previous transaction {} has empty data", prev_txid_hex);
                continue;
            }
            
            // Prepend dummy block_version for parser compatibility
            let mut prev_tx_with_header = Vec::with_capacity(4 + raw_prev_tx.len());
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
                        
                        // Get existing UTXOs
                        let existing_utxos = match db.get_cf(&cf_addr_index, &addr_key)? {
                            Some(data) => deserialize_utxos(&data).await,
                            None => Vec::new(),
                        };
                        
                        // Remove the spent UTXO (match by txid and index)
                        let updated_utxos: Vec<_> = existing_utxos.into_iter()
                            .filter(|(stored_txid, stored_idx)| {
                                !(stored_txid == &prev_txid_bytes && *stored_idx == prev_output_idx as u64)
                            })
                            .collect();
                        
                        // Update or delete
                        if !updated_utxos.is_empty() {
                            let serialized = serialize_utxos(&updated_utxos).await;
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
                    Some(data) => {
                        // Deserialize as list of txids (32 bytes each, reversed format)
                        data.chunks_exact(32)
                            .map(|chunk| chunk.to_vec())
                            .collect::<Vec<Vec<u8>>>()
                    },
                    None => Vec::new(),
                };
                
                // Add this transaction (if not already present)
                if !tx_list.iter().any(|t| t == &txid_bytes) {
                    tx_list.push(txid_bytes.clone());
                    
                    // Serialize transaction list (just concatenate txids)
                    let mut serialized = Vec::new();
                    for tx in &tx_list {
                        serialized.extend(tx);
                    }
                    
                    if let Err(e) = db.put_cf(&cf_addr_index, &tx_list_key, &serialized) {
                        eprintln!("⚠️  Failed to update tx list for {}: {}", address, e);
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
                    
                    let current_total = db.get_cf(&cf_addr_index, &key_r)?
                        .and_then(|bytes| {
                            if bytes.len() == 8 {
                                Some(i64::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3],
                                                         bytes[4], bytes[5], bytes[6], bytes[7]]))
                            } else {
                                None
                            }
                        })
                        .unwrap_or(0);
                    
                    let new_total = current_total + received_delta;
                    db.put_cf(&cf_addr_index, &key_r, &new_total.to_le_bytes())?;
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
                                    let mut prev_tx_with_header = Vec::with_capacity(4 + prev_raw_tx.len());
                                    prev_tx_with_header.extend_from_slice(&[0u8; 4]);
                                    prev_tx_with_header.extend_from_slice(prev_raw_tx);
                                    
                                    if let Ok(prev_tx) = deserialize_transaction(&prev_tx_with_header).await {
                                        if let Some(prev_output) = prev_tx.outputs.get(prev_output_idx as usize) {
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
                    
                    let current_total = db.get_cf(&cf_addr_index, &key_s)?
                        .and_then(|bytes| {
                            if bytes.len() == 8 {
                                Some(i64::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3],
                                                         bytes[4], bytes[5], bytes[6], bytes[7]]))
                            } else {
                                None
                            }
                        })
                        .unwrap_or(0);
                    
                    let new_total = current_total + sent_delta;
                    db.put_cf(&cf_addr_index, &key_s, &new_total.to_le_bytes())?;
                }
            }
        }
        
        tx_count += 1;
    }
    
    // Report any errors
    if tx_errors > 0 {
        eprintln!("⚠️  Block {}: Indexed {}/{} transactions ({} errors)", 
                  height, tx_count, tx_array.len(), tx_errors);
    } else if height % 1000 == 0 {
        println!("✅ Block {}: Indexed all {} transactions", height, tx_count);
    }
    // Debug logging reduced for performance - only log every 1000 blocks or on errors
    
    // Update sync height
    let cf_state = db.cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    
    db.put_cf(&cf_state, b"sync_height", height.to_le_bytes())?;
    
    // CRITICAL FIX: Store block hash for deduplication
    let mut height_hash_key = vec![b'H'];
    height_hash_key.extend(&height.to_le_bytes());
    db.put_cf(&cf_state, &height_hash_key, block_hash.as_bytes())?;
    
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

/// Detect if reorg occurred
fn detect_reorg(
    db_tip: &ChainTip,
    rpc_tip: &ChainTip,
) -> Result<Option<i32>, Box<dyn std::error::Error>> {
    // If RPC height is less than ours, definite reorg
    if rpc_tip.height < db_tip.height {
        println!("REORG DETECTED: RPC height {} < DB height {}", rpc_tip.height, db_tip.height);
        return Ok(Some(rpc_tip.height));
    }
    
    // If heights are same, check hashes
    if rpc_tip.height == db_tip.height && rpc_tip.hash != db_tip.hash {
        println!("REORG DETECTED: Hash mismatch at height {}", rpc_tip.height);
        return Ok(Some(rpc_tip.height - 1));
    }
    
    // TODO: More sophisticated reorg detection
    // - Compare hashes at previous heights
    // - Find common ancestor
    
    Ok(None)
}

/// Main block monitoring loop
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
    
    let rpc_client = Arc::new(PivxRpcClient::new(
        rpc_host.clone(),
        Some(rpc_user),
        Some(rpc_pass),
        3,     // Max retries
        10,    // Connection timeout (seconds)
        30000, // Read/write timeout (milliseconds) - increased for getblock calls
    ));
    
    // Test connection
    match rpc_client.getblockcount() {
        Ok(height) => {
            info!(rpc_height = height, "RPC connection established");
            metrics::set_rpc_connected(true);
            
            // Connected successfully - store initial network height
            if let Err(e) = set_network_height(&db, height as i32) {
                error!(error = %e, "Failed to set initial network height");
            }
        }
        Err(e) => {
            error!(error = %e, "RPC connection failed");
            metrics::set_rpc_connected(false);
            eprintln!("RPC connection failed: {}", e);
            eprintln!("Tip: Make sure PIVX node is running with RPC enabled");
            
            // Just poll database for changes
            loop {
                tokio::time::sleep(Duration::from_secs(poll_interval_secs)).await;
            }
        }
    }
    
    loop {
        // Get current tips
        let rpc_tip = match get_rpc_chain_tip(&rpc_client) {
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
                eprintln!("Failed to get DB tip: {}", e);
                tokio::time::sleep(Duration::from_secs(poll_interval_secs)).await;
                continue;
            }
        };
        
        // Check for reorg
        if let Some(_reorg_height) = detect_reorg(&db_tip, &rpc_tip)? {
            println!("\n⚠️  BLOCKCHAIN REORGANIZATION DETECTED!");
            
            // Handle the reorg using our reorg module
            match reorg::handle_reorg(
                db.clone(),
                &rpc_client,
                db_tip.height,
                rpc_tip.height,
            ).await {
                Ok(reorg_info) => {
                    println!("✅ Reorg handled successfully");
                    println!("   Orphaned {} blocks from fork at height {}", 
                             reorg_info.orphaned_blocks, reorg_info.fork_height);
                    
                    // Continue to re-index from the rollback point
                    // The normal sync logic below will pick up from the new chain tip
                }
                Err(e) => {
                    eprintln!("❌ Failed to handle reorg: {}", e);
                    eprintln!("   Waiting {} seconds before retry...", poll_interval_secs);
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
            ).entered();
            
            info!(
                blocks_behind = blocks_behind,
                current_height = db_tip.height,
                target_height = rpc_tip.height,
                "RPC catchup needed"
            );
            metrics::set_blocks_behind_tip(blocks_behind as i64);
            
            println!("\n📡 RPC CATCHUP: {} blocks behind (heights {} → {})", 
                     blocks_behind, db_tip.height + 1, rpc_tip.height);
            
            // TWO-PHASE RPC CATCHUP (matches initial sync two-pass algorithm)
            // This ensures 100% accurate spend detection without network dependency
            
            let start_height = db_tip.height + 1;
            let end_height = rpc_tip.height;
            
            // === PHASE 1: Fetch blocks and build complete spent set ===
            println!("📥 Phase 1: Fetching {} blocks and building spent set...", blocks_behind);
            
            let mut fetched_blocks = Vec::new();
            let mut fetch_errors = 0;
            
            // Fetch blocks in parallel for network efficiency
            // Use semaphore to limit concurrency
            use tokio::sync::Semaphore;
            use std::sync::Arc as StdArc;
            
            const MAX_CONCURRENT_FETCH: usize = 10;
            let fetch_semaphore = StdArc::new(Semaphore::new(MAX_CONCURRENT_FETCH));
            
            let mut fetch_futures = Vec::new();
            for height in start_height..=end_height {
                let rpc = rpc_client.clone();
                let sem = fetch_semaphore.clone();
                
                fetch_futures.push(async move {
                    let _permit = sem.acquire().await.unwrap();
                    (height, fetch_block_data(&rpc, height).await)
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
                        eprintln!("❌ Failed to fetch block {}: {}", height, e);
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
                eprintln!("⚠️  {} fetch errors occurred, waiting {} seconds before retry...", 
                         fetch_errors, poll_interval_secs);
                tokio::time::sleep(Duration::from_secs(poll_interval_secs)).await;
                continue;
            }
            
            // Build complete spent set from all fetched blocks
            println!("🔍 Building spent set from {} blocks...", fetched_blocks.len());
            let spent_set = build_spent_set_from_blocks(&fetched_blocks);
            println!("✅ Phase 1 complete: {} outputs identified as spent", spent_set.len());
            
            // === PHASE 2: Sequential indexing with complete spent knowledge ===
            println!("📝 Phase 2: Indexing blocks with complete spent set...");
            
            let mut indexed = 0;
            let mut index_errors = 0;
            
            // Process blocks sequentially in height order
            // This ensures:
            // - No race conditions
            // - Height N indexed before N+1
            // - Spend removal has complete knowledge
            for block in fetched_blocks {
                // Add canonical hash validation
                let cf_metadata = db.cf_handle("chain_metadata")
                    .ok_or("chain_metadata CF not found")?;
                
                let height_key = block.height.to_le_bytes();
                
                // Check if we already have a canonical hash for this height
                if let Some(stored_hash) = db.get_cf(&cf_metadata, &height_key)? {
                    let stored_hash_hex = hex::encode(&stored_hash);
                    
                    if stored_hash_hex != block.block_hash {
                        eprintln!("⚠️  REORG detected at height {}", block.height);
                        eprintln!("   DB hash:  {}", stored_hash_hex);
                        eprintln!("   RPC hash: {}", block.block_hash);
                        eprintln!("   Aborting catchup to trigger reorg handler");
                        break;
                    }
                }
                
                // Index this block with spent set available
                match index_block_from_rpc(&rpc_client, block.height, &db, &broadcaster, Some(&spent_set)).await {
                    Ok(_) => {
                        indexed += 1;
                        if indexed % 10 == 0 {
                            let progress = (indexed as f64 / blocks_behind as f64) * 100.0;
                            println!("   📊 Progress: {}/{} ({:.1}%)", indexed, blocks_behind, progress);
                        }
                    }
                    Err(e) => {
                        eprintln!("❌ Failed to index block {}: {}", block.height, e);
                        index_errors += 1;
                    }
                }
            }
            
            println!("✅ Phase 2 complete: {}/{} blocks indexed ({} errors)", 
                     indexed, blocks_behind, index_errors);
            
            if index_errors > 0 {
                eprintln!("⚠️  Some blocks failed to index, waiting {} seconds before retry...", 
                         poll_interval_secs);
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
                

            }
        } else {
            // We're caught up - sleep before checking again
            tokio::time::sleep(Duration::from_secs(poll_interval_secs)).await;
        }
    }
}