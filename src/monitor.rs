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
fn index_block_from_rpc(
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
    
    let response = client
        .post(&url)
        .basic_auth(&user, Some(&pass))
        .json(&serde_json::json!({
            "jsonrpc": "1.0",
            "id": "rustyblox",
            "method": "getblock",
            "params": [block_hash.clone(), 2]
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
    
    db.put_cf(&cf_metadata, &height_key, &hash_bytes)?;
    
    // Index all transactions from this block
    let cf_transactions = db.cf_handle("transactions")
        .ok_or("transactions CF not found")?;
    
    let mut tx_count = 0;
    for tx_val in tx_array {
        // Each tx should be an object with txid and hex fields
        if let Some(tx_obj) = tx_val.as_object() {
            if let (Some(txid), Some(hex_str)) = (
                tx_obj.get("txid").and_then(|v| v.as_str()),
                tx_obj.get("hex").and_then(|v| v.as_str())
            ) {
                // Decode the transaction hex
                if let Ok(tx_bytes) = hex::decode(hex_str) {
                    // Create transaction key: 't' + txid
                    let mut key = vec![b't'];
                    if let Ok(txid_bytes) = hex::decode(txid) {
                        key.extend_from_slice(&txid_bytes);
                        
                        // Store: block_version (4 bytes) + transaction_data
                        let mut full_data = version.to_le_bytes().to_vec();
                        full_data.extend(&tx_bytes);
                        
                        db.put_cf(&cf_transactions, &key, &full_data)?;
                        tx_count += 1;
                    }
                }
            }
        }
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
        3,    // Max retries
        10,   // Connection timeout (seconds)
        1000, // Read/write timeout (milliseconds)
    ));
    
    // Test connection
    match rpc_client.getblockcount() {
        Ok(_height) => {
            // Connected successfully
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
        tokio::time::sleep(Duration::from_secs(poll_interval_secs)).await;
        
        // Get current tips
        let rpc_tip = match get_rpc_chain_tip(&rpc_client) {
            Ok(tip) => tip,
            Err(e) => {
                eprintln!("Failed to get RPC tip: {}", e);
                continue;
            }
        };
        
        let db_tip = match get_db_chain_tip(&db) {
            Ok(tip) => tip,
            Err(e) => {
                eprintln!("Failed to get DB tip: {}", e);
                continue;
            }
        };
        
        // Check for reorg
        if let Some(reorg_height) = detect_reorg(&db_tip, &rpc_tip)? {
            println!("Handling reorg from height {}", reorg_height);
            // TODO: Implement reorg handling
            // 1. Rollback DB to reorg_height
            // 2. Re-index from that point
            continue;
        }
        
        // Check if we're behind
        if rpc_tip.height > db_tip.height {
            // Index new blocks
            for height in (db_tip.height + 1)..=rpc_tip.height {
                // Index the block
                if let Err(e) = index_block_from_rpc(&rpc_client, height, &db, &broadcaster) {
                    eprintln!("Failed to index block at height {}: {}", height, e);
                    break;
                }
            }
        }
    }
}
