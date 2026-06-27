use crate::atomic_writer::AtomicBatchWriter;
use crate::parser::{deserialize_transaction, deserialize_utxos, serialize_utxos};
use rocksdb::DB;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
/// Address Index Rollback Module
///
/// Handles reversal of address index entries during blockchain reorganizations.
/// Maintains address history accuracy by properly removing and re-adding transactions
/// from the addr_index CF during reorgs.
///
/// PIVX Core Equivalent: DisconnectBlock() address index updates
///
/// # Address Index Data Structure
///
/// The addr_index CF stores 4 types of keys per address:
///
/// 1. **UTXO List** - `'a' + address` -> serialized [(txid, vout), ...]
///    - Unspent outputs only
///    - Used by /api/address/{address}/utxos
///
/// 2. **Transaction List** - `'t' + address` -> concatenated txids (32 bytes each)
///    - All transactions involving this address (sent OR received)
///    - Used by /api/address/{address}/txs
///
/// 3. **Total Received** - `'r' + address` -> i64 LE bytes
///    - Cumulative amount received by this address
///    - Never decreases (even when outputs are spent)
///
/// 4. **Total Sent** - `'s' + address` -> i64 LE bytes
///    - Cumulative amount sent from this address
///    - Only increases when address spends UTXOs
///
/// # Rollback Strategy
///
/// During a reorg from height H to height F (fork point):
///
/// **Phase 1: Remove Invalidated Blocks (H down to F+1)**
/// 1. Track all addresses affected by rolled-back transactions
/// 2. Store block-level transaction data for each address
/// 3. Decrement totals (received/sent) for each address
/// 4. Remove UTXOs created in rolled-back blocks
/// 5. Mark previously-spent UTXOs as unspent again
///
/// **Phase 2: Apply New Chain (F+1 up to new height)**
/// 1. Re-index transactions from new canonical chain
/// 2. Rebuild UTXO lists, transaction lists, and totals
///
/// # Implementation Notes
///
/// This module provides:
/// - **Block-level tracking**: Track address changes per block for efficient rollback
/// - **Atomic updates**: All address changes commit or rollback together
/// - **UTXO resurrection**: Restore UTXOs that were spent in rolled-back blocks
/// - **Balance recalculation**: Accurately adjust received/sent totals
///
/// Unlike PIVX Core's in-memory view, we store explicit undo data because:
/// - We index addresses (Core doesn't)
/// - We track spent UTXOs for API queries
/// - We need efficient rollback without full re-indexing
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Tracks address index changes for a single block
/// This enables efficient rollback without re-scanning the entire chain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddressBlockUndo {
    /// Block height this undo data applies to
    pub height: i32,

    /// Map of address -> transactions added in this block
    /// Key: address string, Value: list of txid (internal format, 32 bytes)
    pub address_txs: HashMap<String, Vec<Vec<u8>>>,

    /// Map of address -> UTXOs created in this block
    /// Key: address string, Value: list of (txid, vout) tuples
    pub address_utxos_created: HashMap<String, Vec<(Vec<u8>, u64)>>,

    /// Map of address -> UTXOs spent in this block
    /// Key: address string, Value: list of (txid, vout) tuples
    pub address_utxos_spent: HashMap<String, Vec<(Vec<u8>, u64)>>,

    /// Map of address -> amount received in this block
    /// Key: address string, Value: satoshis received
    pub address_received: HashMap<String, i64>,

    /// Map of address -> amount sent in this block
    /// Key: address string, Value: satoshis sent
    pub address_sent: HashMap<String, i64>,
}

impl AddressBlockUndo {
    /// Create new empty undo data for a block
    pub fn new(height: i32) -> Self {
        Self {
            height,
            address_txs: HashMap::new(),
            address_utxos_created: HashMap::new(),
            address_utxos_spent: HashMap::new(),
            address_received: HashMap::new(),
            address_sent: HashMap::new(),
        }
    }

    /// Add a transaction to an address's history
    pub fn add_tx(&mut self, address: String, txid: Vec<u8>) {
        self.address_txs.entry(address).or_default().push(txid);
    }

    /// Add a created UTXO
    pub fn add_utxo_created(&mut self, address: String, txid: Vec<u8>, vout: u64) {
        self.address_utxos_created
            .entry(address)
            .or_default()
            .push((txid, vout));
    }

    /// Add a spent UTXO
    pub fn add_utxo_spent(&mut self, address: String, txid: Vec<u8>, vout: u64) {
        self.address_utxos_spent
            .entry(address)
            .or_default()
            .push((txid, vout));
    }

    /// Add received amount
    pub fn add_received(&mut self, address: String, amount: i64) {
        *self.address_received.entry(address).or_insert(0) += amount;
    }

    /// Add sent amount
    pub fn add_sent(&mut self, address: String, amount: i64) {
        *self.address_sent.entry(address).or_insert(0) += amount;
    }

    /// Fold another block's undo into this one (used to reverse a multi-block reorg
    /// as a single read-modify-write per address). Sums r/s and concatenates the
    /// tx/UTXO lists so reverse_address_block reads each on-disk base value exactly
    /// once and applies the combined delta — avoiding the stale-read last-write-wins
    /// bug that an un-flushed per-block loop would hit on a repeated address.
    pub fn merge_from(&mut self, other: AddressBlockUndo) {
        for (addr, txids) in other.address_txs {
            self.address_txs.entry(addr).or_default().extend(txids);
        }
        for (addr, utxos) in other.address_utxos_created {
            self.address_utxos_created
                .entry(addr)
                .or_default()
                .extend(utxos);
        }
        for (addr, utxos) in other.address_utxos_spent {
            self.address_utxos_spent
                .entry(addr)
                .or_default()
                .extend(utxos);
        }
        for (addr, amount) in other.address_received {
            *self.address_received.entry(addr).or_insert(0) += amount;
        }
        for (addr, amount) in other.address_sent {
            *self.address_sent.entry(addr).or_insert(0) += amount;
        }
    }
}

/// Store address undo data for a block
///
/// Key format: 'addr_undo' + height (4 bytes, LE)
/// Value: bincode-serialized AddressBlockUndo
pub async fn store_address_undo(
    db: Arc<DB>,
    undo_data: &AddressBlockUndo,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let height = undo_data.height;
    let serialized = bincode::serialize(undo_data)?;

    let db_clone = db.clone();
    tokio::task::spawn_blocking(move || {
        let cf_chain_state = db_clone
            .cf_handle("chain_state")
            .ok_or("chain_state CF not found")?;

        let mut key = b"addr_undo".to_vec();
        key.extend_from_slice(&height.to_le_bytes());

        db_clone
            .put_cf(&cf_chain_state, &key, &serialized)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    })
    .await?
}

/// Load address undo data for a block
pub async fn load_address_undo(
    db: Arc<DB>,
    height: i32,
) -> Result<Option<AddressBlockUndo>, Box<dyn std::error::Error + Send + Sync>> {
    let db_clone = db.clone();

    let data = tokio::task::spawn_blocking(move || {
        let cf_chain_state = db_clone
            .cf_handle("chain_state")
            .ok_or("chain_state CF not found")?;

        let mut key = b"addr_undo".to_vec();
        key.extend_from_slice(&height.to_le_bytes());

        db_clone
            .get_cf(&cf_chain_state, &key)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    })
    .await??;

    if let Some(bytes) = data {
        let undo_data: AddressBlockUndo = bincode::deserialize(&bytes)?;
        Ok(Some(undo_data))
    } else {
        Ok(None)
    }
}

/// Delete address undo data for a block (called after successful reorg)
pub async fn delete_address_undo(
    db: Arc<DB>,
    height: i32,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let db_clone = db.clone();

    tokio::task::spawn_blocking(move || {
        let cf_chain_state = db_clone
            .cf_handle("chain_state")
            .ok_or("chain_state CF not found")?;

        let mut key = b"addr_undo".to_vec();
        key.extend_from_slice(&height.to_le_bytes());

        db_clone
            .delete_cf(&cf_chain_state, &key)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    })
    .await?
}

/// Build address undo data by analyzing a block's transactions
///
/// This should be called during normal block indexing to prepare for potential reorgs
pub async fn build_address_undo_from_block(
    db: Arc<DB>,
    height: i32,
    txids: Vec<Vec<u8>>,
) -> Result<AddressBlockUndo, Box<dyn std::error::Error + Send + Sync>> {
    let mut undo = AddressBlockUndo::new(height);

    let db_clone = db.clone();
    let cf_transactions = db_clone
        .cf_handle("transactions")
        .ok_or("transactions CF not found")?;
    let _cf_addr_index = db_clone
        .cf_handle("addr_index")
        .ok_or("addr_index CF not found")?;

    // Process each transaction in the block
    for txid_internal in txids {
        // Load transaction
        let mut tx_key = vec![b't'];
        tx_key.extend_from_slice(&txid_internal);

        let tx_data = db.get_cf(&cf_transactions, &tx_key)?;
        if let Some(tx_bytes) = tx_data {
            if tx_bytes.len() < 8 {
                continue;
            }

            // Parse transaction (skip version + height)
            let mut tx_with_header = vec![0u8; 4];
            tx_with_header.extend_from_slice(&tx_bytes[8..]);

            if let Ok(tx) = deserialize_transaction(&tx_with_header).await {
                // Track addresses that received funds (outputs)
                for output in &tx.outputs {
                    for address in &output.address {
                        if address.is_empty()
                            || address == "Nonstandard"
                            || address == "CoinBaseTx"
                            || address == "CoinStakeTx"
                        {
                            continue;
                        }

                        // Add transaction to address history
                        undo.add_tx(address.clone(), txid_internal.clone());

                        // Track UTXO creation
                        undo.add_utxo_created(address.clone(), txid_internal.clone(), output.index);

                        // Track received amount
                        undo.add_received(address.clone(), output.value);
                    }
                }

                // Track addresses that spent funds (inputs)
                for input in &tx.inputs {
                    // Skip coinbase
                    if input.coinbase.is_some() {
                        continue;
                    }

                    if let Some(prevout) = &input.prevout {
                        // Load previous transaction to get addresses
                        if let Ok(prev_txid_bytes) = hex::decode(&prevout.hash) {
                            // prevout.hash is DISPLAY-order hex; the tx CF and the
                            // addr_index 'a' set are keyed by display-order bytes
                            // (matching monitor.rs's writes), so use the decoded
                            // bytes directly. The prior .rev() produced internal
                            // order, which missed the prev-tx lookup and silently
                            // under-reversed 'sent' (the reorg r/s-reversal bug).
                            let prev_txid_display = prev_txid_bytes;

                            let mut prev_tx_key = vec![b't'];
                            prev_tx_key.extend_from_slice(&prev_txid_display);

                            if let Some(prev_tx_data) = db.get_cf(&cf_transactions, &prev_tx_key)? {
                                if prev_tx_data.len() >= 8 {
                                    let mut prev_with_header = vec![0u8; 4];
                                    prev_with_header.extend_from_slice(&prev_tx_data[8..]);

                                    if let Ok(prev_tx) =
                                        deserialize_transaction(&prev_with_header).await
                                    {
                                        if let Some(prev_output) =
                                            prev_tx.outputs.get(prevout.n as usize)
                                        {
                                            for address in &prev_output.address {
                                                if address.is_empty()
                                                    || address == "Nonstandard"
                                                    || address == "CoinBaseTx"
                                                    || address == "CoinStakeTx"
                                                {
                                                    continue;
                                                }

                                                // Add spending transaction to address history
                                                undo.add_tx(address.clone(), txid_internal.clone());

                                                // Track UTXO spend (display-order
                                                // txid, matching the 'a' set keys)
                                                undo.add_utxo_spent(
                                                    address.clone(),
                                                    prev_txid_display.clone(),
                                                    prevout.n as u64,
                                                );

                                                // Track sent amount
                                                undo.add_sent(address.clone(), prev_output.value);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(undo)
}

/// Rollback address index to a specific height
///
/// This reverses all address index changes from (rollback_to_height + 1) to current_height
///
/// # Arguments
/// * `db` - Database handle
/// * `writer` - Atomic batch writer for database operations (shared for atomicity)
/// * `db` - Database handle
/// * `current_height` - Current blockchain height
/// * `rollback_to_height` - Target height to rollback to
///
/// # Returns
/// Number of blocks rolled back
pub async fn rollback_address_index(
    writer: &mut AtomicBatchWriter,
    db: Arc<DB>,
    current_height: i32,
    rollback_to_height: i32,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    info!(
        from = current_height,
        to = rollback_to_height,
        "Rolling back address index"
    );

    let blocks_to_remove = (current_height - rollback_to_height) as usize;

    // Merge every reversed block's undo into ONE combined undo, then reverse once.
    //
    // reverse_address_block reads each address's current r/s/t/a from the DB (not
    // from the still-buffered AtomicBatchWriter) and writes back disk_value - delta.
    // If we reversed block-by-block into the shared writer with no flush between
    // blocks, an address touched by N reversed blocks would read the SAME stale disk
    // value N times and the batch's last-write-wins would drop N-1 subtractions,
    // leaving r/s over-stated. Merging first (sum r/s, concat t/a edits) makes it a
    // single read-modify-write per address — correct AND still atomic (one flush by
    // the caller).
    let mut merged = AddressBlockUndo::new(rollback_to_height);
    let mut reversed_any = false;
    for height in ((rollback_to_height + 1)..=current_height).rev() {
        if height % 1000 == 0 {
            debug!(height = height, "Collecting address undo");
        }

        // Load undo data for this block
        if let Some(undo) = load_address_undo(db.clone(), height).await? {
            merged.merge_from(undo);
            reversed_any = true;

            // Delete undo data (using shared writer, atomic with the reversal)
            let mut undo_key = b"addr_undo".to_vec();
            undo_key.extend_from_slice(&height.to_le_bytes());
            writer.delete("chain_state", undo_key);
        } else {
            // No undo data - expected for blocks indexed before undo tracking (e.g.
            // pre-deploy or below the enrichment watermark). Those blocks' r/s are
            // left as-is and require a re-enrichment to fully correct.
            warn!(
                height = height,
                "No address undo data for block - full rebuild may be required"
            );
        }
    }

    if reversed_any {
        // Single read-modify-write per address against committed on-disk state.
        reverse_address_block(writer, &db, &merged).await?;
    }

    // Note: Final flush removed - caller is responsible for flushing shared writer

    info!(
        blocks_removed = blocks_to_remove,
        "Address index rollback complete"
    );

    Ok(blocks_to_remove)
}

/// Reverse address index changes for a single block
async fn reverse_address_block(
    writer: &mut AtomicBatchWriter,
    db: &Arc<DB>,
    undo: &AddressBlockUndo,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let db_clone = db.clone();

    // Get all affected addresses
    let mut all_addresses: HashSet<String> = HashSet::new();
    all_addresses.extend(undo.address_txs.keys().cloned());
    all_addresses.extend(undo.address_utxos_created.keys().cloned());
    all_addresses.extend(undo.address_utxos_spent.keys().cloned());
    all_addresses.extend(undo.address_received.keys().cloned());
    all_addresses.extend(undo.address_sent.keys().cloned());

    for address in all_addresses {
        // 1. Remove transactions from this block
        if let Some(txids) = undo.address_txs.get(&address) {
            let mut tx_list_key = vec![b't'];
            tx_list_key.extend_from_slice(address.as_bytes());

            // Load current transaction list
            let current_txs = tokio::task::spawn_blocking({
                let db_clone = db_clone.clone();
                let key = tx_list_key.clone();
                move || {
                    let cf_addr_index = db_clone
                        .cf_handle("addr_index")
                        .ok_or("addr_index CF not found")?;
                    db_clone
                        .get_cf(&cf_addr_index, &key)
                        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
                }
            })
            .await??;

            if let Some(current_bytes) = current_txs {
                // Parse current list (32 bytes per txid)
                let mut current_list: Vec<Vec<u8>> = Vec::new();
                for chunk in current_bytes.chunks_exact(32) {
                    current_list.push(chunk.to_vec());
                }

                // Remove txids from this block
                let txids_set: HashSet<Vec<u8>> = txids.iter().cloned().collect();
                current_list.retain(|txid| !txids_set.contains(txid));

                // Serialize and write back
                let mut new_bytes = Vec::with_capacity(current_list.len() * 32);
                for txid in current_list {
                    new_bytes.extend_from_slice(&txid);
                }

                if new_bytes.is_empty() {
                    writer.delete("addr_index", tx_list_key);
                } else {
                    writer.put("addr_index", tx_list_key, new_bytes);
                }
            }
        }

        // 2. Remove created UTXOs and restore spent UTXOs
        let mut utxo_key = vec![b'a'];
        utxo_key.extend_from_slice(address.as_bytes());

        let current_utxos = tokio::task::spawn_blocking({
            let db_clone = db_clone.clone();
            let key = utxo_key.clone();
            move || {
                let cf_addr_index = db_clone
                    .cf_handle("addr_index")
                    .ok_or("addr_index CF not found")?;
                db_clone
                    .get_cf(&cf_addr_index, &key)
                    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
            }
        })
        .await??;

        let mut utxo_list = if let Some(bytes) = current_utxos {
            deserialize_utxos(&bytes).await
        } else {
            Vec::new()
        };

        // Set of UTXOs this reversal CREATED. A (txid,vout) that was both created
        // and spent within the reversed range nets to absent, so it must be removed
        // and NOT restored — otherwise a cross-block create-then-spend (both blocks
        // reversed and merged) would leave a phantom UTXO in 'a'. Computing this set
        // once and excluding it from the restore makes the merged reversal
        // order-independent. ('a'-set correctness only; the served r-s balance is
        // independent of 'a'.)
        let created_set: HashSet<(Vec<u8>, u64)> = undo
            .address_utxos_created
            .get(&address)
            .map(|created| created.iter().cloned().collect())
            .unwrap_or_default();

        // Remove created UTXOs
        if !created_set.is_empty() {
            utxo_list.retain(|(txid, vout)| !created_set.contains(&(txid.clone(), *vout)));
        }

        // Restore spent UTXOs, except any also created in the reversed range.
        if let Some(spent) = undo.address_utxos_spent.get(&address) {
            for (txid, vout) in spent {
                if created_set.contains(&(txid.clone(), *vout)) {
                    continue; // created-then-spent within the range → stays absent
                }
                // Only add if not already in list
                if !utxo_list.iter().any(|(t, v)| t == txid && v == vout) {
                    utxo_list.push((txid.clone(), *vout));
                }
            }
        }

        // Write updated UTXO list
        if utxo_list.is_empty() {
            writer.delete("addr_index", utxo_key);
        } else {
            let serialized = serialize_utxos(&utxo_list).await;
            writer.put("addr_index", utxo_key, serialized);
        }

        // 3. Subtract received amount
        if let Some(received) = undo.address_received.get(&address) {
            let mut key_r = vec![b'r'];
            key_r.extend_from_slice(address.as_bytes());

            let current_received = tokio::task::spawn_blocking({
                let db_clone = db_clone.clone();
                let key = key_r.clone();
                move || {
                    let cf_addr_index = db_clone
                        .cf_handle("addr_index")
                        .ok_or("addr_index CF not found")?;
                    db_clone
                        .get_cf(&cf_addr_index, &key)
                        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
                }
            })
            .await??;

            let mut total: i64 = 0;
            if let Some(bytes) = current_received {
                if bytes.len() == 8 {
                    total = i64::from_le_bytes(bytes.try_into().unwrap());
                }
            }

            total -= received;
            if total < 0 {
                // Reversing should never drive a total below zero. If it does, r/s
                // had pre-existing drift (e.g. a block whose undo was missing/partial).
                // Clamp to keep the value sane, but surface it instead of masking.
                warn!(address = %address, received, deficit = -total,
                    "total_received reversal went negative; clamping (pre-existing r/s drift)");
                total = 0;
            }

            writer.put("addr_index", key_r, total.to_le_bytes().to_vec());
        }

        // 4. Subtract sent amount
        if let Some(sent) = undo.address_sent.get(&address) {
            let mut key_s = vec![b's'];
            key_s.extend_from_slice(address.as_bytes());

            let current_sent = tokio::task::spawn_blocking({
                let db_clone = db_clone.clone();
                let key = key_s.clone();
                move || {
                    let cf_addr_index = db_clone
                        .cf_handle("addr_index")
                        .ok_or("addr_index CF not found")?;
                    db_clone
                        .get_cf(&cf_addr_index, &key)
                        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
                }
            })
            .await??;

            let mut total: i64 = 0;
            if let Some(bytes) = current_sent {
                if bytes.len() == 8 {
                    total = i64::from_le_bytes(bytes.try_into().unwrap());
                }
            }

            total -= sent;
            if total < 0 {
                warn!(address = %address, sent, deficit = -total,
                    "total_sent reversal went negative; clamping (pre-existing r/s drift)");
                total = 0;
            }

            writer.put("addr_index", key_s, total.to_le_bytes().to_vec());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_address_block_undo_creation() {
        let mut undo = AddressBlockUndo::new(12345);

        assert_eq!(undo.height, 12345);
        assert_eq!(undo.address_txs.len(), 0);

        // Add transaction
        undo.add_tx(
            "DMJRSsuU9zfyrvxVaAEFQqK4MxZg34fk73".to_string(),
            vec![1; 32],
        );
        assert_eq!(undo.address_txs.len(), 1);

        // Add UTXO
        undo.add_utxo_created(
            "DMJRSsuU9zfyrvxVaAEFQqK4MxZg34fk73".to_string(),
            vec![1; 32],
            0,
        );
        assert_eq!(undo.address_utxos_created.len(), 1);

        // Add amounts
        undo.add_received(
            "DMJRSsuU9zfyrvxVaAEFQqK4MxZg34fk73".to_string(),
            100_000_000,
        );
        assert_eq!(
            undo.address_received
                .get("DMJRSsuU9zfyrvxVaAEFQqK4MxZg34fk73"),
            Some(&100_000_000)
        );
    }

    #[test]
    fn test_address_block_undo_serialization() {
        let mut undo = AddressBlockUndo::new(12345);
        undo.add_tx(
            "DMJRSsuU9zfyrvxVaAEFQqK4MxZg34fk73".to_string(),
            vec![1; 32],
        );
        undo.add_utxo_created(
            "DMJRSsuU9zfyrvxVaAEFQqK4MxZg34fk73".to_string(),
            vec![1; 32],
            0,
        );
        undo.add_received(
            "DMJRSsuU9zfyrvxVaAEFQqK4MxZg34fk73".to_string(),
            100_000_000,
        );

        // Serialize and deserialize
        let serialized = bincode::serialize(&undo).unwrap();
        let deserialized: AddressBlockUndo = bincode::deserialize(&serialized).unwrap();

        assert_eq!(deserialized.height, 12345);
        assert_eq!(deserialized.address_txs.len(), 1);
        assert_eq!(deserialized.address_utxos_created.len(), 1);
        assert_eq!(
            deserialized
                .address_received
                .get("DMJRSsuU9zfyrvxVaAEFQqK4MxZg34fk73"),
            Some(&100_000_000)
        );
    }

    #[test]
    fn test_multiple_addresses() {
        let mut undo = AddressBlockUndo::new(12345);

        // Add data for address 1
        undo.add_tx("Address1".to_string(), vec![1; 32]);
        undo.add_received("Address1".to_string(), 100_000_000);

        // Add data for address 2
        undo.add_tx("Address2".to_string(), vec![2; 32]);
        undo.add_sent("Address2".to_string(), 50_000_000);

        assert_eq!(undo.address_txs.len(), 2);
        assert_eq!(undo.address_received.len(), 1);
        assert_eq!(undo.address_sent.len(), 1);
    }

    #[test]
    fn test_accumulation() {
        let mut undo = AddressBlockUndo::new(12345);

        // Multiple additions to same address
        undo.add_received("Address1".to_string(), 100_000_000);
        undo.add_received("Address1".to_string(), 50_000_000);

        assert_eq!(undo.address_received.get("Address1"), Some(&150_000_000));
    }

    /// Reorg-reversal regression: the real entrypoint `rollback_address_index`
    /// (store_address_undo -> load_address_undo -> reverse_address_block -> delete)
    /// must reverse r/s/a/t EXACTLY back to the pre-block state, and consume the undo.
    /// This is the guarantee the addr_v2 balance fix depends on after a chain reorg.
    #[tokio::test]
    async fn rollback_reverses_addr_index_r_s_a_t() {
        use crate::atomic_writer::AtomicBatchWriter;
        use crate::parser::{deserialize_utxos, serialize_utxos};
        use rocksdb::{Options, DB};

        let temp = tempfile::TempDir::new().unwrap();
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db = std::sync::Arc::new(
            DB::open_cf(
                &opts,
                temp.path(),
                ["addr_index", "transactions", "chain_state"],
            )
            .unwrap(),
        );
        let cf_ai = db.cf_handle("addr_index").unwrap();

        let addr = "DReorgTestAddressXXXXXXXXXXXXXXXXXX";
        let key = |p: u8| {
            let mut k = vec![p];
            k.extend_from_slice(addr.as_bytes());
            k
        };
        let tx0 = vec![0xA0u8; 32];
        let tx1 = vec![0xB1u8; 32];

        // POST-block state the live monitor left after connecting height H, where the
        // block received 500 to `addr` and spent addr's prior UTXO (tx0,0)=150:
        //   r = 1000+500 = 1500, s = 200+150 = 350, t = [tx0, tx1], a = [(tx1,0)]
        db.put_cf(&cf_ai, &key(b'r'), 1500i64.to_le_bytes())
            .unwrap();
        db.put_cf(&cf_ai, &key(b's'), 350i64.to_le_bytes()).unwrap();
        let mut tlist = Vec::new();
        tlist.extend_from_slice(&tx0);
        tlist.extend_from_slice(&tx1);
        db.put_cf(&cf_ai, &key(b't'), &tlist).unwrap();
        let a_post = serialize_utxos(&vec![(tx1.clone(), 0u64)]).await;
        db.put_cf(&cf_ai, &key(b'a'), &a_post).unwrap();

        // The undo the connect path captured for height H (exactly what it applied).
        let height = 1000i32;
        let mut undo = AddressBlockUndo::new(height);
        undo.add_received(addr.to_string(), 500);
        undo.add_sent(addr.to_string(), 150);
        undo.add_tx(addr.to_string(), tx1.clone());
        undo.add_utxo_created(addr.to_string(), tx1.clone(), 0);
        undo.add_utxo_spent(addr.to_string(), tx0.clone(), 0);
        store_address_undo(db.clone(), &undo).await.unwrap();

        // Reverse exactly one block via the real reorg entrypoint.
        let mut writer = AtomicBatchWriter::new(db.clone(), 100_000);
        rollback_address_index(&mut writer, db.clone(), height, height - 1)
            .await
            .unwrap();
        writer.flush().await.unwrap();

        // Pre-block state must be restored exactly.
        let read_i64 = |p: u8| -> i64 {
            let v = db.get_cf(&cf_ai, &key(p)).unwrap().unwrap();
            i64::from_le_bytes(v.as_slice().try_into().unwrap())
        };
        assert_eq!(read_i64(b'r'), 1000, "r must reverse to 1500 - 500");
        assert_eq!(read_i64(b's'), 200, "s must reverse to 350 - 150");

        let t_after = db.get_cf(&cf_ai, &key(b't')).unwrap().unwrap();
        assert_eq!(t_after, tx0, "t must drop tx1, keep tx0");

        let a_after = deserialize_utxos(&db.get_cf(&cf_ai, &key(b'a')).unwrap().unwrap()).await;
        assert_eq!(
            a_after,
            vec![(tx0.clone(), 0u64)],
            "a must drop (tx1,0) and restore (tx0,0)"
        );

        // The undo record must be consumed (deleted) after a successful rollback.
        assert!(
            load_address_undo(db.clone(), height)
                .await
                .unwrap()
                .is_none(),
            "undo must be deleted after rollback",
        );
    }

    /// P1 regression (multi-block reorg): an address touched by 2+ reversed blocks
    /// must have ALL its subtractions applied. The pre-fix code read the same stale
    /// on-disk r for each block and the batch's last-write-wins dropped all but one
    /// subtraction (yielding 1300 instead of 1000 here). The merge-then-reverse-once
    /// fix must restore the exact pre-block total.
    #[tokio::test]
    async fn rollback_reverses_multi_block_same_address() {
        use crate::atomic_writer::AtomicBatchWriter;
        use rocksdb::{Options, DB};

        let temp = tempfile::TempDir::new().unwrap();
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db = std::sync::Arc::new(
            DB::open_cf(
                &opts,
                temp.path(),
                ["addr_index", "transactions", "chain_state"],
            )
            .unwrap(),
        );
        let cf_ai = db.cf_handle("addr_index").unwrap();

        let addr = "DMultiBlockReorgAddrXXXXXXXXXXXXXXX";
        let mut r_key = vec![b'r'];
        r_key.extend_from_slice(addr.as_bytes());

        // Pre-reorg disk state: r = 1000 (base) + 300 (block H) + 300 (block H+1) = 1600,
        // i.e. the address received 300 in EACH of the two blocks we will reverse.
        db.put_cf(&cf_ai, &r_key, 1600i64.to_le_bytes()).unwrap();

        let h0 = 1000i32;
        let h1 = 1001i32;
        for h in [h0, h1] {
            let mut undo = AddressBlockUndo::new(h);
            undo.add_received(addr.to_string(), 300);
            store_address_undo(db.clone(), &undo).await.unwrap();
        }

        // Reverse BOTH blocks (current=h1, rollback_to=h0-1) in one batch.
        let mut writer = AtomicBatchWriter::new(db.clone(), 100_000);
        rollback_address_index(&mut writer, db.clone(), h1, h0 - 1)
            .await
            .unwrap();
        writer.flush().await.unwrap();

        let r_after = i64::from_le_bytes(
            db.get_cf(&cf_ai, &r_key)
                .unwrap()
                .unwrap()
                .as_slice()
                .try_into()
                .unwrap(),
        );
        assert_eq!(
            r_after, 1000,
            "both blocks' +300 must be reversed (1600 - 600), not just one"
        );

        // Both undo records consumed.
        assert!(load_address_undo(db.clone(), h0).await.unwrap().is_none());
        assert!(load_address_undo(db.clone(), h1).await.unwrap().is_none());
    }

    /// 'a'-set regression (cross-block create-then-spend): a UTXO created in block H
    /// and spent in block H+1, with BOTH reversed in one reorg, must NOT be restored
    /// into 'a' (it was net-absent before the reorg). The merge concatenates it into
    /// both created and spent; the created∩spent cancellation must keep 'a' empty.
    #[tokio::test]
    async fn rollback_cross_block_create_spend_leaves_no_phantom_utxo() {
        use crate::atomic_writer::AtomicBatchWriter;
        use crate::parser::deserialize_utxos;
        use rocksdb::{Options, DB};

        let temp = tempfile::TempDir::new().unwrap();
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db = std::sync::Arc::new(
            DB::open_cf(
                &opts,
                temp.path(),
                ["addr_index", "transactions", "chain_state"],
            )
            .unwrap(),
        );
        let cf_ai = db.cf_handle("addr_index").unwrap();

        let addr = "DCrossBlockUtxoAddrXXXXXXXXXXXXXXXX";
        let mut a_key = vec![b'a'];
        a_key.extend_from_slice(addr.as_bytes());
        let xtx = vec![0xCCu8; 32];

        // Pre-reorg 'a' is EMPTY: X was created in H then spent in H+1, so it is not
        // a current UTXO. (No 'a' key written.)
        let h0 = 2000i32;
        let h1 = 2001i32;

        let mut undo_h0 = AddressBlockUndo::new(h0);
        undo_h0.add_utxo_created(addr.to_string(), xtx.clone(), 0); // created in H
        store_address_undo(db.clone(), &undo_h0).await.unwrap();

        let mut undo_h1 = AddressBlockUndo::new(h1);
        undo_h1.add_utxo_spent(addr.to_string(), xtx.clone(), 0); // spent in H+1
        store_address_undo(db.clone(), &undo_h1).await.unwrap();

        let mut writer = AtomicBatchWriter::new(db.clone(), 100_000);
        rollback_address_index(&mut writer, db.clone(), h1, h0 - 1)
            .await
            .unwrap();
        writer.flush().await.unwrap();

        // 'a' must remain empty — no phantom UTXO restored.
        match db.get_cf(&cf_ai, &a_key).unwrap() {
            None => {} // empty (deleted) — correct
            Some(bytes) => {
                let utxos = deserialize_utxos(&bytes).await;
                assert!(
                    utxos.is_empty(),
                    "cross-block create-then-spend must leave no phantom UTXO, got {utxos:?}"
                );
            }
        }
    }
}
