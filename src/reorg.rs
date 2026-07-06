use crate::address_rollback::rollback_address_index;
use crate::atomic_writer::AtomicBatchWriter;
use crate::constants::HEIGHT_ORPHAN;
use crate::metrics;
use crate::parser::deserialize_transaction;
use crate::spent_utxo::get_spent_utxo;
use crate::telemetry::{truncate_hex, ProgressCounter};
use rocksdb::DB;
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
use tracing::{info, info_span, warn};

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
    let cf_metadata = db
        .cf_handle("chain_metadata")
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
        let rpc_block_hash =
            tokio::task::spawn_blocking(move || rpc_client_clone.getblockhash(height_copy as i64))
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
    )
    .entered();
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
    match rollback_address_index(&mut writer, db.clone(), current_height, rollback_to_height).await
    {
        Ok(blocks_rolled_back) => {
            info!(blocks = blocks_rolled_back, "Address index rolled back");
        }
        Err(e) => {
            // Address rollback failure is critical - abort the entire reorg
            return Err(format!("FATAL: Address index rollback failed: {e}. Reorg aborted to prevent inconsistency.").into());
        }
    }

    // Final flush commits ALL remaining operations atomically (chain state +
    // address index) INCLUDING the sync_height watermark. Writing sync_height as
    // a separate put after the flush left a crash window where the watermark
    // still pointed at metadata the batch had just deleted (rollback deletes
    // top-down) — the monitor then error-looped on get_db_chain_tip forever,
    // with nothing repairing it on restart.
    writer.put(
        "chain_state",
        b"sync_height".to_vec(),
        rollback_to_height.to_le_bytes().to_vec(),
    );
    writer.flush().await?;

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
        let cf_transactions = db_clone
            .cf_handle("transactions")
            .ok_or("transactions CF not found")?;

        let mut txids = Vec::new();
        let height_bytes = height.to_le_bytes();

        // Iterate over 'B' + height + index keys
        let mut prefix = vec![b'B'];
        prefix.extend_from_slice(&height_bytes);

        let iter = db_clone.prefix_iterator_cf(&cf_transactions, &prefix);

        for item in iter {
            let (key, value) = item?;

            // Key format: 'B' + height(4) + index(8). The transactions CF uses a
            // fixed_prefix(1) extractor (main.rs:534), so prefix_iterator_cf over-scans
            // EVERY 'B' key past the seek — and LE heights are not numerically ordered,
            // so those foreign keys belong to scattered other heights. Bound to THIS
            // height's exact 5-byte prefix and break at the first non-match (the run is
            // contiguous); otherwise disconnect_transaction would orphan-mark unrelated
            // canonical txs on every reorg.
            if key.len() == 13 && key[0] == b'B' && key[1..5] == height_bytes[..] {
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
            } else {
                break; // sorted keys: first non-match ends this height's contiguous run
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
        let cf_transactions = db_clone
            .cf_handle("transactions")
            .ok_or("transactions CF not found")?;
        db_clone
            .get_cf(&cf_transactions, &tx_key_clone)
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
                            if let Ok(Some(spent_utxo)) =
                                get_spent_utxo(db.clone(), &prev_hash_bytes, prevout.n as u64).await
                            {
                                // Resurrect UTXO: restore it to the UTXO set
                                let mut utxo_key = vec![b'u'];
                                utxo_key.extend_from_slice(&spent_utxo.txid);
                                utxo_key.extend_from_slice(&spent_utxo.vout.to_le_bytes());

                                // Reconstruct UTXO value format:
                                // version (4 bytes) + height (4 bytes) + value (8 bytes) + script_len + script
                                let mut utxo_value = vec![0u8; 4]; // version = 0
                                utxo_value
                                    .extend_from_slice(&spent_utxo.created_height.to_le_bytes());
                                utxo_value.extend_from_slice(&spent_utxo.value.to_le_bytes());
                                utxo_value.extend_from_slice(
                                    &(spent_utxo.script_pubkey.len() as u32).to_le_bytes(),
                                );
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

    // 4. Orphan-mark the tx record (HEIGHT_ORPHAN) so it is no longer served as confirmed.
    // CRITICAL: only when we actually have the body to preserve. Writing a body-LESS 8-byte
    // record (version + height, no tx bytes) is exactly the stub that makes /tx return
    // "Empty transaction data". If the body isn't at this (display) key it lives untouched at
    // the internal key (initial-sync), and read_valid_tx_record serves it from there — so
    // skipping the write here never loses data, and never manufactures a phantom stub.
    if let Some(ref data) = tx_data {
        if data.len() > 8 {
            let mut orphan_data = vec![0u8; 4]; // version = 0
            orphan_data.extend_from_slice(&HEIGHT_ORPHAN.to_le_bytes()); // height = HEIGHT_ORPHAN
            orphan_data.extend_from_slice(&data[8..]); // preserve original tx bytes
            writer.put("transactions", tx_key, orphan_data);
        }
    }

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

    // Delete the block-transaction index ('B' + height(4) + tx_index(8) -> txid).
    // Reorg previously left these — so 'B' membership meant "was ever a tip" rather
    // than "is canonical now", polluting block-detail (which reads 'B') and any
    // height repair that trusts it. Remove them so 'B'+height reflects only the
    // current canonical block (the replacement block re-writes its own on connect).
    // Bounded to THIS height's exact 5-byte prefix — a delete must never over-scan.
    if let Some(cf_transactions) = db.cf_handle("transactions") {
        let mut prefix = vec![b'B'];
        prefix.extend_from_slice(&height_key);
        for item in db.prefix_iterator_cf(&cf_transactions, &prefix) {
            let (key, _) = item?;
            if key.len() == 13 && key[0] == b'B' && key[1..5] == height_key[..] {
                writer.delete("transactions", key.to_vec());
            } else {
                break; // sorted keys: first non-match ends this height's range
            }
        }
    }

    // Delete the forward height -> hash mapping.
    writer.delete("chain_metadata", height_key);

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
    )
    .entered();

    metrics::set_reorg_depth(orphaned_blocks as i64);

    // Get chain tip hashes for logging
    let cf_metadata = db
        .cf_handle("chain_metadata")
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
    let new_tip_hash =
        tokio::task::spawn_blocking(move || rpc_client_clone.getblockhash(rpc_height_copy as i64))
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

    // Temp DB with the CFs the disconnect tests touch.
    fn seed() -> (tempfile::TempDir, Arc<rocksdb::DB>) {
        use rocksdb::{Options, DB};
        let temp = tempfile::TempDir::new().unwrap();
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db = Arc::new(DB::open_cf(&opts, temp.path(), &["transactions", "utxo"]).unwrap());
        (temp, db)
    }

    #[test]
    fn test_reorg_info_creation() {
        let info = ReorgInfo::new(100, 100, 5, "old_hash".to_string(), "new_hash".to_string());

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
        assert!(
            db.get_cf(&cf, &height_key).unwrap().is_none(),
            "forward height->hash leaked"
        );
        assert!(
            db.get_cf(&cf, &h_display).unwrap().is_none(),
            "live-path 'h'+display hash leaked"
        );
        assert!(
            db.get_cf(&cf, &h_internal).unwrap().is_none(),
            "parse-path 'h'+internal hash leaked"
        );
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
        use crate::constants::HEIGHT_ORPHAN;

        let (_temp, db) = seed();
        let cf_tx = db.cf_handle("transactions").unwrap();

        let txid_display =
            hex::decode("aabbccddeeff00112233445566778899aabbccddeeff001122334455667788ff")
                .unwrap();
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
        disconnect_transaction(&mut writer, &db, &txid_display, height)
            .await
            .unwrap();
        writer.flush().await.unwrap();

        // Real tx (display key) is orphan-marked.
        let after = db
            .get_cf(&cf_tx, &tkey_display)
            .unwrap()
            .expect("tx record must still exist");
        let marked = i32::from_le_bytes([after[4], after[5], after[6], after[7]]);
        assert_eq!(
            marked, HEIGHT_ORPHAN,
            "real tx must be orphan-marked at the display key"
        );

        // No phantom stub at the wrong (internal) key.
        let mut tkey_internal = vec![b't'];
        tkey_internal.extend_from_slice(&txid_internal);
        assert!(
            db.get_cf(&cf_tx, &tkey_internal).unwrap().is_none(),
            "no phantom orphan stub at the internal key",
        );
    }

    /// Reorg must NEVER manufacture a body-less 8-byte stub. When the tx body lives only at
    /// the INTERNAL key (initial-sync style), disconnect reads the display key, finds nothing,
    /// and must SKIP the write — not write version+HEIGHT_ORPHAN (8 bytes) at the display key.
    /// That 8-byte record is exactly what makes /tx return "Empty transaction data".
    #[tokio::test]
    async fn disconnect_never_writes_bodyless_stub() {
        let (_temp, db) = seed();
        let cf_tx = db.cf_handle("transactions").unwrap();

        let txid_display =
            hex::decode("00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff")
                .unwrap();
        let txid_internal: Vec<u8> = txid_display.iter().rev().cloned().collect();

        // Body lives ONLY at the internal key (as initial-sync writes it).
        let height: i32 = 4200;
        let mut rec = vec![1u8, 0, 0, 0];
        rec.extend_from_slice(&height.to_le_bytes());
        rec.extend_from_slice(&[0x11, 0x22, 0x33, 0x44]); // body
        let mut tkey_internal = vec![b't'];
        tkey_internal.extend_from_slice(&txid_internal);
        db.put_cf(&cf_tx, &tkey_internal, &rec).unwrap();

        let mut writer = AtomicBatchWriter::new(db.clone(), 1000);
        disconnect_transaction(&mut writer, &db, &txid_display, height)
            .await
            .unwrap();
        writer.flush().await.unwrap();

        // No body-less stub manufactured at the display key.
        let mut tkey_display = vec![b't'];
        tkey_display.extend_from_slice(&txid_display);
        assert!(
            db.get_cf(&cf_tx, &tkey_display).unwrap().is_none(),
            "must not manufacture a body-less stub at the display key",
        );
        // The real body at the internal key is left intact.
        assert_eq!(
            db.get_cf(&cf_tx, &tkey_internal).unwrap().unwrap(),
            rec,
            "internal-key body must be left intact",
        );
    }

    /// Reorg rollback must DELETE the disconnected block's 'B' (block-tx) index
    /// entries, not just orphan-mark the tx records. Leaving them makes 'B'
    /// membership mean "was ever a tip", not "is canonical now" — polluting
    /// block-detail (which reads 'B') and any height repair that trusts it. Only
    /// THIS height's entries may be removed; a neighbour's must survive.
    #[tokio::test]
    async fn delete_block_metadata_removes_b_index_for_height_only() {
        use rocksdb::{Options, SliceTransform, DB};
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        // Match production (main.rs:534): fixed_prefix(1) makes prefix_iterator_cf
        // over-scan every 'B' key past the seek — so the neighbour survives ONLY if
        // the height guard + break actually bound the delete.
        opts.set_prefix_extractor(SliceTransform::create_fixed_prefix(1));
        let db =
            Arc::new(DB::open_cf(&opts, temp.path(), &["transactions", "chain_metadata"]).unwrap());
        let cf = db.cf_handle("transactions").unwrap();

        let height: i32 = 5000;
        let bkey = |h: i32, idx: u64| {
            let mut k = vec![b'B'];
            k.extend(&h.to_le_bytes());
            k.extend(&idx.to_le_bytes());
            k
        };
        let (b0, b1, b2) = (bkey(height, 0), bkey(height, 1), bkey(height, 2));
        let neighbour = bkey(height + 1, 0); // a different block's entry must survive
        for k in [&b0, &b1, &b2, &neighbour] {
            db.put_cf(&cf, k, b"deadbeefcafe").unwrap();
        }

        let mut writer = AtomicBatchWriter::new(db.clone(), 1000);
        delete_block_metadata(&mut writer, &db, height).unwrap();
        writer.flush().await.unwrap();

        assert!(db.get_cf(&cf, &b0).unwrap().is_none(), "'B'+H+0 leaked");
        assert!(db.get_cf(&cf, &b1).unwrap().is_none(), "'B'+H+1 leaked");
        assert!(db.get_cf(&cf, &b2).unwrap().is_none(), "'B'+H+2 leaked");
        assert!(
            db.get_cf(&cf, &neighbour).unwrap().is_some(),
            "neighbouring height's 'B' entry must NOT be deleted"
        );
    }

    /// get_block_transactions(H) must return ONLY height H's txids. Under the
    /// production fixed_prefix(1) extractor, prefix_iterator_cf over-scans every
    /// 'B' key past the seek (LE heights are not numerically ordered), so without a
    /// height guard it returns foreign heights' txids — which disconnect_transaction
    /// would then orphan-mark, corrupting unrelated canonical blocks on every reorg.
    #[tokio::test]
    async fn get_block_transactions_does_not_overscan_other_heights() {
        use rocksdb::{Options, SliceTransform, DB};
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        opts.set_prefix_extractor(SliceTransform::create_fixed_prefix(1));
        let db = Arc::new(DB::open_cf(&opts, temp.path(), &["transactions"]).unwrap());
        let cf = db.cf_handle("transactions").unwrap();

        // 5001's LE key ([0x89,0x13,..]) sorts AFTER 5000's ([0x88,0x13,..]), so a
        // seek at 5000 over-scans into 5001.
        let (target, foreign) = (5000i32, 5001i32);
        let txid_target = "aa".repeat(32);
        let txid_foreign = "bb".repeat(32);
        let bput = |h: i32, hex_txid: &str| {
            let mut k = vec![b'B'];
            k.extend(&h.to_le_bytes());
            k.extend(&0u64.to_le_bytes());
            db.put_cf(&cf, &k, hex_txid.as_bytes()).unwrap();
        };
        bput(target, &txid_target);
        bput(foreign, &txid_foreign);

        let txids = get_block_transactions(&db, target).await.unwrap();
        assert_eq!(txids.len(), 1, "must not over-scan into height {foreign}");
        assert_eq!(hex::encode(&txids[0]), txid_target);
    }
}
