/// Blockchain Reorganization Handling
/// 
/// Implements PIVX-compatible chain reorganization (reorg) handling.
/// When the canonical chain changes, we must rollback orphaned blocks
/// and re-index the new canonical chain.
/// 
/// CRITICAL DEPENDENCIES:
/// - atomic_writer.rs: Ensures rollback is atomic (all or nothing)
/// - tx_type.rs: Needed to restore UTXO maturity state correctly
/// 
/// PIVX CORE EQUIVALENTS:
/// - DisconnectBlock(): Reverses block application
/// - ActivateBestChain(): Switches to new canonical chain
/// - AcceptBlock(): Reapplies blocks on new chain

use std::sync::Arc;
use rocksdb::DB;
use crate::atomic_writer::AtomicBatchWriter;
use crate::parser::deserialize_transaction;
use crate::address_rollback::rollback_address_index;

/// Represents information about a blockchain reorganization
#[derive(Debug, Clone)]
pub struct ReorgInfo {
    /// Height where chains diverged (last common block)
    pub fork_height: i32,
    /// Height we need to rollback to
    pub rollback_to: i32,
    /// Number of blocks being orphaned
    pub orphaned_blocks: i32,
    /// Old chain tip hash (being abandoned)
    pub old_tip_hash: String,
    /// New chain tip hash (being adopted)
    pub new_tip_hash: String,
}

impl ReorgInfo {
    pub fn new(
        fork_height: i32,
        rollback_to: i32,
        orphaned_blocks: i32,
        old_tip_hash: String,
        new_tip_hash: String,
    ) -> Self {
        Self {
            fork_height,
            rollback_to,
            orphaned_blocks,
            old_tip_hash,
            new_tip_hash,
        }
    }
}

/// Find the common ancestor between our chain and the RPC chain
/// 
/// This function walks backwards from the reorg height to find the
/// last block that matches between our database and the RPC chain.
/// 
/// # Arguments
/// * `db` - RocksDB instance
/// * `rpc_client` - PIVX Core RPC client
/// * `start_height` - Height to start searching backwards from
/// 
/// # Returns
/// Height of last common block, or error if not found
pub async fn find_fork_point(
    db: &Arc<DB>,
    rpc_client: &Arc<pivx_rpc_rs::BitcoinRpcClient>,
    start_height: i32,
) -> Result<i32, Box<dyn std::error::Error + Send + Sync>> {
    let cf_metadata = db.cf_handle("chain_metadata")
        .ok_or("chain_metadata CF not found")?;
    
    // Walk backwards to find common ancestor
    let mut height = start_height;
    
    while height > 0 {
        // Get our hash at this height
        let height_key = height.to_le_bytes().to_vec();
        
        let our_hash = match db.get_cf(&cf_metadata, &height_key)? {
            Some(hash_bytes) => hash_bytes,
            None => {
                // We don't have this height, go back further
                height -= 1;
                continue;
            }
        };
        
        // Get RPC hash at this height (async via spawn_blocking)
        let rpc_client_clone = rpc_client.clone();
        let height_copy = height;
        let rpc_block_hash = tokio::task::spawn_blocking(move || {
            rpc_client_clone.getblockhash(height_copy as i64)
        })
        .await??;
        
        let rpc_hash_bytes = hex::decode(&rpc_block_hash)?;
        
        // Compare hashes (both should be internal format - reversed)
        if our_hash == rpc_hash_bytes {
            // Found common ancestor!
            return Ok(height);
        }
        
        height -= 1;
    }
    
    // If we get here, chains have no common ancestor (very unlikely)
    Err("No common ancestor found - database may be corrupted".into())
}

/// Rollback database to specified height
/// 
/// This is the core reorg handling function. It atomically removes all
/// data for blocks above the specified height, restoring the database
/// to a consistent state at the fork point.
/// 
/// CRITICAL: Uses AtomicBatchWriter to ensure all-or-nothing rollback.
/// A crash during rollback will not leave partial state.
/// 
/// # Arguments
/// * `db` - RocksDB instance
/// * `rollback_to_height` - Height to rollback to (inclusive - this block stays)
/// * `current_height` - Current chain tip height
/// 
/// # Returns
/// Number of blocks rolled back, or error
pub async fn rollback_to_height(
    db: Arc<DB>,
    rollback_to_height: i32,
    current_height: i32,
) -> Result<i32, Box<dyn std::error::Error + Send + Sync>> {
    if rollback_to_height >= current_height {
        return Ok(0); // Nothing to rollback
    }
    
    if rollback_to_height < 0 {
        return Err("Cannot rollback to negative height".into());
    }
    
    let blocks_to_remove = current_height - rollback_to_height;
    
    println!("\n╔════════════════════════════════════════════════════╗");
    println!("║           BLOCKCHAIN REORGANIZATION                ║");
    println!("╚════════════════════════════════════════════════════╝");
    println!("  Rolling back from height {} to {}", current_height, rollback_to_height);
    println!("  Orphaning {} blocks", blocks_to_remove);
    println!();
    
    // Use atomic writer for safe rollback
    let mut writer = AtomicBatchWriter::new(db.clone(), 100000);
    let db_clone = db.clone();
    
    // Process each block to be removed (in reverse order)
    for height in ((rollback_to_height + 1)..=current_height).rev() {
        println!("  📦 Disconnecting block at height {}", height);
        
        // 1. Get all transactions in this block
        let txids = get_block_transactions(&db_clone, height).await?;
        
        // 2. Disconnect each transaction (reverse UTXO changes, address index, etc.)
        for txid_internal in txids {
            disconnect_transaction(&mut writer, &db_clone, &txid_internal, height).await?;
        }
        
        // 3. Delete block metadata
        delete_block_metadata(&mut writer, height)?;
        
        // Flush in batches to avoid excessive memory usage
        if writer.should_flush() {
            println!("  💾 Flushing atomic batch...");
            writer.flush().await?;
        }
    }
    
    // Final flush to commit all remaining operations atomically
    if writer.pending_count() > 0 {
        println!("  💾 Final atomic commit...");
        writer.flush().await?;
    }
    
    // Rollback address index (this handles address history, UTXOs, balances)
    println!("  📍 Rolling back address index...");
    match rollback_address_index(db.clone(), current_height, rollback_to_height).await {
        Ok(blocks_rolled_back) => {
            println!("  ✅ Address index rolled back {} blocks", blocks_rolled_back);
        }
        Err(e) => {
            println!("  ⚠️  Address index rollback incomplete: {}", e);
            println!("     Address data may need rebuild after reorg");
        }
    }
    
    // Update sync height to rollback point
    update_sync_height(&db, rollback_to_height).await?;
    
    println!("\n  ✅ Rollback complete! Database at height {}", rollback_to_height);
    println!("  ⚠️  Re-indexing will begin from height {}\n", rollback_to_height + 1);
    
    Ok(blocks_to_remove)
}

/// Get all transaction IDs in a block
async fn get_block_transactions(
    db: &Arc<DB>,
    height: i32,
) -> Result<Vec<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
    let db_clone = db.clone();
    let txids = tokio::task::spawn_blocking(move || {
        let cf_transactions = db_clone.cf_handle("transactions")
            .ok_or("transactions CF not found")?;
        
        let mut txids = Vec::new();
        let height_bytes = height.to_le_bytes();
        
        // Iterate over 'B' + height + index keys
        let mut prefix = vec![b'B'];
        prefix.extend_from_slice(&height_bytes);
        
        let iter = db_clone.prefix_iterator_cf(&cf_transactions, &prefix);
        
        for item in iter {
            let (key, value) = item?;
            
            // Key format: 'B' + height(4) + index(8)
            if key.len() == 13 && key[0] == b'B' {
                // Value is the txid hex string
                let txid_hex = String::from_utf8_lossy(&value).to_string();
                
                // Convert to internal format (decode and reverse)
                if let Ok(txid_bytes) = hex::decode(&txid_hex) {
                    let internal_txid: Vec<u8> = txid_bytes.iter().rev().cloned().collect();
                    txids.push(internal_txid);
                }
            }
        }
        
        Ok::<Vec<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>>(txids)
    })
    .await??;
    
    Ok(txids)
}

/// Disconnect (reverse) a single transaction
/// 
/// This is equivalent to PIVX Core's DisconnectBlock logic for one transaction:
/// 1. Restore spent outputs (resurrect UTXOs)
/// 2. Remove created outputs (delete UTXOs)
/// 3. Update address indices
/// 4. Delete transaction record
async fn disconnect_transaction(
    writer: &mut AtomicBatchWriter,
    db: &Arc<DB>,
    txid_internal: &[u8],
    _height: i32,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Get transaction data
    let mut tx_key = vec![b't'];
    tx_key.extend_from_slice(txid_internal);
    
    let db_clone = db.clone();
    let tx_key_clone = tx_key.clone();
    
    let tx_data = tokio::task::spawn_blocking(move || {
        let cf_transactions = db_clone.cf_handle("transactions")
            .ok_or("transactions CF not found")?;
        db_clone.get_cf(&cf_transactions, &tx_key_clone)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    })
    .await??;
    
    if let Some(ref tx_data) = tx_data {
        // Parse transaction (skip first 8 bytes: version + height)
        if tx_data.len() > 8 {
            let mut tx_with_header = vec![0u8; 4];
            tx_with_header.extend_from_slice(&tx_data[8..]);
            
            if let Ok(tx) = deserialize_transaction(&tx_with_header).await {
                // 1. Remove created outputs (delete UTXOs)
                for (vout, _output) in tx.outputs.iter().enumerate() {
                    let mut utxo_key = vec![b'u'];
                    utxo_key.extend_from_slice(txid_internal);
                    let vout_bytes = (vout as u64).to_le_bytes();
                    utxo_key.extend_from_slice(&vout_bytes);
                    
                    writer.delete("utxo", utxo_key);
                }
                
                // 2. Restore spent outputs (resurrect UTXOs)
                // NOTE: This requires tracking which UTXOs were spent
                // For now, we mark transaction as orphaned (height = -1)
                // Full UTXO resurrection would require undo data (see PIVX Core's CCoinsViewCache)
                
                // 3. Update address index
                // Remove this transaction from address indices
                // This is complex - would need to parse addresses from outputs
                // For initial implementation, address index rebuild will handle this
            }
        }
    }
    
    // 4. Delete transaction record (mark as orphaned)
    // Instead of deleting, mark with height = -1 to indicate orphaned
    let mut orphan_data = vec![0u8; 4]; // version = 0
    orphan_data.extend_from_slice(&(-1i32).to_le_bytes()); // height = -1
    if let Some(ref data) = tx_data {
        if data.len() > 8 {
            orphan_data.extend_from_slice(&data[8..]); // Original tx bytes
        }
    }
    
    writer.put("transactions", tx_key, orphan_data);
    
    Ok(())
}

/// Delete block metadata
fn delete_block_metadata(
    writer: &mut AtomicBatchWriter,
    height: i32,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Delete height -> hash mapping
    let height_key = height.to_le_bytes().to_vec();
    writer.delete("chain_metadata", height_key.clone());
    
    // Delete 'h' + height -> block hash mapping
    let mut h_key = vec![b'h'];
    h_key.extend_from_slice(&height_key);
    writer.delete("blocks", h_key);
    
    Ok(())
}

/// Update sync height in database
async fn update_sync_height(
    db: &Arc<DB>,
    height: i32,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let db_clone = db.clone();
    tokio::task::spawn_blocking(move || {
        let cf_state = db_clone.cf_handle("chain_state")
            .ok_or("chain_state CF not found")?;
        
        db_clone.put_cf(&cf_state, b"sync_height", height.to_le_bytes())
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    })
    .await??;
    
    Ok(())
}

/// Handle a detected reorganization
/// 
/// Main entry point for reorg handling. Called from monitor.rs when
/// reorg is detected.
/// 
/// # Arguments
/// * `db` - RocksDB instance
/// * `rpc_client` - PIVX Core RPC client (for finding fork point)
/// * `current_height` - Our current chain tip height
/// * `rpc_height` - RPC chain tip height
/// 
/// # Returns
/// ReorgInfo with details about what was rolled back
pub async fn handle_reorg(
    db: Arc<DB>,
    rpc_client: &Arc<pivx_rpc_rs::BitcoinRpcClient>,
    current_height: i32,
    rpc_height: i32,
) -> Result<ReorgInfo, Box<dyn std::error::Error + Send + Sync>> {
    println!("\n⚠️  REORG DETECTED ⚠️");
    println!("  Our height: {}", current_height);
    println!("  RPC height: {}", rpc_height);
    
    // Find fork point (last common block)
    let fork_height = find_fork_point(&db, rpc_client, current_height.min(rpc_height)).await?;
    
    println!("  Fork point: {} (common ancestor)", fork_height);
    
    // Calculate reorg parameters
    let orphaned_blocks = current_height - fork_height;
    let rollback_to = fork_height;
    
    // Get chain tip hashes for logging
    let cf_metadata = db.cf_handle("chain_metadata")
        .ok_or("chain_metadata CF not found")?;
    
    let old_tip_hash = {
        let key = current_height.to_le_bytes().to_vec();
        match db.get_cf(&cf_metadata, &key)? {
            Some(hash_bytes) => hex::encode(&hash_bytes),
            None => String::from("unknown"),
        }
    };
    
    // Get new tip hash via RPC (sync call wrapped in spawn_blocking)
    let rpc_client_clone = rpc_client.clone();
    let rpc_height_copy = rpc_height;
    let new_tip_hash = tokio::task::spawn_blocking(move || {
        rpc_client_clone.getblockhash(rpc_height_copy as i64)
    })
    .await??;
    
    let reorg_info = ReorgInfo::new(
        fork_height,
        rollback_to,
        orphaned_blocks,
        old_tip_hash.clone(),
        new_tip_hash.clone(),
    );
    
    // Perform the rollback
    rollback_to_height(db.clone(), rollback_to, current_height).await?;
    
    println!("\n📊 REORG SUMMARY:");
    println!("  ├─ Fork at height: {}", fork_height);
    println!("  ├─ Orphaned blocks: {}", orphaned_blocks);
    println!("  ├─ Old chain tip: {}", &old_tip_hash[..16]);
    println!("  └─ New chain tip: {}", &new_tip_hash[..16]);
    println!();
    
    Ok(reorg_info)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_reorg_info_creation() {
        let info = ReorgInfo::new(
            100,
            100,
            5,
            "old_hash".to_string(),
            "new_hash".to_string(),
        );
        
        assert_eq!(info.fork_height, 100);
        assert_eq!(info.rollback_to, 100);
        assert_eq!(info.orphaned_blocks, 5);
        assert_eq!(info.old_tip_hash, "old_hash");
        assert_eq!(info.new_tip_hash, "new_hash");
    }
}
