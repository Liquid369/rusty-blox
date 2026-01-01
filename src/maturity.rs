/// Maturity Rule Enforcement - PIVX Consensus Compliance
/// 
/// Implements PIVX Core's maturity requirements for coinbase and coinstake outputs.
/// These outputs cannot be spent until sufficient confirmations have passed.
/// 
/// PIVX Consensus Rules:
/// - Coinbase outputs: 100 block maturity (COINBASE_MATURITY)
/// - Coinstake outputs: 600 block maturity (COINSTAKE_MATURITY)
/// - Normal transaction outputs: immediately spendable (0 maturity)
/// 
/// This is critical for financial accuracy - without maturity checks, balances
/// can show coins as spendable when they are not yet available per consensus.

use crate::tx_type::{TransactionType, COINBASE_MATURITY, COINSTAKE_MATURITY};
use crate::constants::{is_canonical_height, HEIGHT_ORPHAN};
use crate::types::CTransaction;
use std::sync::Arc;
use rocksdb::DB;

/// Check if an output is spendable at the current chain height
/// 
/// # Arguments
/// * `tx_type` - The type of transaction containing the output
/// * `output_height` - The block height where the output was created
/// * `current_height` - The current chain tip height
/// 
/// # Returns
/// `true` if the output can be spent, `false` if maturity period not yet reached
/// 
/// # PIVX Core Equivalent
/// This implements the same logic as PIVX Core's CWallet::IsFinalTx and maturity checks
pub fn is_output_spendable(
    tx_type: TransactionType,
    output_height: i32,
    current_height: i32,
) -> bool {
    // Orphaned or unresolved heights are invalid
    if !is_canonical_height(output_height) || !is_canonical_height(current_height) {
        return false;
    }
    
    // Calculate confirmations
    let confirmations = current_height.saturating_sub(output_height);
    
    // Check maturity requirements
    match tx_type {
        TransactionType::Coinbase => {
            // Coinbase outputs require 100 confirmations
            confirmations >= COINBASE_MATURITY as i32
        }
        TransactionType::Coinstake => {
            // Coinstake outputs require 600 confirmations
            confirmations >= COINSTAKE_MATURITY as i32
        }
        TransactionType::Normal => {
            // Normal transaction outputs are immediately spendable
            true
        }
    }
}

/// Check if a specific output index is spendable
/// 
/// For coinstake transactions, the first output (index 0) is always empty and unspendable.
/// This function handles that special case in addition to maturity checks.
/// 
/// # Arguments
/// * `tx` - The transaction containing the output
/// * `tx_type` - The type of transaction
/// * `output_index` - The index of the output to check
/// * `output_height` - The block height where the output was created
/// * `current_height` - The current chain tip height
pub fn is_output_index_spendable(
    tx: &CTransaction,
    tx_type: TransactionType,
    output_index: usize,
    output_height: i32,
    current_height: i32,
) -> bool {
    // Coinstake first output is always empty/unspendable (marker output)
    if tx_type == TransactionType::Coinstake && output_index == 0 {
        return false;
    }
    
    // Check if output exists
    if output_index >= tx.outputs.len() {
        return false;
    }
    
    // Check maturity
    is_output_spendable(tx_type, output_height, current_height)
}

/// Get the current chain height from database
/// 
/// # Arguments
/// * `db` - RocksDB instance
/// 
/// # Returns
/// Current sync height or error
pub fn get_current_height(db: &Arc<DB>) -> Result<i32, Box<dyn std::error::Error>> {
    let cf_state = db.cf_handle("chain_state")
        .ok_or("chain_state CF not found")?;
    
    let height = match db.get_cf(&cf_state, b"sync_height")? {
        Some(bytes) => {
            if bytes.len() >= 4 {
                i32::from_le_bytes(bytes[0..4].try_into()?)
            } else {
                0
            }
        }
        None => 0,
    };
    
    Ok(height)
}

/// Filter a list of UTXOs to only include spendable ones based on maturity rules
/// 
/// This is the main function used by balance calculations and API endpoints.
/// 
/// # Arguments
/// * `utxos` - List of (txid_internal, output_index) tuples
/// * `db` - RocksDB instance to look up transaction data
/// * `current_height` - Current chain tip height
/// 
/// # Returns
/// Filtered list containing only spendable UTXOs
pub async fn filter_spendable_utxos(
    utxos: Vec<(Vec<u8>, u64)>,
    db: Arc<DB>,
    current_height: i32,
) -> Vec<(Vec<u8>, u64)> {
    use crate::parser::deserialize_transaction;
    use crate::tx_type::detect_transaction_type;
    
    let mut spendable = Vec::new();
    
    for (txid_internal, output_index) in utxos {
        // Get transaction data
        let mut key = vec![b't'];
        key.extend(&txid_internal);
        
        let tx_data = {
            let key_clone = key.clone();
            let db_clone = db.clone();
            
            tokio::task::spawn_blocking(move || -> Result<Option<Vec<u8>>, String> {
                let cf_transactions = db_clone.cf_handle("transactions")
                    .ok_or_else(|| "transactions CF not found".to_string())?;
                db_clone.get_cf(&cf_transactions, &key_clone)
                    .map_err(|e| e.to_string())
            })
            .await
            .ok()
            .and_then(|r| r.ok())
            .flatten()
        };
        
        if let Some(tx_data) = tx_data {
            // Parse stored transaction format: version(4) + height(4) + tx_bytes
            if tx_data.len() < 8 {
                continue;
            }
            
            // Extract height
            let height_bytes: [u8; 4] = match tx_data[4..8].try_into() {
                Ok(bytes) => bytes,
                Err(_) => continue,
            };
            let output_height = i32::from_le_bytes(height_bytes);
            
            // Skip orphaned and unresolved transactions
            if output_height == HEIGHT_ORPHAN {
                continue;
            }
            
            // Parse transaction
            let tx_bytes = &tx_data[8..];
            let mut tx_data_with_header = Vec::with_capacity(4 + tx_bytes.len());
            tx_data_with_header.extend_from_slice(&[0u8; 4]);
            tx_data_with_header.extend_from_slice(tx_bytes);
            
            if let Ok(tx) = deserialize_transaction(&tx_data_with_header).await {
                // Detect transaction type
                let tx_type = detect_transaction_type(&tx);
                
                // Check if this specific output is spendable
                if is_output_index_spendable(
                    &tx,
                    tx_type,
                    output_index as usize,
                    output_height,
                    current_height,
                ) {
                    spendable.push((txid_internal, output_index));
                }
            }
        }
    }
    
    spendable
}

/// Calculate maturity status for a UTXO
/// 
/// Returns a tuple of (is_spendable, confirmations_needed)
/// 
/// # Arguments
/// * `tx_type` - Transaction type
/// * `output_height` - Block height where output was created
/// * `current_height` - Current chain tip height
/// 
/// # Returns
/// (is_spendable, blocks_until_spendable)
pub fn get_maturity_status(
    tx_type: TransactionType,
    output_height: i32,
    current_height: i32,
) -> (bool, i32) {
    if !is_canonical_height(output_height) || !is_canonical_height(current_height) {
        return (false, i32::MAX);
    }
    
    let confirmations = current_height.saturating_sub(output_height);
    let required = tx_type.maturity_blocks() as i32;
    
    if confirmations >= required {
        (true, 0)
    } else {
        (false, required - confirmations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_coinbase_maturity() {
        // Coinbase at height 100, current height 199
        assert!(!is_output_spendable(TransactionType::Coinbase, 100, 199));
        
        // Coinbase at height 100, current height 200 (exactly 100 confirmations)
        assert!(is_output_spendable(TransactionType::Coinbase, 100, 200));
        
        // Coinbase at height 100, current height 250
        assert!(is_output_spendable(TransactionType::Coinbase, 100, 250));
    }
    
    #[test]
    fn test_coinstake_maturity() {
        // Coinstake at height 1000, current height 1599
        assert!(!is_output_spendable(TransactionType::Coinstake, 1000, 1599));
        
        // Coinstake at height 1000, current height 1600 (exactly 600 confirmations)
        assert!(is_output_spendable(TransactionType::Coinstake, 1000, 1600));
        
        // Coinstake at height 1000, current height 2000
        assert!(is_output_spendable(TransactionType::Coinstake, 1000, 2000));
    }
    
    #[test]
    fn test_normal_tx_immediately_spendable() {
        // Normal transaction is spendable immediately
        assert!(is_output_spendable(TransactionType::Normal, 1000, 1000));
        assert!(is_output_spendable(TransactionType::Normal, 1000, 1001));
    }
    
    #[test]
    fn test_negative_height_invalid() {
        assert!(!is_output_spendable(TransactionType::Coinbase, -1, 100));
        assert!(!is_output_spendable(TransactionType::Coinbase, 100, -1));
    }
    
    #[test]
    fn test_maturity_status() {
        // Coinbase needs 50 more blocks
        let (spendable, blocks_needed) = get_maturity_status(TransactionType::Coinbase, 100, 150);
        assert!(!spendable);
        assert_eq!(blocks_needed, 50);
        
        // Coinstake ready to spend
        let (spendable, blocks_needed) = get_maturity_status(TransactionType::Coinstake, 1000, 1700);
        assert!(spendable);
        assert_eq!(blocks_needed, 0);
        
        // Normal tx always ready
        let (spendable, blocks_needed) = get_maturity_status(TransactionType::Normal, 1000, 1000);
        assert!(spendable);
        assert_eq!(blocks_needed, 0);
    }
}
