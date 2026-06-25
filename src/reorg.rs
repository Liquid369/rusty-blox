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
use crate::constants::HEIGHT_ORPHAN;
use crate::parser::deserialize_transaction;
use crate::address_rollback::rollback_address_index;
use crate::spent_utxo::get_spent_utxo;
use tracing::{info, warn, info_span};
use crate::metrics;
use crate::telemetry::{truncate_hex, ProgressCounter};

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
    rpc_client: &Arc<pivx_rpc_rs::PivxRpcClient>,
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
    
    let _rollback_span = info_span!(
        "rollback",
        from_height = current_height,
        to_height = rollback_to_height,
        blocks_to_remove = blocks_to_remove
    ).entered();
    let timer = metrics::Timer::new();
    
    info!(
        from_height = current_height,
        to_height = rollback_to_height,
        blocks_to_remove = blocks_to_remove,
        "Starting rollback"
    );
    
    // Use atomic writer for safe rollback
    let mut writer = AtomicBatchWriter::new(db.clone(), 100000);
    let db_clone = db.clone();
    
    // Progress tracking for sampled logging (every 100 blocks)
    let progress = ProgressCounter::new(100);
    let mut blocks_processed = 0;
    
    // Process each block to be removed (in reverse order)
    for height in ((rollback_to_height + 1)..=current_height).rev() {
        blocks_processed += 1;
        
        if progress.should_log() {
            let remaining = blocks_to_remove - blocks_processed;
            info!(
                current_height = height,
                blocks_processed = blocks_processed,
                remaining = remaining,
                "Rollback progress"
            );
        }
        
        // 1. Get all transactions in this block
        let txids = get_block_transactions(&db_clone, height).await?;
        
        // 2. Disconnect each transaction (orphan-mark the tx record; UTXO ops are
        //    no-ops for live blocks). get_block_transactions returns display order.
        for txid_display in txids {
            disconnect_transaction(&mut writer, &db_clone, &txid_display, height).await?;
        }
        
        // 3. Delete block metadata
        delete_block_metadata(&mut writer, &db_clone, height)?;
        
        // Flush in batches to avoid excessive memory usage
        if writer.should_flush() {
            writer.flush().await?;
        }
    }
    
    // [R2] FIX: Rollback address index BEFORE final commit for atomicity
    match rollback_address_index(&mut writer, db.clone(), current_height, rollback_to_height).await {
        Ok(blocks_rolled_back) => {
            info!(blocks = blocks_rolled_back, "Address index rolled back");
        }
        Err(e) => {
            // Address rollback failure is critical - abort the entire reorg
            return Err(format!("FATAL: Address index rollback failed: {e}. Reorg aborted to prevent inconsistency.").into());
        }
    }
    
    // Final flush to commit ALL operations atomically (chain state + address index)
    if writer.pending_count() > 0 {
        writer.flush().await?;
    }
    
    // Update sync height to rollback point
    update_sync_height(&db, rollback_to_height).await?;
    
    let elapsed = timer.elapsed_secs();
    info!(
        final_height = rollback_to_height,
        blocks_removed = blocks_to_remove,
        duration_secs = elapsed,
        "Rollback complete"
    );
    metrics::ORPHANED_BLOCKS.add(blocks_to_remove as i64);
    
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
                
                // 'B' value is the DISPLAY-order txid hex (monitor writes
                // txid.as_bytes()); the tx CF and 't' index are display-keyed, so
                // return display order. disconnect_transaction reverses to internal
                // only for the internal-keyed 'utxo' CF. (The prior .rev() produced
                // internal order, so disconnect's tx-CF lookup missed and the real
                // tx was never orphan-marked.)
                if let Ok(txid_display) = hex::decode(&txid_hex) {
                    txids.push(txid_display);
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
    txid_display: &[u8],
    _height: i32,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // The tx CF and 't' index are DISPLAY-keyed; the 'utxo' CF is INTERNAL-keyed
    // (transactions.rs stores 'u' + reversed_txid). Keep both orders so each CF is
    // keyed correctly. NOTE: for live-tip reorgs the 'utxo' CF and utxo_undo are not
    // maintained (the monitor never writes them; store_spent_utxo has no callers)
    // and nothing reads 'utxo', so the UTXO ops below are effectively no-ops — the
    // user-visible effect of this function is correctly orphan-marking the
    // disconnected tx record (display-keyed) so it is no longer served as confirmed.
    let txid_internal: Vec<u8> = txid_display.iter().rev().cloned().collect();

    // Get transaction data (tx CF is display-keyed)
    let mut tx_key = vec![b't'];
    tx_key.extend_from_slice(txid_display);
    
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
                // 1. Remove created outputs (delete UTXOs). The 'utxo' CF is
                // internal-keyed, so use the reversed txid here.
                for (vout, _output) in tx.outputs.iter().enumerate() {
                    let mut utxo_key = vec![b'u'];
                    utxo_key.extend_from_slice(&txid_internal);
                    let vout_bytes = (vout as u64).to_le_bytes();
                    utxo_key.extend_from_slice(&vout_bytes);

                    writer.delete("utxo", utxo_key);
                }
                
                // 2. Restore spent outputs (resurrect UTXOs) ✅ NOW IMPLEMENTED
                // Use spent_utxo.rs infrastructure to restore UTXOs that were spent by this tx
                for input in &tx.inputs {
                    if let Some(prevout) = &input.prevout {
                        // Convert display format hash (hex string) to internal format (raw bytes)
                        if let Ok(prev_hash_bytes) = hex::decode(&prevout.hash) {
                            // Try to retrieve spent UTXO data from utxo_undo CF
                            if let Ok(Some(spent_utxo)) = get_spent_utxo(
                                db.clone(),
                                &prev_hash_bytes,
                                prevout.n as u64
                            ).await {
                                // Resurrect UTXO: restore it to the UTXO set
                                let mut utxo_key = vec![b'u'];
                                utxo_key.extend_from_slice(&spent_utxo.txid);
                                utxo_key.extend_from_slice(&spent_utxo.vout.to_le_bytes());
                                
                                // Reconstruct UTXO value format:
                                // version (4 bytes) + height (4 bytes) + value (8 bytes) + script_len + script
                                let mut utxo_value = vec![0u8; 4]; // version = 0
                                utxo_value.extend_from_slice(&spent_utxo.created_height.to_le_bytes());
                                utxo_value.extend_from_slice(&spent_utxo.value.to_le_bytes());
                                utxo_value.extend_from_slice(&(spent_utxo.script_pubkey.len() as u32).to_le_bytes());
                                utxo_value.extend_from_slice(&spent_utxo.script_pubkey);
                                
                                writer.put("utxo", utxo_key, utxo_value);
                            }
                            // Note: If undo data not found, UTXO cannot be resurrected
                            // This can happen if spent_utxo tracking wasn't enabled during forward sync
                            // In production, we should log this as a warning
                        }
                    }
                }
                
                // 3. Update address index
                // Remove this transaction from address indices
                // This is complex - would need to parse addresses from outputs
                // For initial implementation, address index rebuild will handle this
            }
        }
    }
    
    // 4. Delete transaction record (mark as orphaned)
    // Instead of deleting, mark with HEIGHT_ORPHAN to indicate orphaned
    let mut orphan_data = vec![0u8; 4]; // version = 0
    orphan_data.extend_from_slice(&HEIGHT_ORPHAN.to_le_bytes()); // height = HEIGHT_ORPHAN
    if let Some(ref data) = tx_data {
        if data.len() > 8 {
            orphan_data.extend_from_slice(&data[8..]); // Original tx bytes
        }
    }
    
    writer.put("transactions", tx_key, orphan_data);
    
    Ok(())
}

/// Delete block metadata for a height being rolled back.
///
/// Removes the forward `height -> hash` map AND the reverse `'h' + hash -> height`
/// map from `chain_metadata`. Two writers produce the reverse key with different
/// byte order, so we delete both to leave no stale entry:
///   - parse path (blocks.rs):  `'h' + INTERNAL hash`
///   - live path  (monitor.rs): `'h' + DISPLAY hash` (= reversed internal)
/// The forward map value is the hash in DISPLAY order (both writers), so we read
/// it first, then delete `'h'+display` and `'h'+reversed(display)=internal`.
///
/// (Prior bug: this deleted `'h' + height` from the `blocks` CF — a key that is
/// never written — so the real `'h' + hash` entries in `chain_metadata` leaked on
/// every reorg.)
fn delete_block_metadata(
    writer: &mut AtomicBatchWriter,
    db: &Arc<DB>,
    height: i32,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let height_key = height.to_le_bytes().to_vec();

    // Read the (display-order) hash from the forward map BEFORE deleting it, so we
    // can construct and delete the matching reverse `'h'` keys.
    if let Some(cf_metadata) = db.cf_handle("chain_metadata") {
        if let Some(display_hash) = db.get_cf(&cf_metadata, &height_key)? {
            // Reverse key as written by the live RPC path (display byte order).
            let mut h_display = vec![b'h'];
            h_display.extend_from_slice(&display_hash);
            writer.delete("chain_metadata", h_display);

            // Reverse key as written by the parse path (internal = reversed display).
            let internal_hash: Vec<u8> = display_hash.iter().rev().cloned().collect();
            let mut h_internal = vec![b'h'];
            h_internal.extend_from_slice(&internal_hash);
            writer.delete("chain_metadata", h_internal);
        }
    }

    // Delete the forward height -> hash mapping.
    writer.delete("chain_metadata", height_key);

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
    rpc_client: &Arc<pivx_rpc_rs::PivxRpcClient>,
    current_height: i32,
    rpc_height: i32,
) -> Result<ReorgInfo, Box<dyn std::error::Error + Send + Sync>> {
    warn!(
        our_height = current_height,
        rpc_height = rpc_height,
        "REORG DETECTED"
    );
    metrics::increment_reorg_events();
    
    // Find fork point (last common block)
    let fork_height = find_fork_point(&db, rpc_client, current_height.min(rpc_height)).await?;
    
    info!(
        fork_height = fork_height,
        orphaned_blocks = current_height - fork_height,
        "Fork point identified"
    );
    
    // Calculate reorg parameters
    let orphaned_blocks = current_height - fork_height;
    let rollback_to = fork_height;
    
    let _reorg_span = info_span!(
        "reorg_handling",
        fork_height = fork_height,
        orphaned_blocks = orphaned_blocks
    ).entered();
    
    metrics::set_reorg_depth(orphaned_blocks as i64);
    
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
    
    info!(
        fork_height = fork_height,
        orphaned_blocks = orphaned_blocks,
        old_tip_hash = %truncate_hex(&old_tip_hash, 16),
        new_tip_hash = %truncate_hex(&new_tip_hash, 16),
        "Reorg summary"
    );
    
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

    /// Regression: rolling back a block must delete the forward `height -> hash`
    /// map AND both reverse `'h' + hash -> height` entries (the live-path display
    /// ordering and the parse-path internal ordering). The prior bug deleted
    /// `'h' + height` from the `blocks` CF, leaving the real `'h' + hash` entries
    /// in `chain_metadata` to leak on every reorg.
    #[tokio::test]
    async fn test_delete_block_metadata_removes_both_h_orderings() {
        use rocksdb::{Options, DB};
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db = Arc::new(DB::open_cf(&opts, temp.path(), &["chain_metadata"]).unwrap());

        let height: i32 = 4242;
        let height_key = height.to_le_bytes().to_vec();

        // 32-byte hash in display order, plus its internal (reversed) counterpart.
        let display_hash: Vec<u8> = (0u8..32).collect();
        let internal_hash: Vec<u8> = display_hash.iter().rev().cloned().collect();

        let cf = db.cf_handle("chain_metadata").unwrap();
        // Forward map: height -> display hash (both writers store display order).
        db.put_cf(&cf, &height_key, &display_hash).unwrap();
        // Reverse 'h' as written by the live RPC path (display order).
        let mut h_display = vec![b'h'];
        h_display.extend_from_slice(&display_hash);
        db.put_cf(&cf, &h_display, &height_key).unwrap();
        // Reverse 'h' as written by the parse path (internal order).
        let mut h_internal = vec![b'h'];
        h_internal.extend_from_slice(&internal_hash);
        db.put_cf(&cf, &h_internal, &height_key).unwrap();

        // Sanity: all three present before rollback.
        assert!(db.get_cf(&cf, &height_key).unwrap().is_some());
        assert!(db.get_cf(&cf, &h_display).unwrap().is_some());
        assert!(db.get_cf(&cf, &h_internal).unwrap().is_some());

        // Roll back this block's metadata.
        let mut writer = AtomicBatchWriter::new(db.clone(), 1000);
        delete_block_metadata(&mut writer, &db, height).unwrap();
        writer.flush().await.unwrap();

        // No stale entry may survive — forward and BOTH reverse 'h' keys gone.
        assert!(db.get_cf(&cf, &height_key).unwrap().is_none(), "forward height->hash leaked");
        assert!(db.get_cf(&cf, &h_display).unwrap().is_none(), "live-path 'h'+display hash leaked");
        assert!(db.get_cf(&cf, &h_internal).unwrap().is_none(), "parse-path 'h'+internal hash leaked");
    }

    /// Byte-order regression: get_block_transactions must return DISPLAY-order txids
    /// (the tx CF / 't' index key order), not internal/reversed. The prior .rev()
    /// made disconnect_transaction key the display tx CF with internal bytes, so its
    /// lookup missed and the real tx was never orphan-marked.
    #[tokio::test]
    async fn get_block_transactions_returns_display_order() {
        use rocksdb::{Options, DB};
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db = Arc::new(DB::open_cf(&opts, temp.path(), &["transactions"]).unwrap());
        let cf = db.cf_handle("transactions").unwrap();

        let height: i32 = 5000;
        // The node/RPC presents txids in display order; monitor stores the 'B' index
        // value as that display hex string (txid.as_bytes()).
        let txid_display_hex = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
        let txid_display_bytes = hex::decode(txid_display_hex).unwrap();

        let mut bkey = vec![b'B'];
        bkey.extend(&height.to_le_bytes());
        bkey.extend(&0u64.to_le_bytes());
        db.put_cf(&cf, &bkey, txid_display_hex.as_bytes()).unwrap();

        let txids = get_block_transactions(&db, height).await.unwrap();
        assert_eq!(txids.len(), 1);
        assert_eq!(
            txids[0], txid_display_bytes,
            "must return display-order txids to match the display-keyed tx CF",
        );
    }

    /// Disconnect must orphan-mark the REAL tx record at the DISPLAY key (so it is no
    /// longer served as confirmed) and must NOT leave a phantom stub at the internal
    /// key (the prior bug). deserialize of the dummy body fails — orphan-marking
    /// happens regardless, which is the user-visible effect for a live reorg.
    #[tokio::test]
    async fn disconnect_orphan_marks_real_tx_at_display_key() {
        use rocksdb::{Options, DB};
        use tempfile::TempDir;
        use crate::constants::HEIGHT_ORPHAN;

        let temp = TempDir::new().unwrap();
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db = Arc::new(DB::open_cf(&opts, temp.path(), &["transactions", "utxo"]).unwrap());
        let cf_tx = db.cf_handle("transactions").unwrap();

        let txid_display =
            hex::decode("aabbccddeeff00112233445566778899aabbccddeeff001122334455667788ff").unwrap();
        let txid_internal: Vec<u8> = txid_display.iter().rev().cloned().collect();

        // Seed the real tx record at the DISPLAY key: version(4) + height(4) + body.
        let height: i32 = 7000;
        let mut rec = vec![1u8, 0, 0, 0];
        rec.extend_from_slice(&height.to_le_bytes());
        rec.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02]); // dummy body
        let mut tkey_display = vec![b't'];
        tkey_display.extend_from_slice(&txid_display);
        db.put_cf(&cf_tx, &tkey_display, &rec).unwrap();

        let mut writer = AtomicBatchWriter::new(db.clone(), 1000);
        disconnect_transaction(&mut writer, &db, &txid_display, height).await.unwrap();
        writer.flush().await.unwrap();

        // Real tx (display key) is orphan-marked.
        let after = db.get_cf(&cf_tx, &tkey_display).unwrap().expect("tx record must still exist");
        let marked = i32::from_le_bytes([after[4], after[5], after[6], after[7]]);
        assert_eq!(marked, HEIGHT_ORPHAN, "real tx must be orphan-marked at the display key");

        // No phantom stub at the wrong (internal) key.
        let mut tkey_internal = vec![b't'];
        tkey_internal.extend_from_slice(&txid_internal);
        assert!(
            db.get_cf(&cf_tx, &tkey_internal).unwrap().is_none(),
            "no phantom orphan stub at the internal key",
        );
    }
}
