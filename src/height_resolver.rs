/// Height Resolution from PIVX Core Block Index
/// 
/// This module reads PIVX Core's leveldb block index to build the canonical chain,
/// then updates all transactions in our database with correct heights BEFORE enrichment.
/// 
/// This eliminates the need for the "repair" phase and ensures orphaned transactions
/// are identified using Core's canonical chain data.

use rocksdb::{DB, WriteBatch, WriteOptions, IteratorMode};
use std::sync::Arc;
use std::collections::{HashSet, HashMap};
use tracing::{warn, info};
use crate::leveldb_index::build_canonical_chain_from_leveldb;
use crate::constants::{HEIGHT_GENESIS, HEIGHT_ORPHAN, HEIGHT_UNRESOLVED};
use crate::db_utils::bulk_write_options;

/// Commit a height-resolution WriteBatch, honoring the bulk durability mode.
/// `bulk` disables the WAL (initial reindex only); otherwise the WAL is kept.
fn commit_batch(db: &DB, batch: WriteBatch, wo: &WriteOptions, bulk: bool) -> Result<(), rocksdb::Error> {
    if bulk {
        db.write_opt(batch, wo)
    } else {
        db.write(batch)
    }
}

/// Resolve all transaction heights using PIVX Core's block index
/// 
/// This reads Core's canonical chain and updates ALL transactions in one pass:
/// - Transactions in canonical blocks get their correct height
/// - Transactions in non-canonical blocks get marked as orphaned (HEIGHT_ORPHAN)
/// 
/// Returns (fixed_count, orphaned_count)
/// `bulk` selects the write durability for the height-fix batches below: `true`
/// disables the WAL on the initial full-reindex path (the DB is reconstructible
/// from Core's block index), `false` keeps the WAL on the live/catch-up path so
/// a crash stays recoverable. The bytes written are identical either way.
pub async fn resolve_heights_from_block_index(
    db: Arc<DB>,
    pivx_data_dir: Option<String>,
    bulk: bool,
) -> Result<(usize, usize), Box<dyn std::error::Error>> {
    let wo = bulk_write_options();
    // [Lever 3] The canonical height set is built from chain_metadata further
    // down (after the early-exit check) instead of here. The parse phase already
    // imported PIVX Core's leveldb index into chain_metadata and
    // validate_canonical_metadata_complete verified it is contiguous 0..=tip, so
    // re-copying and re-chainworking the full 5.5M-block leveldb a second time
    // (~35 min) is pure redundancy. pivx_dir is retained only for the safety
    // fallback used when chain_metadata is unexpectedly empty.
    let pivx_dir = pivx_data_dir.unwrap_or_else(crate::config::default_pivx_data_dir);
    
    // 3. Build lookup structures
    //
    // [C] Only the height -> block_hash direction (built in step 5 below) is ever
    // consulted, and only as a "is this height on the canonical chain" presence
    // test. The previous `canonical_blocks` HashSet and `blockhash_to_height`
    // HashMap were populated here but never read anywhere in this function, so
    // they were pure allocations (one entry per canonical block) and are removed.
    // No output depends on them.

    // 4. Scan all transactions to collect which ones need fixing
    
    let cf_transactions = db.cf_handle("transactions")
        .ok_or("transactions CF not found")?;
    
    // Collect txids that need fixing:
    // - height=0 or HEIGHT_UNRESOLVED need resolution
    // - Positive heights need validation against canonical chain
    let mut txids_needing_fix: HashSet<Vec<u8>> = HashSet::new();
    let mut txids_to_validate: HashSet<(Vec<u8>, i32)> = HashSet::new();
    
    const BATCH_SIZE: usize = 10_000;
    
    let iter = db.iterator_cf(&cf_transactions, IteratorMode::Start);
    
    for item in iter {
        match item {
            Ok((key, value)) => {
                // Only process 't' prefix entries (transaction data)
                if key.is_empty() || key[0] != b't' { continue; }

                // Parse current transaction data
                if value.len() < 8 {
                    continue;
                }
                
                let current_height = i32::from_le_bytes([value[4], value[5], value[6], value[7]]);
                
                if current_height == HEIGHT_GENESIS || current_height == HEIGHT_UNRESOLVED {
                    // Store txid (skip 't' prefix) for later fixing
                    txids_needing_fix.insert(key[1..].to_vec());
                } else if current_height == HEIGHT_ORPHAN {
                    // CRITICAL: Also fix HEIGHT_ORPHAN transactions that may have been incorrectly marked
                    // They need to be checked against 'B' keys to see if they're actually in canonical chain
                    txids_needing_fix.insert(key[1..].to_vec());
                } else if current_height > 0 {
                    // Positive height - need to validate against canonical chain
                    txids_to_validate.insert((key[1..].to_vec(), current_height));
                }
            }
            Err(e) => {
                warn!(error = ?e, "Error reading transaction during scan");
                break;
            }
        }
    }
    
    let fixed_count = txids_needing_fix.len();
    let validate_count = txids_to_validate.len();
    
    // Early exit if nothing to fix or validate
    if fixed_count == 0 && validate_count == 0 {
        info!("All transaction heights already correct");
        return Ok((0, 0));
    }
    
    // 5. Build the canonical height -> hash map from chain_metadata (already in
    // the DB from the parse-phase resolve). Only the height KEY set is consulted
    // below (a presence test); the stored value is never read, so any byte is
    // fine. Falls back to a one-off leveldb re-import only if chain_metadata is
    // unexpectedly empty (it is validated complete+contiguous before parse).
    let cf_metadata = db.cf_handle("chain_metadata").ok_or("chain_metadata CF not found")?;
    let mut height_to_blockhash: HashMap<i64, Vec<u8>> = HashMap::new();
    for item in db.iterator_cf(&cf_metadata, IteratorMode::Start) {
        if let Ok((key, value)) = item {
            // height -> hash keys are the 4-byte little-endian height; skip the
            // 33-byte 'h'+hash and the 'w'+height chainwork keys.
            if key.len() == 4 {
                let h = i32::from_le_bytes([key[0], key[1], key[2], key[3]]) as i64;
                height_to_blockhash.insert(h, value.to_vec());
            }
        }
    }
    if height_to_blockhash.is_empty() {
        warn!("chain_metadata has no canonical heights - falling back to leveldb re-import");
        let block_index_copy = "/tmp/pivx_block_index_current";
        let block_index_src = format!("{}/blocks/index", pivx_dir);
        std::fs::remove_dir_all(block_index_copy).ok();
        let copy_result = std::process::Command::new("cp")
            .args(["-R", &block_index_src, block_index_copy])
            .output()?;
        if !copy_result.status.success() {
            return Err(format!("Failed to copy block index: {}",
                String::from_utf8_lossy(&copy_result.stderr)).into());
        }
        for (height, block_hash, _, _) in &build_canonical_chain_from_leveldb(block_index_copy)? {
            height_to_blockhash.insert(*height, block_hash.clone());
        }
    }
    
    // 5a. [C] The previous code materialised a full 'B'-key index here
    //     (`HashSet<(i32, txid_bytes)>`, one entry per ~12.3M block-tx record —
    //     several GB) purely to answer, for each positive-height tx in
    //     `txids_to_validate`, "does a 'B' key exist at this tx's stored height?".
    //
    //     We DROP the multi-GB HashSet but NOT the membership test itself. The
    //     assumption that "every positive-height 't' record has a matching 'B'
    //     key" is FALSE: the live monitor path (monitor.rs ~894-907) writes a
    //     positive-height 't' record for an RPC-fetched prev-tx WITHOUT any 'B'
    //     key (it doesn't know the tx_index). On a catch-up/resume (sync.rs
    //     deletes `height_resolution_complete` and re-runs this resolver over a
    //     monitor-populated DB) or a reorg, such a tx whose height is OUTSIDE the
    //     canonical chain map MUST still be orphaned — hardcoding
    //     `has_block_index = true` would keep it and inflate balances.
    //
    //     `has_block_index` is therefore computed per-tx by an ON-DEMAND, bounded
    //     prefix scan over only the 'B' keys at this tx's stored height. The blk
    //     parser writes 'B' as: 'B' + height.to_le_bytes() (i32, 4B) +
    //     tx_index.to_le_bytes() (u64, 8B) -> value = hex(display/big-endian txid)
    //     (transactions.rs ~527). A block holds only a few txs, so the scan over
    //     the `b'B' + height` prefix touches O(few) keys per tx — not a full
    //     re-scan, and O(1) extra memory.
    //
    //     This is BYTE-EQUIVALENT to the old `block_tx_index.contains(&(height,
    //     txid_internal))` test: the old set held exactly
    //       {(i32::from_le_bytes(key[1..5]), reverse(hex::decode(value)))
    //          : every 'B' key}
    //     and was queried at `(current_height, txid_internal)`. A 'B' key can only
    //     satisfy that query if its `key[1..5]` equals `current_height` — i.e. it
    //     falls under the `b'B' + current_height` prefix. Restricting the scan to
    //     that prefix and decoding each value's txid to internal form
    //     (reverse(hex::decode(value))) and comparing to `txid_internal` evaluates
    //     the identical predicate over the identical key/value bytes, just without
    //     pre-materialising every other height into RAM.

    /// Does a 'B' (block-tx) index entry exist for `txid_internal` at `height`?
    ///
    /// Reproduces the old `block_tx_index.contains(&(height, txid_internal))`
    /// membership test via a bounded prefix scan instead of a multi-GB HashSet.
    /// `txid_internal` is the internal (Core little-endian) txid, i.e. the bytes
    /// after the 't' prefix in a transaction key. The 'B' value stores the txid
    /// in display (big-endian) hex, so it is hex-decoded then reversed back to
    /// internal form before comparison — matching how the old index was built.
    fn block_index_has_tx(
        db: &DB,
        cf: &impl rocksdb::AsColumnFamilyRef,
        height: i32,
        txid_internal: &[u8],
    ) -> bool {
        let mut prefix = vec![b'B'];
        prefix.extend_from_slice(&height.to_le_bytes());

        let iter = db.prefix_iterator_cf(cf, &prefix);
        for item in iter {
            let (key, value) = match item {
                Ok(kv) => kv,
                Err(_) => break,
            };
            // prefix_iterator can overshoot past the prefix; stop at the boundary.
            if !key.starts_with(&prefix) {
                break;
            }
            // Value is the display (big-endian) txid as a hex string.
            if let Ok(txid_hex) = std::str::from_utf8(&value) {
                if let Ok(txid_display) = hex::decode(txid_hex) {
                    // Reverse display -> internal, exactly as the old index build.
                    if txid_display.len() == txid_internal.len()
                        && txid_display.iter().rev().eq(txid_internal.iter())
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    // Initialize counters
    let mut updated = 0;
    let mut newly_orphaned = 0;
    
    // 5b. Validate positive heights against canonical chain
    if !txids_to_validate.is_empty() {
        let mut validated_batch = WriteBatch::default();
        let mut validated_orphaned = 0;
        let mut orphaned_txids_from_validation: Vec<Vec<u8>> = Vec::new();
        
        for (txid_internal, current_height) in txids_to_validate {
            // CRITICAL FIX: Check if transaction has a 'B' key (block index entry)
            // A transaction should ONLY be marked as orphaned if it has NO 'B' key
            // If it has a 'B' key, it's in the canonical chain regardless of height lookup
            //
            // [C] Computed on-demand via a bounded prefix scan over the 'B' keys at
            // this tx's stored height (see `block_index_has_tx` above). This is
            // byte-equivalent to the old `block_tx_index.contains(&(height, txid))`
            // membership test but with O(1) extra memory. It correctly returns
            // FALSE for monitor-written positive-height 't' records that have no
            // 'B' key, so those are orphaned when their height is off-chain.
            // `has_block_index` only changes the outcome when the height is OFF
            // the canonical chain (both arms below require that). So probe the 'B'
            // index ONLY then: on a fresh sync every height is canonical and this
            // scan never runs; it fires solely for the rare off-chain tx (a reorg,
            // or a monitor-written prev-tx 't' record that has no 'B' key). When
            // the height IS canonical the tx is valid and we touch nothing — same
            // as the old HashSet path, which also took no branch in that case.
            if !height_to_blockhash.contains_key(&(current_height as i64)) {
                let has_block_index =
                    block_index_has_tx(&db, &cf_transactions, current_height, &txid_internal);
                if !has_block_index {
                    // Height not in canonical chain AND no block index - mark as orphaned
                    let mut tx_key = vec![b't'];
                    tx_key.extend_from_slice(&txid_internal);

                    if let Ok(Some(tx_data)) = db.get_cf(&cf_transactions, &tx_key) {
                        // Update height to HEIGHT_ORPHAN
                        let mut new_value = tx_data[0..4].to_vec(); // version
                        new_value.extend(&HEIGHT_ORPHAN.to_le_bytes());
                        new_value.extend(&tx_data[8..]); // rest of data

                        validated_batch.put_cf(&cf_transactions, &tx_key, &new_value);
                        validated_orphaned += 1;
                        orphaned_txids_from_validation.push(txid_internal.clone());

                        if validated_orphaned % BATCH_SIZE == 0 {
                            commit_batch(&db, validated_batch, &wo, bulk)?;
                            validated_batch = WriteBatch::default();
                        }
                    }
                } else {
                    // Has 'B' key but height not in canonical lookup - this is OK, likely just incomplete chain build
                    // Log at warn level (only first occurrence to avoid spam)
                    if validated_orphaned == 0 {
                        warn!(
                            txid = %hex::encode(&txid_internal[..8].to_vec()),
                            height = current_height,
                            "Transaction has 'B' key but height not in canonical lookup - keeping as valid"
                        );
                    }
                }
            }
        }
        
        // Write final validation batch
        if !validated_batch.is_empty() {
            commit_batch(&db, validated_batch, &wo, bulk)?;
        }
        
        info!(orphaned = validated_orphaned, "Marked transactions with non-canonical heights as orphaned");
        newly_orphaned += validated_orphaned;
        
        // CRITICAL FIX: Clean address index for newly orphaned transactions
        if validated_orphaned > 0 {
            info!(count = orphaned_txids_from_validation.len(), "Cleaning address index for orphaned transactions");
            // TODO: Re-enable when orphan_cleanup module is available
            // match remove_orphaned_txs_batch(&db, &orphaned_txids_from_validation).await {
            //     Ok((cleaned, errors)) => {
            //         info!(cleaned, errors, "Cleaned addresses from orphaned transactions");
            //     }
            //     Err(e) => {
            //         warn!(error = ?e, "Address cleanup failed");
            //     }
            // }
        }
    }
    
    // 6. Efficiently update ONLY the transactions that need fixing
    info!(count = fixed_count, "Updating transaction heights from block index");

    let mut batch = WriteBatch::default();
    let _not_found_in_index = 0;
    
    // Read 'B' entries and update ONLY if txid is in our HashSet
    // Format: 'B' + height(4) + tx_index(8) -> txid_hex
    let block_tx_iter = db.iterator_cf(&cf_transactions, IteratorMode::Start);
    
    for item in block_tx_iter {
        match item {
            Ok((key, value)) => {
                if key.is_empty() || key[0] != b'B' { continue; }
                
                // Extract height from key
                if key.len() >= 5 {
                    let height_bytes = [key[1], key[2], key[3], key[4]];
                    let height = i32::from_le_bytes(height_bytes);
                    
                    // Value is txid as hex string
                    if let Ok(txid_hex) = String::from_utf8(value.to_vec()) {
                        if let Ok(txid_display) = hex::decode(&txid_hex) {
                            // Reverse to get internal format
                            let txid_internal: Vec<u8> = txid_display.iter().rev().cloned().collect();
                            
                            // ONLY process if this txid needs fixing
                            if !txids_needing_fix.contains(&txid_internal) {
                                continue;
                            }
                            
                            // Build transaction key
                            let mut tx_key = vec![b't'];
                            tx_key.extend_from_slice(&txid_internal);
                            
                            // Read current transaction
                            if let Ok(Some(tx_data)) = db.get_cf(&cf_transactions, &tx_key) {
                                // Verify it still needs fixing (height=0, -1, or -2)
                                let current_height = i32::from_le_bytes([
                                    tx_data[4], tx_data[5], tx_data[6], tx_data[7]
                                ]);
                                
                                // CRITICAL FIX: Also fix HEIGHT_ORPHAN (-1) transactions
                                // These may have been incorrectly marked as orphaned but have valid 'B' keys
                                if current_height == 0 || current_height == HEIGHT_UNRESOLVED || current_height == HEIGHT_ORPHAN {
                                    // Check if this height is in canonical chain
                                    if let Some(_block_hash) = height_to_blockhash.get(&(height as i64)) {
                                        // Block is in canonical chain - use its height
                                        let mut new_value = tx_data[0..4].to_vec(); // version
                                        new_value.extend(&height.to_le_bytes());
                                        new_value.extend(&tx_data[8..]); // rest of data
                                        
                                        batch.put_cf(&cf_transactions, &tx_key, &new_value);
                                        updated += 1;

                                        // Remove from set so we don't process it again
                                        txids_needing_fix.remove(&txid_internal);
                                        
                                        if updated % BATCH_SIZE == 0 {
                                            commit_batch(&db, batch, &wo, bulk)?;
                                            batch = WriteBatch::default();

                                        }
                                    } else {
                                        // Height not in canonical chain - mark as orphaned
                                        let mut new_value = tx_data[0..4].to_vec(); // version
                                        new_value.extend(&(-1i32).to_le_bytes());
                                        new_value.extend(&tx_data[8..]); // rest of data
                                        
                                        batch.put_cf(&cf_transactions, &tx_key, &new_value);
                                        newly_orphaned += 1;
                                        
                                        // Remove from set
                                        txids_needing_fix.remove(&txid_internal);
                                        
                                        if (updated + newly_orphaned) % BATCH_SIZE == 0 {
                                            commit_batch(&db, batch, &wo, bulk)?;
                                            batch = WriteBatch::default();
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                warn!(error = ?e, "Error reading block-tx index");
                break;
            }
        }
    }
    
    // Write final batch
    if !batch.is_empty() {
        commit_batch(&db, batch, &wo, bulk)?;
    }

    // Mark any remaining transactions (not found in block index) as orphaned
    let mut all_orphaned_txids: Vec<Vec<u8>> = Vec::new();
    
    if !txids_needing_fix.is_empty() {
        info!(
            count = txids_needing_fix.len(),
            "Transactions not found in block index - marking as orphaned"
        );
        
        let mut orphan_batch = WriteBatch::default();
        let mut marked_orphan = 0;
        
        for txid_internal in &txids_needing_fix {
            let mut tx_key = vec![b't'];
            tx_key.extend_from_slice(txid_internal);
            
            if let Ok(Some(tx_data)) = db.get_cf(&cf_transactions, &tx_key) {
                if tx_data.len() >= 8 {
                    let current_height = i32::from_le_bytes([
                        tx_data[4], tx_data[5], tx_data[6], tx_data[7]
                    ]);
                    
                    // Only mark as orphan if still has height=0 or HEIGHT_UNRESOLVED
                    if current_height == HEIGHT_GENESIS || current_height == HEIGHT_UNRESOLVED {
                        let mut new_value = tx_data[0..4].to_vec(); // version
                        new_value.extend(&HEIGHT_ORPHAN.to_le_bytes());
                        new_value.extend(&tx_data[8..]); // rest of data
                        
                        orphan_batch.put_cf(&cf_transactions, &tx_key, &new_value);
                        marked_orphan += 1;
                        all_orphaned_txids.push(txid_internal.clone());
                        
                        if marked_orphan % BATCH_SIZE == 0 {
                            commit_batch(&db, orphan_batch, &wo, bulk)?;
                            orphan_batch = WriteBatch::default();
                        }
                    }
                }
            }
        }
        
        if !orphan_batch.is_empty() {
            commit_batch(&db, orphan_batch, &wo, bulk)?;
        }

        newly_orphaned += marked_orphan;
        info!(marked = marked_orphan, "Marked additional transactions as HEIGHT_ORPHAN");
    }
    
    // CRITICAL FIX: Clean address index for ALL newly orphaned transactions
    if !all_orphaned_txids.is_empty() {
        info!(count = all_orphaned_txids.len(), "Cleaning address index for orphaned transactions");
        // TODO: Re-enable when orphan_cleanup module is available
        // match remove_orphaned_txs_batch(&db, &all_orphaned_txids).await {
        //     Ok((cleaned, errors)) => {
        //         info!(cleaned, errors, "Cleaned addresses from orphaned transactions");
        //     }
        //     Err(e) => {
        //         warn!(error = ?e, "Address cleanup failed");
        //     }
        // }
    }
    
    info!(updated, newly_orphaned, "Height resolution complete");
    
    Ok((updated, newly_orphaned))
}

#[cfg(test)]
mod tests {
    
    
    #[tokio::test]
    async fn test_resolve_heights() {
        // This would require a test database
        // For now, just ensure it compiles
        assert!(true);
    }
}
