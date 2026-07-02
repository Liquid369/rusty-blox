use crate::constants::HEIGHT_GENESIS;
/// Transaction repair utilities
///
/// Fixes database inconsistencies like transactions stored with height=0
/// NOTE: After Fix #1, new transactions use HEIGHT_UNRESOLVED (-2) instead.
///       This repair handles legacy databases that had the height=0 bug.
use rocksdb::{WriteBatch, DB};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};

/// Fix all transactions that have height=0 by looking them up in the block index
///
/// This repairs a LEGACY bug where transactions were stored with height=0 during initial sync
/// when block heights hadn't been resolved yet. After Fix #1, new unresolved transactions
/// use HEIGHT_UNRESOLVED (-2) instead.
///
/// The fix:
/// 1. Scans all transactions to find ones with height=0 (excluding actual genesis)
/// 2. Uses the 'B' (block transaction) index to find the correct height
/// 3. Updates the transaction data with the correct height or marks as HEIGHT_ORPHAN
///
/// Returns (fixed_count, unfixable_count)
pub async fn fix_zero_height_transactions(
    db: &Arc<DB>,
) -> Result<(usize, usize), Box<dyn std::error::Error>> {
    info!("Checking for transactions with height=0");

    let cf_transactions = db
        .cf_handle("transactions")
        .ok_or("transactions CF not found")?;

    // Step 1: Find all transactions with height=0
    let mut zero_height_txs: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let mut total_txs = 0;

    let iter = db.iterator_cf(&cf_transactions, rocksdb::IteratorMode::Start);

    for item in iter {
        match item {
            Ok((key, value)) => {
                // Only process 't' prefix entries (transaction data)
                if !key.is_empty() && key[0] == b't' {
                    total_txs += 1;

                    // Check height (bytes 4-8 in value)
                    // Format: version(4) + height(4) + raw_tx
                    if value.len() >= 8 {
                        let height = i32::from_le_bytes([value[4], value[5], value[6], value[7]]);

                        // Find transactions with HEIGHT_GENESIS that aren't actually genesis
                        // (legacy bug - after Fix #1, unresolved use HEIGHT_UNRESOLVED instead)
                        if height == HEIGHT_GENESIS {
                            zero_height_txs.push((key.to_vec(), value.to_vec()));
                        }
                    }

                    if total_txs % 500_000 == 0 {
                        info!(
                            scanned = total_txs,
                            found_zero = zero_height_txs.len(),
                            "Scanning transactions"
                        );
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "Error reading transaction");
            }
        }
    }

    if zero_height_txs.is_empty() {
        info!("No transactions with height=0 found");
        return Ok((0, 0));
    }

    info!(
        found_zero = zero_height_txs.len(),
        total = total_txs,
        "Found transactions with height=0"
    );

    // Step 2: Build txid -> height mapping using the 'B' index
    info!("Looking up correct heights from block index");

    let mut txid_to_height: HashMap<Vec<u8>, i32> = HashMap::new();

    // Iterate through all 'B' prefix entries (block transaction index)
    // Format: 'B' + height(4) + tx_index(8) -> txid_hex
    let block_tx_iter = db.iterator_cf(&cf_transactions, rocksdb::IteratorMode::Start);

    let mut block_entries = 0;
    for item in block_tx_iter {
        match item {
            Ok((key, value)) => {
                // Only process 'B' prefix entries
                if !key.is_empty() && key[0] == b'B' {
                    block_entries += 1;

                    // Extract height from key (bytes 1-5)
                    if key.len() >= 5 {
                        let height = i32::from_le_bytes([key[1], key[2], key[3], key[4]]);

                        // Value is txid as hex string (display format)
                        if let Ok(txid_hex) = String::from_utf8(value.to_vec()) {
                            if let Ok(txid_bytes) = hex::decode(&txid_hex) {
                                // Reverse to get internal format (txids are stored reversed)
                                let txid_internal: Vec<u8> =
                                    txid_bytes.iter().rev().cloned().collect();
                                txid_to_height.insert(txid_internal, height);
                            }
                        }
                    }

                    if block_entries % 100_000 == 0 {
                        info!(processed = block_entries, "Processing block index entries");
                    }
                }
            }
            Err(_) => break,
        }
    }

    info!(
        found_heights = txid_to_height.len(),
        "Found heights for transactions in block index"
    );

    // Step 3: Update fixable transactions and MARK orphaned ones (don't delete)
    let mut batch = WriteBatch::default();
    let mut fixed_count = 0;
    let mut orphaned_count = 0;
    let mut orphaned_txids: Vec<Vec<u8>> = Vec::new(); // Track for cleanup
    const BATCH_SIZE: usize = 10_000;

    info!("Updating fixable transactions and marking orphaned ones");

    for (tx_key, tx_value) in &zero_height_txs {
        // Extract txid from key (skip 't' prefix)
        let txid = &tx_key[1..];

        if let Some(&correct_height) = txid_to_height.get(txid) {
            // Rebuild transaction data with correct height
            // Format: version (4 bytes) + height (4 bytes) + raw_tx
            let version_bytes = &tx_value[0..4];
            let raw_tx = &tx_value[8..]; // Skip old version+height

            let mut new_value = version_bytes.to_vec();
            new_value.extend(&correct_height.to_le_bytes());
            new_value.extend(raw_tx);

            batch.put_cf(&cf_transactions, tx_key, &new_value);
            fixed_count += 1;

            if fixed_count % BATCH_SIZE == 0 {
                db.write(batch)?;
                batch = WriteBatch::default();
                info!(fixed = fixed_count, "Fixed transactions");
            }
        } else {
            // Transaction not in block index = orphaned transaction (from non-canonical block)
            // KEEP it but mark with height = -1 to indicate it's orphaned
            // This preserves the data for historical/debugging purposes while excluding
            // it from active UTXO set and address balances

            let version_bytes = &tx_value[0..4];
            let raw_tx = &tx_value[8..]; // Skip old version+height

            let mut new_value = version_bytes.to_vec();
            new_value.extend(&(-1i32).to_le_bytes()); // -1 = orphaned
            new_value.extend(raw_tx);

            batch.put_cf(&cf_transactions, tx_key, &new_value);
            orphaned_count += 1;
            orphaned_txids.push(txid.to_vec()); // Track for cleanup

            // Log details about first few orphaned transactions
            if orphaned_count <= 10 {
                let txid_hex: Vec<u8> = txid.iter().rev().cloned().collect();
                warn!(txid = %hex::encode(&txid_hex), "Marking as orphaned (not in canonical chain)");
            }
        }

        // Commit batch periodically
        if (fixed_count + orphaned_count) % BATCH_SIZE == 0 {
            db.write(batch)?;
            batch = WriteBatch::default();

            if (fixed_count + orphaned_count) % 10_000 == 0 {
                info!(
                    processed = fixed_count + orphaned_count,
                    fixed = fixed_count,
                    orphaned = orphaned_count,
                    "Processing transactions"
                );
            }
        }
    }

    // Write final batch
    if !batch.is_empty() {
        db.write(batch)?;
    }

    info!(
        fixed = fixed_count,
        "Fixed transactions with correct heights"
    );
    if orphaned_count > 0 {
        warn!(
            orphaned = orphaned_count,
            "Marked transactions as orphaned (height=-1, not in canonical chain)"
        );
        info!("Orphaned transactions kept for historical queries but excluded from balances/UTXOs");

        // CRITICAL FIX: Clean address index for orphaned transactions
        info!(
            count = orphaned_txids.len(),
            "Cleaning address index for orphaned transactions"
        );
        // TODO: Re-enable when orphan_cleanup module is available
        // match remove_orphaned_txs_batch(&db, &orphaned_txids).await {
        //     Ok((cleaned, errors)) => {
        //         info!(cleaned = cleaned, errors = errors, "Cleaned addresses");
        //     }
        //     Err(e) => {
        //         warn!(error = %e, "Address cleanup failed");
        //     }
        // }
    }

    Ok((fixed_count, orphaned_count))
}

/// Promote a canonical block's transactions from a negative stored height to `height`.
///
/// The authoritative txid list is supplied by the caller (RPC `getblock`, canonical), so
/// leaked/stale `'B'` entries can never cause a wrong re-confirmation — an orphaned tx
/// simply isn't in the list and is left untouched. This is the reorg-proof heal for the
/// historic stuck `-1` windows.
///
/// Historic `'t'` records are INTERNAL-keyed (reversed display, transactions.rs:569); some
/// live-tip records are DISPLAY-keyed (monitor.rs:638). Match the readers: internal key
/// first, display fallback only if absent (block_detail.rs:360, api/transactions.rs:66).
/// Only a NEGATIVE stored height is rewritten (no-op if already resolved). Value layout is
/// version(4) ++ height(i32 LE, 4) ++ tx_bytes — only bytes 4..8 change.
pub fn promote_block_txs_to_height(
    db: &Arc<DB>,
    height: i32,
    canonical_display_txids: &[String],
) -> Result<usize, Box<dyn std::error::Error>> {
    let cf = db
        .cf_handle("transactions")
        .ok_or("transactions CF not found")?;
    let mut batch = WriteBatch::default();
    let mut promoted = 0usize;

    for txid_hex in canonical_display_txids {
        let display = match hex::decode(txid_hex) {
            Ok(b) if b.len() == 32 => b,
            _ => continue,
        };
        let internal: Vec<u8> = display.iter().rev().cloned().collect();

        // Internal key is primary (historic parse path); display is the live-tip fallback.
        let mut key = vec![b't'];
        key.extend_from_slice(&internal);
        let mut value = db.get_cf(&cf, &key)?;
        if value.is_none() {
            key.truncate(1);
            key.extend_from_slice(&display);
            value = db.get_cf(&cf, &key)?;
        }
        let Some(value) = value else { continue };
        if value.len() < 8 {
            continue;
        }
        let stored = i32::from_le_bytes([value[4], value[5], value[6], value[7]]);
        if stored >= 0 {
            continue; // already canonical — leave byte-for-byte
        }

        let mut new_value = value[0..4].to_vec();
        new_value.extend(&height.to_le_bytes());
        new_value.extend(&value[8..]);
        batch.put_cf(&cf, &key, &new_value);
        promoted += 1;
    }

    if !batch.is_empty() {
        db.write(batch)?;
    }
    Ok(promoted)
}

/// Find the distinct block heights that contain at least one heightless (`< 0`) transaction,
/// resolved via the `'B'` index. These are the candidate blocks to re-resolve from the node.
///
/// A stale/leaked `'B'` entry can only OVER-nominate a height (a harmless no-op re-resolution
/// — promotion trusts the node's canonical list, never `'B'`, so it can't wrongly confirm).
/// Targets the historic INTERNAL-keyed stuck records (matches the `'B'` display value reversed
/// to internal); transient live-tip mempool `-1` records are the monitor's job, not this pass.
pub fn nominate_heightless_heights(
    db: &Arc<DB>,
) -> Result<std::collections::BTreeSet<i32>, Box<dyn std::error::Error>> {
    use std::collections::{BTreeSet, HashSet};
    let cf = db
        .cf_handle("transactions")
        .ok_or("transactions CF not found")?;

    // Pass 1: internal txids of every heightless 't' record.
    let mut heightless: HashSet<Vec<u8>> = HashSet::new();
    for item in db.iterator_cf(&cf, rocksdb::IteratorMode::Start) {
        let (key, value) = item?;
        if key.first() == Some(&b't') && value.len() >= 8 {
            let h = i32::from_le_bytes([value[4], value[5], value[6], value[7]]);
            if h < 0 {
                heightless.insert(key[1..].to_vec());
            }
        }
    }
    if heightless.is_empty() {
        return Ok(BTreeSet::new());
    }

    // Pass 2: 'B' + height + index -> display txid hex; reverse to internal and match.
    let mut heights = BTreeSet::new();
    for item in db.iterator_cf(&cf, rocksdb::IteratorMode::Start) {
        let (key, value) = item?;
        if key.first() == Some(&b'B') && key.len() == 13 {
            let h = i32::from_le_bytes([key[1], key[2], key[3], key[4]]);
            if let Ok(display) = hex::decode(String::from_utf8_lossy(&value).as_ref()) {
                // A stuck 't' key may be stored INTERNAL (historic parse) or DISPLAY
                // (live-tip index_block_from_rpc) — match either form so both are
                // nominated. (promote then re-checks against the node's canonical list,
                // so a spurious match is a harmless no-op, never a wrong promotion.)
                let internal: Vec<u8> = display.iter().rev().cloned().collect();
                if heightless.contains(&internal) || heightless.contains(&display) {
                    heights.insert(h);
                }
            }
        }
    }
    Ok(heights)
}

/// One-shot heal for historic stuck heightless transactions. Nominates affected heights,
/// then re-resolves each block's canonical txid list from the node (authoritative) and
/// promotes the stuck `'t'` records to their real height. Reorg-proof: an orphaned tx not in
/// the node's canonical block is never in the list, so never promoted. Idempotent.
///
/// Requires RPC. Run as a startup one-shot (before the live monitor) so it never races the
/// tip; it only touches historic (already-buried) heights, which the monitor never rewrites.
/// Returns (blocks_processed, txs_promoted). RPC failures skip the block (retry next run),
/// never abort — a partial heal is safe and idempotent.
pub async fn reresolve_heightless_blocks(
    db: &Arc<DB>,
) -> Result<(usize, usize), Box<dyn std::error::Error>> {
    let heights = nominate_heightless_heights(db)?;
    if heights.is_empty() {
        info!("No heightless transactions to re-resolve");
        return Ok((0, 0));
    }
    info!(
        blocks = heights.len(),
        "Re-resolving heightless blocks from the node (authoritative canonical tx lists)"
    );

    let mut processed = 0usize;
    let mut total_promoted = 0usize;
    for height in heights {
        let hash = match crate::api::helpers::rpc_call_json(
            "getblockhash",
            serde_json::json!([height as i64]),
        )
        .await
        {
            Ok(v) => match v.as_str() {
                Some(s) => s.to_string(),
                None => {
                    warn!(height, "getblockhash returned non-string; skipping");
                    continue;
                }
            },
            Err(e) => {
                warn!(height, error = %e, "getblockhash failed; skipping");
                continue;
            }
        };
        let block = match crate::api::helpers::rpc_call_json(
            "getblock",
            serde_json::json!([hash, 1]),
        )
        .await
        {
            Ok(v) => v,
            Err(e) => {
                warn!(height, error = %e, "getblock failed; skipping");
                continue;
            }
        };
        let txids: Vec<String> = block
            .get("tx")
            .and_then(|t| t.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| t.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        if txids.is_empty() {
            warn!(height, "getblock returned no txids; skipping");
            continue;
        }
        match promote_block_txs_to_height(db, height, &txids) {
            Ok(n) => {
                processed += 1;
                total_promoted += n;
                if n > 0 {
                    info!(height, promoted = n, "Re-resolved heightless block");
                }
            }
            Err(e) => warn!(height, error = %e, "Promotion failed; skipping"),
        }
    }
    info!(
        processed,
        total_promoted, "Heightless re-resolution complete"
    );
    Ok((processed, total_promoted))
}

/// Delete malformed "phantom stub" transaction records: `'t' + txid(32)` entries whose
/// value is too short (< 8 bytes) to hold `version(4) + height(4)`. They can't be a real
/// transaction and only serve to SHADOW the valid record at the other key order (the `/tx`
/// 404 bug). Deleting them is safe — a `< 8`-byte value carries no recoverable tx data.
/// Returns the count deleted. Idempotent.
pub fn delete_stub_tx_records(db: &Arc<DB>) -> Result<usize, Box<dyn std::error::Error>> {
    let cf = db
        .cf_handle("transactions")
        .ok_or("transactions CF not found")?;
    let mut batch = WriteBatch::default();
    let mut deleted = 0usize;
    for item in db.iterator_cf(&cf, rocksdb::IteratorMode::Start) {
        let (key, value) = item?;
        // 't' + txid(32) == 33-byte key; a real record is version(4)+height(4)+raw_tx (>= 8).
        if key.first() == Some(&b't') && key.len() == 33 && value.len() < 8 {
            batch.delete_cf(&cf, &key);
            deleted += 1;
        }
    }
    if !batch.is_empty() {
        db.write(batch)?;
    }
    if deleted > 0 {
        info!(
            deleted,
            "Deleted malformed phantom-stub transaction records"
        );
    }
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rocksdb::Options;

    const TXA: &str = "6d04c36f3f6f4adaf55ce658169d67327c4b56fe172cc5b7d2ad6e8a24d4cded";
    const TXB: &str = "1111111111111111111111111111111111111111111111111111111111111111";

    fn seed_db() -> (tempfile::TempDir, Arc<DB>) {
        let temp = tempfile::TempDir::new().unwrap();
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db = Arc::new(DB::open_cf(&opts, temp.path(), ["transactions"]).unwrap());
        (temp, db)
    }

    fn tkey(display_hex: &str, internal: bool) -> Vec<u8> {
        let display = hex::decode(display_hex).unwrap();
        let bytes: Vec<u8> = if internal {
            display.iter().rev().cloned().collect()
        } else {
            display
        };
        let mut k = vec![b't'];
        k.extend_from_slice(&bytes);
        k
    }
    fn rec(height: i32, raw: &[u8]) -> Vec<u8> {
        let mut v = vec![1u8, 0, 0, 0];
        v.extend(&height.to_le_bytes());
        v.extend(raw);
        v
    }
    fn height_at(db: &Arc<DB>, key: &[u8]) -> i32 {
        let cf = db.cf_handle("transactions").unwrap();
        let v = db.get_cf(&cf, key).unwrap().unwrap();
        i32::from_le_bytes([v[4], v[5], v[6], v[7]])
    }

    // Core heal: a stuck -1 internal-keyed tx that IS in the canonical list -> H.
    #[test]
    fn promotes_stuck_internal_keyed_tx() {
        let (_t, db) = seed_db();
        let cf = db.cf_handle("transactions").unwrap();
        db.put_cf(&cf, tkey(TXA, true), rec(-1, &[0xde, 0xad]))
            .unwrap();
        let n = promote_block_txs_to_height(&db, 5_465_071, &[TXA.to_string()]).unwrap();
        assert_eq!(n, 1);
        assert_eq!(height_at(&db, &tkey(TXA, true)), 5_465_071);
    }

    // Safety: a -1 tx NOT in the canonical list (orphan / leaked 'B') is never touched.
    #[test]
    fn leaves_tx_not_in_canonical_list() {
        let (_t, db) = seed_db();
        let cf = db.cf_handle("transactions").unwrap();
        db.put_cf(&cf, tkey(TXB, true), rec(-1, &[0x01])).unwrap();
        let n = promote_block_txs_to_height(&db, 5_000, &[TXA.to_string()]).unwrap();
        assert_eq!(n, 0);
        assert_eq!(height_at(&db, &tkey(TXB, true)), -1);
    }

    // No-op when already canonical (byte-stable).
    #[test]
    fn noop_when_already_correct() {
        let (_t, db) = seed_db();
        let cf = db.cf_handle("transactions").unwrap();
        db.put_cf(&cf, tkey(TXA, true), rec(5_465_071, &[0x02]))
            .unwrap();
        let n = promote_block_txs_to_height(&db, 5_465_071, &[TXA.to_string()]).unwrap();
        assert_eq!(n, 0);
    }

    // Display-keyed (live-tip) fallback when no internal record exists.
    #[test]
    fn display_fallback_when_no_internal() {
        let (_t, db) = seed_db();
        let cf = db.cf_handle("transactions").unwrap();
        db.put_cf(&cf, tkey(TXA, false), rec(-1, &[0x03])).unwrap();
        let n = promote_block_txs_to_height(&db, 42, &[TXA.to_string()]).unwrap();
        assert_eq!(n, 1);
        assert_eq!(height_at(&db, &tkey(TXA, false)), 42);
    }

    // Only the 4 height bytes change; version and tx_bytes preserved exactly.
    #[test]
    fn preserves_version_and_tx_bytes() {
        let (_t, db) = seed_db();
        let cf = db.cf_handle("transactions").unwrap();
        let mut v = vec![2u8, 0, 0, 0];
        v.extend(&(-1i32).to_le_bytes());
        let raw = [0xAA, 0xBB, 0xCC];
        v.extend(&raw);
        db.put_cf(&cf, tkey(TXA, true), &v).unwrap();
        promote_block_txs_to_height(&db, 777, &[TXA.to_string()]).unwrap();
        let out = db.get_cf(&cf, tkey(TXA, true)).unwrap().unwrap();
        assert_eq!(&out[0..4], &[2, 0, 0, 0]);
        assert_eq!(i32::from_le_bytes([out[4], out[5], out[6], out[7]]), 777);
        assert_eq!(&out[8..], &raw);
    }

    // Nominate only heights that actually contain a heightless tx; canonical excluded.
    #[test]
    fn nominates_only_heights_with_heightless_tx() {
        let (_t, db) = seed_db();
        let cf = db.cf_handle("transactions").unwrap();
        db.put_cf(&cf, tkey(TXA, true), rec(-1, &[0x01])).unwrap(); // stuck
        db.put_cf(&cf, tkey(TXB, true), rec(5_000, &[0x02]))
            .unwrap(); // canonical
        let bput = |h: i32, display_hex: &str| {
            let mut k = vec![b'B'];
            k.extend(&h.to_le_bytes());
            k.extend(&0u64.to_le_bytes());
            db.put_cf(&cf, &k, display_hex.as_bytes()).unwrap();
        };
        bput(5_465_071, TXA);
        bput(5_000, TXB);

        let heights = nominate_heightless_heights(&db).unwrap();
        assert_eq!(heights.into_iter().collect::<Vec<_>>(), vec![5_465_071]);
    }

    // A stuck record may be DISPLAY-keyed (live-tip index_block_from_rpc), not just
    // internal (historic parse). Nominate must catch BOTH orientations, else it silently
    // misses live-tip-originated stuck txs (found on the local DB).
    #[test]
    fn nominates_display_keyed_heightless_tx() {
        let (_t, db) = seed_db();
        let cf = db.cf_handle("transactions").unwrap();
        db.put_cf(&cf, tkey(TXA, false), rec(-1, &[0x01])).unwrap(); // DISPLAY-keyed stuck
        let mut k = vec![b'B'];
        k.extend(&5_465_071i32.to_le_bytes());
        k.extend(&0u64.to_le_bytes());
        db.put_cf(&cf, &k, TXA.as_bytes()).unwrap(); // 'B' value = display hex

        let heights = nominate_heightless_heights(&db).unwrap();
        assert_eq!(heights.into_iter().collect::<Vec<_>>(), vec![5_465_071]);
    }

    // Delete only the short 't'+txid stubs; valid records and non-'t' keys survive.
    #[test]
    fn deletes_short_stub_records_only() {
        let (_t, db) = seed_db();
        let cf = db.cf_handle("transactions").unwrap();
        db.put_cf(&cf, tkey(TXA, true), rec(5000, &[0xAA])).unwrap(); // valid (>=8)
        db.put_cf(&cf, tkey(TXB, true), [0u8, 1, 2]).unwrap(); // 3-byte stub
        let mut bkey = vec![b'B']; // a 'B' index entry with a short value
        bkey.extend(&100i32.to_le_bytes());
        bkey.extend(&0u64.to_le_bytes());
        db.put_cf(&cf, &bkey, b"x").unwrap();

        let n = delete_stub_tx_records(&db).unwrap();

        assert_eq!(n, 1);
        assert!(
            db.get_cf(&cf, tkey(TXA, true)).unwrap().is_some(),
            "valid record deleted"
        );
        assert!(
            db.get_cf(&cf, tkey(TXB, true)).unwrap().is_none(),
            "stub not deleted"
        );
        assert!(
            db.get_cf(&cf, &bkey).unwrap().is_some(),
            "'B' entry wrongly deleted"
        );
    }
}
