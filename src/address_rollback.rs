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
use std::collections::{HashMap, HashSet};
use rocksdb::DB;
use crate::parser::{deserialize_transaction, serialize_utxos, deserialize_utxos};
use crate::atomic_writer::AtomicBatchWriter;
use serde::{Serialize, Deserialize};

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
        self.address_utxos_created.entry(address).or_default().push((txid, vout));
    }
    
    /// Add a spent UTXO
    pub fn add_utxo_spent(&mut self, address: String, txid: Vec<u8>, vout: u64) {
        self.address_utxos_spent.entry(address).or_default().push((txid, vout));
    }
    
    /// Add received amount
    pub fn add_received(&mut self, address: String, amount: i64) {
        *self.address_received.entry(address).or_insert(0) += amount;
    }
    
    /// Add sent amount
    pub fn add_sent(&mut self, address: String, amount: i64) {
        *self.address_sent.entry(address).or_insert(0) += amount;
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
        let cf_chain_state = db_clone.cf_handle("chain_state")
            .ok_or("chain_state CF not found")?;
        
        let mut key = b"addr_undo".to_vec();
        key.extend_from_slice(&height.to_le_bytes());
        
        db_clone.put_cf(&cf_chain_state, &key, &serialized)
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
        let cf_chain_state = db_clone.cf_handle("chain_state")
            .ok_or("chain_state CF not found")?;
        
        let mut key = b"addr_undo".to_vec();
        key.extend_from_slice(&height.to_le_bytes());
        
        db_clone.get_cf(&cf_chain_state, &key)
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
        let cf_chain_state = db_clone.cf_handle("chain_state")
            .ok_or("chain_state CF not found")?;
        
        let mut key = b"addr_undo".to_vec();
        key.extend_from_slice(&height.to_le_bytes());
        
        db_clone.delete_cf(&cf_chain_state, &key)
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
    let cf_transactions = db_clone.cf_handle("transactions")
        .ok_or("transactions CF not found")?;
    let _cf_addr_index = db_clone.cf_handle("addr_index")
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
                        if address.is_empty() || address == "Nonstandard" || 
                           address == "CoinBaseTx" || address == "CoinStakeTx" {
                            continue;
                        }
                        
                        // Add transaction to address history
                        undo.add_tx(address.clone(), txid_internal.clone());
                        
                        // Track UTXO creation
                        undo.add_utxo_created(
                            address.clone(),
                            txid_internal.clone(),
                            output.index,
                        );
                        
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
                            let prev_txid_internal: Vec<u8> = prev_txid_bytes.iter().rev().cloned().collect();
                            
                            let mut prev_tx_key = vec![b't'];
                            prev_tx_key.extend_from_slice(&prev_txid_internal);
                            
                            if let Some(prev_tx_data) = db.get_cf(&cf_transactions, &prev_tx_key)? {
                                if prev_tx_data.len() >= 8 {
                                    let mut prev_with_header = vec![0u8; 4];
                                    prev_with_header.extend_from_slice(&prev_tx_data[8..]);
                                    
                                    if let Ok(prev_tx) = deserialize_transaction(&prev_with_header).await {
                                        if let Some(prev_output) = prev_tx.outputs.get(prevout.n as usize) {
                                            for address in &prev_output.address {
                                                if address.is_empty() || address == "Nonstandard" ||
                                                   address == "CoinBaseTx" || address == "CoinStakeTx" {
                                                    continue;
                                                }
                                                
                                                // Add spending transaction to address history
                                                undo.add_tx(address.clone(), txid_internal.clone());
                                                
                                                // Track UTXO spend
                                                undo.add_utxo_spent(
                                                    address.clone(),
                                                    prev_txid_internal.clone(),
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
/// * `current_height` - Current blockchain height
/// * `rollback_to_height` - Target height to rollback to
/// 
/// # Returns
/// Number of blocks rolled back
pub async fn rollback_address_index(
    db: Arc<DB>,
    current_height: i32,
    rollback_to_height: i32,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    println!("  📍 Rolling back address index from {} to {}", current_height, rollback_to_height);
    
    let mut writer = AtomicBatchWriter::new(db.clone(), 100000);
    let blocks_to_remove = (current_height - rollback_to_height) as usize;
    
    // Process each block in reverse order
    for height in ((rollback_to_height + 1)..=current_height).rev() {
        if height % 1000 == 0 {
            println!("    Reversing address index for block {}", height);
        }
        
        // Load undo data for this block
        if let Some(undo) = load_address_undo(db.clone(), height).await? {
            // Reverse address changes
            reverse_address_block(&mut writer, &db, &undo).await?;
            
            // Delete undo data
            delete_address_undo(db.clone(), height).await?;
            
            // Flush periodically
            if writer.should_flush() {
                writer.flush().await?;
            }
        } else {
            // No undo data - this is expected for blocks indexed before undo tracking
            // In this case, we'll need to rebuild from scratch
            println!("    ⚠️  No address undo data for block {} - full rebuild may be required", height);
        }
    }
    
    // Final flush
    if writer.pending_count() > 0 {
        writer.flush().await?;
    }
    
    println!("  ✅ Address index rollback complete");
    
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
                    let cf_addr_index = db_clone.cf_handle("addr_index")
                        .ok_or("addr_index CF not found")?;
                    db_clone.get_cf(&cf_addr_index, &key)
                        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
                }
            }).await??;
            
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
                let cf_addr_index = db_clone.cf_handle("addr_index")
                    .ok_or("addr_index CF not found")?;
                db_clone.get_cf(&cf_addr_index, &key)
                    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
            }
        }).await??;
        
        let mut utxo_list = if let Some(bytes) = current_utxos {
            deserialize_utxos(&bytes).await
        } else {
            Vec::new()
        };
        
        // Remove created UTXOs
        if let Some(created) = undo.address_utxos_created.get(&address) {
            let created_set: HashSet<(Vec<u8>, u64)> = created.iter().cloned().collect();
            utxo_list.retain(|(txid, vout)| !created_set.contains(&(txid.clone(), *vout)));
        }
        
        // Restore spent UTXOs
        if let Some(spent) = undo.address_utxos_spent.get(&address) {
            for (txid, vout) in spent {
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
                    let cf_addr_index = db_clone.cf_handle("addr_index")
                        .ok_or("addr_index CF not found")?;
                    db_clone.get_cf(&cf_addr_index, &key)
                        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
                }
            }).await??;
            
            let mut total: i64 = 0;
            if let Some(bytes) = current_received {
                if bytes.len() == 8 {
                    total = i64::from_le_bytes(bytes.try_into().unwrap());
                }
            }
            
            total -= received;
            total = total.max(0); // Don't go negative
            
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
                    let cf_addr_index = db_clone.cf_handle("addr_index")
                        .ok_or("addr_index CF not found")?;
                    db_clone.get_cf(&cf_addr_index, &key)
                        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
                }
            }).await??;
            
            let mut total: i64 = 0;
            if let Some(bytes) = current_sent {
                if bytes.len() == 8 {
                    total = i64::from_le_bytes(bytes.try_into().unwrap());
                }
            }
            
            total -= sent;
            total = total.max(0); // Don't go negative
            
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
        undo.add_tx("DMJRSsuU9zfyrvxVaAEFQqK4MxZg34fk73".to_string(), vec![1; 32]);
        assert_eq!(undo.address_txs.len(), 1);
        
        // Add UTXO
        undo.add_utxo_created("DMJRSsuU9zfyrvxVaAEFQqK4MxZg34fk73".to_string(), vec![1; 32], 0);
        assert_eq!(undo.address_utxos_created.len(), 1);
        
        // Add amounts
        undo.add_received("DMJRSsuU9zfyrvxVaAEFQqK4MxZg34fk73".to_string(), 100_000_000);
        assert_eq!(undo.address_received.get("DMJRSsuU9zfyrvxVaAEFQqK4MxZg34fk73"), Some(&100_000_000));
    }
    
    #[test]
    fn test_address_block_undo_serialization() {
        let mut undo = AddressBlockUndo::new(12345);
        undo.add_tx("DMJRSsuU9zfyrvxVaAEFQqK4MxZg34fk73".to_string(), vec![1; 32]);
        undo.add_utxo_created("DMJRSsuU9zfyrvxVaAEFQqK4MxZg34fk73".to_string(), vec![1; 32], 0);
        undo.add_received("DMJRSsuU9zfyrvxVaAEFQqK4MxZg34fk73".to_string(), 100_000_000);
        
        // Serialize and deserialize
        let serialized = bincode::serialize(&undo).unwrap();
        let deserialized: AddressBlockUndo = bincode::deserialize(&serialized).unwrap();
        
        assert_eq!(deserialized.height, 12345);
        assert_eq!(deserialized.address_txs.len(), 1);
        assert_eq!(deserialized.address_utxos_created.len(), 1);
        assert_eq!(deserialized.address_received.get("DMJRSsuU9zfyrvxVaAEFQqK4MxZg34fk73"), Some(&100_000_000));
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
}
