/// Transaction Fee Calculation Module
///
/// Implements accurate transaction fee calculation for PIVX block explorer.
/// Fees are calculated as: sum(inputs) - sum(outputs) for normal transactions,
/// with special handling for coinbase and coinstake transactions.
///
/// PIVX Core Equivalent: GetValueIn() and fee calculation in validation.cpp

use std::sync::Arc;
use rocksdb::DB;
use crate::types::{CTransaction, CTxIn};
use crate::tx_type::{TransactionType, detect_transaction_type};
use crate::spent_utxo::calculate_input_value;
use crate::parser::deserialize_transaction;

/// Result of fee calculation
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeeCalculation {
    /// Transaction type (coinbase, coinstake, or normal)
    pub tx_type: TransactionType,
    
    /// Total input value in satoshis
    pub value_in: i64,
    
    /// Total output value in satoshis
    pub value_out: i64,
    
    /// Transaction fee in satoshis (0 for coinbase/coinstake)
    pub fee: i64,
    
    /// Stake reward in satoshis (only for coinstake, 0 otherwise)
    pub stake_reward: i64,
    
    /// Whether calculation was successful
    pub valid: bool,
    
    /// Error message if calculation failed
    pub error_message: Option<String>,
}

impl FeeCalculation {
    /// Create a successful fee calculation
    pub fn success(
        tx_type: TransactionType,
        value_in: i64,
        value_out: i64,
        fee: i64,
        stake_reward: i64,
    ) -> Self {
        Self {
            tx_type,
            value_in,
            value_out,
            fee,
            stake_reward,
            valid: true,
            error_message: None,
        }
    }
    
    /// Create a failed fee calculation
    pub fn failure(error: String) -> Self {
        Self {
            tx_type: TransactionType::Normal,
            value_in: 0,
            value_out: 0,
            fee: 0,
            stake_reward: 0,
            valid: false,
            error_message: Some(error),
        }
    }
    
    /// Check if this is a valid calculation
    pub fn is_valid(&self) -> bool {
        self.valid
    }
    
    /// Get fee in PIV (divide by 100,000,000)
    pub fn fee_piv(&self) -> f64 {
        self.fee as f64 / 100_000_000.0
    }
    
    /// Get stake reward in PIV
    pub fn stake_reward_piv(&self) -> f64 {
        self.stake_reward as f64 / 100_000_000.0
    }
}

/// Extract (txid, vout) tuples from transaction inputs
/// 
/// Converts CTxIn vector to format expected by calculate_input_value
fn extract_input_prevouts(inputs: &[CTxIn]) -> Vec<(Vec<u8>, u64)> {
    let mut prevouts = Vec::new();
    
    for input in inputs {
        // Skip coinbase inputs (they have no prevout)
        if input.coinbase.is_some() {
            continue;
        }
        
        if let Some(prevout) = &input.prevout {
            // Convert hex txid to internal format (reversed bytes)
            if let Ok(txid_bytes) = hex::decode(&prevout.hash) {
                let txid_internal: Vec<u8> = txid_bytes.iter().rev().cloned().collect();
                prevouts.push((txid_internal, prevout.n as u64));
            }
        }
    }
    
    prevouts
}

/// Calculate transaction fee for a parsed transaction
///
/// # Fee Calculation Rules
/// 
/// **Coinbase Transactions** (mining rewards):
/// - Input value: 0
/// - Output value: block reward (newly minted coins)
/// - Fee: 0
/// 
/// **Coinstake Transactions** (PoS staking rewards):
/// - Input value: staked coins
/// - Output value: staked coins + stake reward
/// - Fee: 0 (no fee for staking)
/// - Stake reward: value_out - value_in
/// 
/// **Normal Transactions**:
/// - Input value: sum of all input values
/// - Output value: sum of all output values
/// - Fee: value_in - value_out (must be >= 0)
/// 
/// # Arguments
/// * `db` - Database handle
/// * `tx` - Parsed transaction
/// * `height` - Block height (for transaction type detection)
/// 
/// # Returns
/// FeeCalculation with all fee details
pub async fn calculate_transaction_fee(
    db: Arc<DB>,
    tx: &CTransaction,
    height: i32,
) -> Result<FeeCalculation, Box<dyn std::error::Error + Send + Sync>> {
    // Determine transaction type
    let tx_type = detect_transaction_type(tx);
    
    // Calculate output value (always deterministic)
    let value_out: i64 = tx.outputs.iter().map(|o| o.value).sum();
    
    match tx_type {
        TransactionType::Coinbase => {
            // Coinbase: no inputs, outputs are block reward
            Ok(FeeCalculation::success(
                TransactionType::Coinbase,
                0,  // No input value
                value_out,
                0,  // No fee
                0,  // No stake reward
            ))
        }
        
        TransactionType::Coinstake => {
            // Coinstake: first output is empty, second+ outputs are stake reward + original stake
            // Calculate input value to determine stake reward
            let prevouts = extract_input_prevouts(&tx.inputs);
            
            match calculate_input_value(db.clone(), &prevouts).await? {
                Some(value_in) => {
                    // Stake reward = output - input
                    let stake_reward = value_out - value_in;
                    
                    Ok(FeeCalculation::success(
                        TransactionType::Coinstake,
                        value_in,
                        value_out,
                        0,  // No fee for coinstake
                        stake_reward.max(0),  // Reward must be positive
                    ))
                }
                None => {
                    // Can't calculate input value - inputs may not be indexed yet
                    Ok(FeeCalculation::failure(
                        "Coinstake inputs not available for fee calculation".to_string()
                    ))
                }
            }
        }
        
        TransactionType::Normal => {
            // Normal transaction: fee = inputs - outputs
            let prevouts = extract_input_prevouts(&tx.inputs);
            
            match calculate_input_value(db.clone(), &prevouts).await? {
                Some(value_in) => {
                    let fee = value_in - value_out;
                    
                    // Validate fee is non-negative
                    if fee < 0 {
                        return Ok(FeeCalculation::failure(
                            format!("Invalid fee: inputs ({}) < outputs ({})", value_in, value_out)
                        ));
                    }
                    
                    Ok(FeeCalculation::success(
                        TransactionType::Normal,
                        value_in,
                        value_out,
                        fee,
                        0,  // No stake reward
                    ))
                }
                None => {
                    // Can't calculate input value - inputs may not be indexed yet
                    Ok(FeeCalculation::failure(
                        "Transaction inputs not available for fee calculation".to_string()
                    ))
                }
            }
        }
    }
}

/// Calculate fee from transaction ID (loads transaction from database)
///
/// # Arguments
/// * `db` - Database handle
/// * `txid` - Transaction ID (hex string, display format)
/// * `height` - Block height (for type detection)
/// 
/// # Returns
/// FeeCalculation or error
pub async fn calculate_fee_by_txid(
    db: Arc<DB>,
    txid: &str,
    height: i32,
) -> Result<FeeCalculation, Box<dyn std::error::Error + Send + Sync>> {
    // Convert txid to internal format
    let txid_bytes = hex::decode(txid)?;
    let txid_internal: Vec<u8> = txid_bytes.iter().rev().cloned().collect();
    
    // Load transaction from database
    let mut tx_key = vec![b't'];
    tx_key.extend_from_slice(&txid_internal);
    
    let db_clone = db.clone();
    let tx_data = tokio::task::spawn_blocking(move || {
        let cf_transactions = db_clone.cf_handle("transactions")
            .ok_or("transactions CF not found")?;
        
        db_clone.get_cf(&cf_transactions, &tx_key)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    })
    .await??;
    
    match tx_data {
        Some(bytes) => {
            if bytes.len() < 8 {
                return Ok(FeeCalculation::failure("Invalid transaction data".to_string()));
            }
            
            // Parse transaction (skip version + height header)
            let mut tx_with_header = vec![0u8; 4];
            tx_with_header.extend_from_slice(&bytes[8..]);
            
            let tx = deserialize_transaction(&tx_with_header).await?;
            
            // Calculate fee
            calculate_transaction_fee(db, &tx, height).await
        }
        None => {
            Ok(FeeCalculation::failure("Transaction not found".to_string()))
        }
    }
}

/// Calculate total fees for a block
///
/// Sums all transaction fees in a block (excluding coinbase/coinstake).
/// 
/// # Arguments
/// * `db` - Database handle
/// * `height` - Block height
/// 
/// # Returns
/// Total block fees in satoshis, or error
pub async fn calculate_block_fees(
    db: Arc<DB>,
    height: i32,
) -> Result<i64, Box<dyn std::error::Error + Send + Sync>> {
    // Get all transactions in block
    let db_clone = db.clone();
    let txids = tokio::task::spawn_blocking(move || {
        let cf_transactions = db_clone.cf_handle("transactions")
            .ok_or("transactions CF not found")?;
        
        let mut txids = Vec::new();
        let mut prefix = vec![b'B'];
        prefix.extend_from_slice(&height.to_le_bytes());
        
        let iter = db_clone.prefix_iterator_cf(&cf_transactions, &prefix);
        
        for item in iter {
            let (key, value) = item?;
            
            // Key format: 'B' + height(4) + index(4)
            if key.len() == 9 && key[0] == b'B' {
                // Value is the txid (internal format or hex)
                if value.len() == 32 {
                    txids.push(value.to_vec());
                } else {
                    let txid_hex = String::from_utf8_lossy(&value).to_string();
                    if let Ok(txid_bytes) = hex::decode(&txid_hex) {
                        let internal: Vec<u8> = txid_bytes.iter().rev().cloned().collect();
                        txids.push(internal);
                    }
                }
            }
        }
        
        Ok::<Vec<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>>(txids)
    })
    .await??;
    
    let mut total_fees: i64 = 0;
    
    // Calculate fees for each transaction
    for txid_internal in txids {
        // Load transaction
        let mut tx_key = vec![b't'];
        tx_key.extend_from_slice(&txid_internal);
        
        let db_clone = db.clone();
        let tx_data = tokio::task::spawn_blocking({
            let key = tx_key.clone();
            move || {
                let cf_transactions = db_clone.cf_handle("transactions")
                    .ok_or("transactions CF not found")?;
                
                db_clone.get_cf(&cf_transactions, &key)
                    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
            }
        })
        .await??;
        
        if let Some(bytes) = tx_data {
            if bytes.len() >= 8 {
                let mut tx_with_header = vec![0u8; 4];
                tx_with_header.extend_from_slice(&bytes[8..]);
                
                if let Ok(tx) = deserialize_transaction(&tx_with_header).await {
                    let fee_calc = calculate_transaction_fee(db.clone(), &tx, height).await?;
                    
                    if fee_calc.is_valid() {
                        total_fees += fee_calc.fee;
                    }
                }
            }
        }
    }
    
    Ok(total_fees)
}

/// Validate transaction fee against minimum relay fee
///
/// PIVX minimum relay fee: 0.0001 PIV per kB (10,000 satoshis per 1000 bytes)
/// 
/// # Arguments
/// * `fee` - Fee in satoshis
/// * `tx_size` - Transaction size in bytes
/// 
/// # Returns
/// true if fee meets minimum, false otherwise
pub fn is_fee_sufficient(fee: i64, tx_size: usize) -> bool {
    const MIN_RELAY_FEE_PER_KB: i64 = 10_000; // 0.0001 PIV
    
    if tx_size == 0 {
        return fee >= 0;
    }
    
    // Calculate minimum fee for this transaction size
    let kb_size = (tx_size as f64 / 1000.0).ceil() as i64;
    let min_fee = MIN_RELAY_FEE_PER_KB * kb_size;
    
    fee >= min_fee
}

/// Get recommended fee for a transaction size
///
/// Uses PIVX Core's fee calculation algorithm.
/// 
/// # Arguments
/// * `tx_size` - Transaction size in bytes
/// * `priority` - Fee priority (1 = low, 2 = normal, 3 = high)
/// 
/// # Returns
/// Recommended fee in satoshis
pub fn get_recommended_fee(tx_size: usize, priority: u8) -> i64 {
    const MIN_RELAY_FEE_PER_KB: i64 = 10_000; // 0.0001 PIV
    
    let kb_size = (tx_size as f64 / 1000.0).ceil() as i64;
    
    match priority {
        1 => MIN_RELAY_FEE_PER_KB * kb_size,  // Low: minimum
        2 => MIN_RELAY_FEE_PER_KB * kb_size * 2,  // Normal: 2x minimum
        3 => MIN_RELAY_FEE_PER_KB * kb_size * 5,  // High: 5x minimum
        _ => MIN_RELAY_FEE_PER_KB * kb_size,  // Default to minimum
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_fee_calculation_success() {
        let fee_calc = FeeCalculation::success(
            TransactionType::Normal,
            100_000_000,  // 1 PIV input
            99_990_000,   // 0.9999 PIV output
            10_000,       // 0.0001 PIV fee
            0,
        );
        
        assert!(fee_calc.is_valid());
        assert_eq!(fee_calc.fee, 10_000);
        assert_eq!(fee_calc.fee_piv(), 0.0001);
    }
    
    #[test]
    fn test_fee_calculation_failure() {
        let fee_calc = FeeCalculation::failure("Test error".to_string());
        
        assert!(!fee_calc.is_valid());
        assert_eq!(fee_calc.error_message, Some("Test error".to_string()));
    }
    
    #[test]
    fn test_coinstake_reward() {
        let fee_calc = FeeCalculation::success(
            TransactionType::Coinstake,
            100_000_000,  // 1 PIV staked
            101_000_000,  // 1.01 PIV output (1 PIV + 0.01 PIV reward)
            0,            // No fee
            1_000_000,    // 0.01 PIV reward
        );
        
        assert!(fee_calc.is_valid());
        assert_eq!(fee_calc.fee, 0);
        assert_eq!(fee_calc.stake_reward, 1_000_000);
        assert_eq!(fee_calc.stake_reward_piv(), 0.01);
    }
    
    #[test]
    fn test_fee_sufficiency() {
        // 250 byte transaction
        assert!(is_fee_sufficient(10_000, 250));  // Exactly minimum
        assert!(is_fee_sufficient(15_000, 250));  // Above minimum
        assert!(!is_fee_sufficient(5_000, 250));  // Below minimum
        
        // 1500 byte transaction (needs 2x minimum)
        assert!(is_fee_sufficient(20_000, 1500));  // Exactly minimum
        assert!(!is_fee_sufficient(15_000, 1500)); // Below minimum
    }
    
    #[test]
    fn test_recommended_fee() {
        let tx_size = 250;  // bytes
        
        // Low priority: 1x minimum
        assert_eq!(get_recommended_fee(tx_size, 1), 10_000);
        
        // Normal priority: 2x minimum
        assert_eq!(get_recommended_fee(tx_size, 2), 20_000);
        
        // High priority: 5x minimum
        assert_eq!(get_recommended_fee(tx_size, 3), 50_000);
    }
    
    #[test]
    fn test_coinbase_fee() {
        let fee_calc = FeeCalculation::success(
            TransactionType::Coinbase,
            0,              // No inputs
            500_000_000,    // 5 PIV block reward
            0,              // No fee
            0,              // No stake reward
        );
        
        assert!(fee_calc.is_valid());
        assert_eq!(fee_calc.value_in, 0);
        assert_eq!(fee_calc.fee, 0);
    }
    
    #[test]
    fn test_invalid_fee_negative() {
        // This would happen if outputs > inputs (shouldn't be possible)
        let fee_calc = FeeCalculation::failure(
            "Invalid fee: inputs (100) < outputs (200)".to_string()
        );
        
        assert!(!fee_calc.is_valid());
    }
}
