/// Block Monitor Service - Real-time blockchain monitoring via RPC
/// 
/// Responsibilities:
/// - Poll RPC node for new blocks
/// - Detect chain tip changes
/// - Trigger block indexing
/// - Detect and handle reorgs

use std::sync::Arc;
use std::time::Duration;
use rocksdb::DB;
use pivx_rpc_rs::BitcoinRpcClient;
use serde_json::Value;

use crate::config::get_global_config;
use crate::websocket::EventBroadcaster;
use crate::chain_state::set_network_height;

#[derive(Debug, Clone)]
pub struct ChainTip {
    pub height: i32,
    pub hash: String,
}

/// Get current chain tip from RPC node
fn get_rpc_chain_tip(
    rpc_client: &BitcoinRpcClient,
) -> Result<ChainTip, Box<dyn std::error::Error>> {
    // Get block count (height)
    let height_i64 = rpc_client.getblockcount()?;
    let height = height_i64 as i32;
    
    // Get block hash at this height
    let hash = rpc_client.getblockhash(height as i64)?;
    
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

/// Fetch and index a single block from RPC
async fn index_block_from_rpc(
    rpc_client: &BitcoinRpcClient,
    height: i32,
    db: &Arc<DB>,
    broadcaster: &Option<Arc<EventBroadcaster>>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Get block hash at this height
    let block_hash = rpc_client.getblockhash(height as i64)?;
    
    // Make raw RPC call to get block with verbosity=2 (full transaction data)
    // We can't use the library's getblock because FullBlock deserialization
    // fails when the response has mixed string/object types in the tx array
    let config = get_global_config();
    let url = config.get_string("rpc.host")?;
    let user = config.get_string("rpc.user")?;
    let pass = config.get_string("rpc.pass")?;
    
    let client = reqwest::blocking::Client::new();
    
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
        .send()?;
    
    let json: Value = response.json()?;
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
            {
                Ok(tx_resp) => {
                    match tx_resp.json::<Value>() {
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
        
        // 1. Store full transaction: 't' + txid_reversed → (version + height + raw_tx)
        // Database uses INTERNAL (reversed) format for txid keys to match Bitcoin Core
        let mut txid_bytes_reversed: Vec<u8> = txid_bytes.iter().rev().cloned().collect();
        
        let mut tx_key = vec![b't'];
        tx_key.extend_from_slice(&txid_bytes_reversed);
        
        let mut full_data = version.to_le_bytes().to_vec();
        full_data.extend(&height.to_le_bytes());
        full_data.extend(&raw_tx_bytes);
        
        if let Err(e) = db.put_cf(&cf_transactions, &tx_key, &full_data) {
            eprintln!("⚠️  Failed to store transaction {}: {}", txid, e);
            tx_errors += 1;
            continue;
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
        
        // Index addresses from outputs - match blk file format
        let cf_addr_index = db.cf_handle("addr_index")
            .ok_or("addr_index CF not found")?;
        
        // Reversed txid for UTXO storage (internal format)
        let mut reversed_txid = txid_bytes.clone();
        reversed_txid.reverse();
        
        for (output_idx, output) in parsed_tx.outputs.iter().enumerate() {
            for address in &output.address {
                if address.is_empty() {
                    continue;
                }
                
                // Key format: 'a' + address
                let mut addr_key = vec![b'a'];
                addr_key.extend_from_slice(address.as_bytes());
                
                // Get existing UTXOs for this address
                let existing_utxos = match db.get_cf(&cf_addr_index, &addr_key)? {
                    Some(data) => deserialize_utxos(&data).await,
                    None => Vec::new(),
                };
                
                // Add new UTXO: (reversed_txid, output_index)
                let mut updated_utxos = existing_utxos;
                updated_utxos.push((reversed_txid.clone(), output_idx as u64));
                
                // Store updated UTXO list
                let serialized = serialize_utxos(&updated_utxos).await;
                if let Err(e) = db.put_cf(&cf_addr_index, &addr_key, &serialized) {
                    eprintln!("⚠️  Failed to index address {} for tx {}: {}", address, txid, e);
                }
            }
        }
        
        // Process inputs: remove spent UTXOs from address index
        // AND fetch missing previous transactions via RPC if needed
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
            
            // Decode the previous txid from hex string to bytes
            let prev_txid_bytes = match hex::decode(prev_txid_hex) {
                Ok(bytes) => bytes,
                Err(_) => {
                    eprintln!("⚠️  Invalid prev txid hex: {}", prev_txid_hex);
                    continue;
                }
            };
            
            // Previous txid needs to be in reversed (internal) format for lookup
            let mut prev_txid_internal: Vec<u8> = prev_txid_bytes.iter().rev().cloned().collect();
            
            // Get the previous transaction
            let mut prev_tx_key = vec![b't'];
            prev_tx_key.extend_from_slice(&prev_txid_internal);
            
            // Try to get from database first
            let prev_tx_data_opt = db.get_cf(&cf_transactions, &prev_tx_key)?;
            
            // If not in database, fetch from RPC and store it
            let prev_tx_data = if let Some(data) = prev_tx_data_opt {
                data
            } else {
                // Previous transaction not in DB - fetch it from RPC with full details
                eprintln!("   🔍 Fetching missing previous tx {} from RPC...", prev_txid_hex);
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
                {
                    Ok(resp) => {
                        match resp.json::<Value>() {
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
                                                {
                                                    Ok(block_resp) => {
                                                        block_resp.json::<Value>()
                                                            .ok()
                                                            .and_then(|j| j.get("result").and_then(|r| r.get("height")).and_then(|h| h.as_i64()))
                                                            .unwrap_or(0) as i32
                                                    }
                                                    Err(_) => 0,
                                                };
                                                
                                                // Store this transaction with proper height
                                                let mut full_data = version.to_le_bytes().to_vec();
                                                full_data.extend(&prev_height.to_le_bytes());
                                                full_data.extend(&raw_bytes);
                                                
                                                if let Err(e) = db.put_cf(&cf_transactions, &prev_tx_key, &full_data) {
                                                    eprintln!("⚠️  Failed to cache previous tx {}: {}", prev_txid_hex, e);
                                                } else {
                                                    eprintln!("   ✅ Cached previous tx {} (height: {})", prev_txid_hex, prev_height);
                                                }
                                                
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
                        
                        let mut addr_key = vec![b'a'];
                        addr_key.extend_from_slice(address.as_bytes());
                        
                        // Get existing UTXOs
                        let existing_utxos = match db.get_cf(&cf_addr_index, &addr_key)? {
                            Some(data) => deserialize_utxos(&data).await,
                            None => Vec::new(),
                        };
                        
                        // Remove the spent UTXO (match by reversed txid and index)
                        let updated_utxos: Vec<_> = existing_utxos.into_iter()
                            .filter(|(stored_txid, stored_idx)| {
                                !(stored_txid == &prev_txid_internal && *stored_idx == prev_output_idx as u64)
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
        
        tx_count += 1;
    }
    
    // Report any errors
    if tx_errors > 0 {
        eprintln!("⚠️  Block {}: Indexed {}/{} transactions ({} errors)", 
                  height, tx_count, tx_array.len(), tx_errors);
    } else if height % 100 == 0 {
        println!("✅ Block {}: Indexed all {} transactions", height, tx_count);
    }
    
    // Update sync height
    let cf_state = db.cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    
    db.put_cf(&cf_state, b"sync_height", &height.to_le_bytes())?;
    
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
    if rpc_tip.height == db_tip.height {
        if rpc_tip.hash != db_tip.hash {
            println!("REORG DETECTED: Hash mismatch at height {}", rpc_tip.height);
            return Ok(Some(rpc_tip.height - 1));
        }
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
    
    // Initialize RPC client
    let config = get_global_config();
    let rpc_host = config.get_string("rpc.host")?;
    let rpc_user = config.get_string("rpc.user")?;
    let rpc_pass = config.get_string("rpc.pass")?;
    
    let rpc_client = Arc::new(BitcoinRpcClient::new(
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
            // Connected successfully - store initial network height
            if let Err(e) = set_network_height(&db, height as i32) {
                eprintln!("Failed to set initial network height: {}", e);
            }
        }
        Err(e) => {
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
            Ok(tip) => tip,
            Err(e) => {
                eprintln!("Failed to get RPC tip: {}", e);
                tokio::time::sleep(Duration::from_secs(poll_interval_secs)).await;
                continue;
            }
        };
        
        // Update network height in database
        if let Err(e) = set_network_height(&db, rpc_tip.height) {
            eprintln!("Failed to update network height: {}", e);
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
        if let Some(reorg_height) = detect_reorg(&db_tip, &rpc_tip)? {
            println!("Handling reorg from height {}", reorg_height);
            // TODO: Implement reorg handling
            // 1. Rollback DB to reorg_height
            // 2. Re-index from that point
            tokio::time::sleep(Duration::from_secs(poll_interval_secs)).await;
            continue;
        }
        
        // Check if we're behind
        if rpc_tip.height > db_tip.height {
            let blocks_behind = rpc_tip.height - db_tip.height;
            
            println!("\n📡 RPC CATCHUP: {} blocks behind (heights {} → {})", 
                     blocks_behind, db_tip.height + 1, rpc_tip.height);
            
            // Index new blocks - NO SLEEP when catching up!
            for (idx, height) in ((db_tip.height + 1)..=rpc_tip.height).enumerate() {
                // Index the block
                if let Err(e) = index_block_from_rpc(&rpc_client, height, &db, &broadcaster).await {
                    eprintln!("❌ Failed to index block at height {}: {}", height, e);
                    // Brief pause on error before retry
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    break;
                }
                
                // Show progress every 100 blocks or on the last block
                if idx % 100 == 0 || height == rpc_tip.height {
                    let progress = ((idx + 1) as f64 / blocks_behind as f64) * 100.0;
                    println!("   📊 Progress: {}/{} ({:.1}%) - Block {}", 
                             idx + 1, blocks_behind, progress, height);
                }
            }
            
            // Check if we successfully caught up
            let new_db_tip = match get_db_chain_tip(&db) {
                Ok(tip) => tip,
                Err(_) => db_tip.clone(),
            };
            
            if new_db_tip.height >= rpc_tip.height {
                println!("\n✅ RPC CATCHUP COMPLETE!");
                println!("   📍 Current height: {}", new_db_tip.height);
                println!("   🌐 Network height: {}", rpc_tip.height);
                println!("   🎯 Status: FULLY SYNCED\n");
                println!("╔════════════════════════════════════════════════════╗");
                println!("║     🎉 INDEXING COMPLETE - READY FOR USE 🎉       ║");
                println!("╚════════════════════════════════════════════════════╝\n");
            }
        } else {
            // We're caught up - sleep before checking again
            tokio::time::sleep(Duration::from_secs(poll_interval_secs)).await;
        }
    }
}