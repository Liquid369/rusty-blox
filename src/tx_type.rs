/// Transaction Type Detection - PIVX Core Conformant
/// 
/// This module provides authoritative transaction type classification
/// matching PIVX Core's consensus rules.
/// 
/// PIVX Transaction Types:
/// 1. Coinbase: Mining reward, first input has null prevout (all zeros + 0xffffffff)
/// 2. Coinstake: PoS reward, first input has null prevout + first output is empty (0 value, 0-length script)
/// 3. Normal: Regular transaction with real prevouts
/// 
/// CRITICAL: This matches PIVX Core's CTransaction::IsCoinBase() and CTransaction::IsCoinStake()

use crate::types::{CTransaction, CTxIn, CTxOut};

/// Transaction type classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionType {
    /// Mining reward transaction (PoW or initial blocks)
    Coinbase,
    /// Proof-of-Stake reward transaction
    Coinstake,
    /// Regular transaction
    Normal,
}

impl TransactionType {
    /// Returns true if this type has maturity requirements
    pub fn requires_maturity(&self) -> bool {
        matches!(self, TransactionType::Coinbase | TransactionType::Coinstake)
    }
    
    /// Get maturity block count (how many blocks must pass before outputs can be spent)
    pub fn maturity_blocks(&self) -> u32 {
        match self {
            TransactionType::Coinbase => COINBASE_MATURITY,
            TransactionType::Coinstake => COINSTAKE_MATURITY,
            TransactionType::Normal => 0,
        }
    }
}

/// Coinbase maturity: 100 blocks (PIVX consensus rule)
pub const COINBASE_MATURITY: u32 = 100;

/// Coinstake maturity: 600 blocks (PIVX consensus rule)
pub const COINSTAKE_MATURITY: u32 = 600;

/// Check if a prevout is null (coinbase/coinstake marker)
/// 
/// PIVX Core logic (primitives/transaction.h):
/// ```cpp
/// bool IsNull() const { return (hash.IsNull() && n == (uint32_t) -1); }
/// ```
fn is_prevout_null(input: &CTxIn) -> bool {
    if let Some(ref prevout) = input.prevout {
        // Check if hash is all zeros (displayed as 64 '0' characters in hex)
        let is_null_hash = prevout.hash.chars().all(|c| c == '0');
        // Check if index is 0xffffffff (4294967295)
        let is_null_index = prevout.n == 0xffffffff;
        
        is_null_hash && is_null_index
    } else {
        // Legacy format where coinbase is stored separately
        input.coinbase.is_some()
    }
}

/// Check if an output is empty (coinstake marker)
/// 
/// PIVX Core logic: first output of coinstake has nValue=0 and empty scriptPubKey
fn is_output_empty(output: &CTxOut) -> bool {
    output.value == 0 && output.script_pubkey.script.is_empty()
}

/// Detect transaction type using PIVX Core's rules
/// 
/// This matches the exact logic from PIVX Core:
/// - CTransaction::IsCoinBase(): vin.size() > 0 && vin[0].prevout.IsNull()
/// - CTransaction::IsCoinStake(): vin.size() > 0 && vin[0].prevout.IsNull() && vout.size() >= 2 && vout[0].IsEmpty()
/// 
/// Reference: PIVX Core src/primitives/transaction.h
pub fn detect_transaction_type(tx: &CTransaction) -> TransactionType {
    // Must have at least one input
    if tx.inputs.is_empty() {
        return TransactionType::Normal;
    }
    
    // Check if first input has null prevout
    let first_input_null = is_prevout_null(&tx.inputs[0]);
    
    if !first_input_null {
        // Regular transaction - has real prevout
        return TransactionType::Normal;
    }
    
    // First input is null - could be coinbase or coinstake
    // Coinstake requires:
    // 1. At least 2 outputs (vout.size() >= 2)
    // 2. First output is empty (vout[0].IsEmpty())
    
    if tx.outputs.len() >= 2 {
        if let Some(first_output) = tx.outputs.first() {
            if is_output_empty(first_output) {
                // Coinstake: null input + empty first output
                return TransactionType::Coinstake;
            }
        }
    }
    
    // Coinbase: null input but NOT coinstake
    TransactionType::Coinbase
}

/// Detect transaction type from raw input/output data (before CTransaction is built)
/// 
/// This is used during transaction parsing when we don't have a full CTransaction yet.
pub fn detect_type_from_components(
    inputs: &[CTxIn],
    outputs: &[CTxOut],
) -> TransactionType {
    if inputs.is_empty() {
        return TransactionType::Normal;
    }
    
    let first_input_null = is_prevout_null(&inputs[0]);
    
    if !first_input_null {
        return TransactionType::Normal;
    }
    
    // Check for coinstake pattern
    if outputs.len() >= 2 {
        if let Some(first_output) = outputs.first() {
            if is_output_empty(first_output) {
                return TransactionType::Coinstake;
            }
        }
    }
    
    TransactionType::Coinbase
}

/// Check if a transaction can spend a specific output based on maturity rules
/// 
/// Returns true if the output is mature enough to be spent at current_height
pub fn is_output_spendable(
    output_tx_type: TransactionType,
    output_height: i32,
    current_height: i32,
) -> bool {
    if !output_tx_type.requires_maturity() {
        // Normal transactions have no maturity requirement
        return true;
    }
    
    let maturity = output_tx_type.maturity_blocks() as i32;
    let required_height = output_height.saturating_add(maturity);
    
    current_height >= required_height
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CScript, COutPoint};
    
    #[test]
    fn test_coinbase_detection() {
        // Coinbase: null prevout, non-empty first output
        let inputs = vec![CTxIn {
            prevout: Some(COutPoint {
                hash: "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
                n: 0xffffffff,
            }),
            script_sig: CScript { script: vec![] },
            sequence: 0,
            index: 0,
            coinbase: None,
        }];
        
        let outputs = vec![CTxOut {
            value: 50_0000_0000, // 50 PIVX
            script_pubkey: CScript { script: vec![0x76, 0xa9] }, // Non-empty
            script_length: 2,
            index: 0,
            address: vec![],
        }];
        
        assert_eq!(detect_type_from_components(&inputs, &outputs), TransactionType::Coinbase);
    }
    
    #[test]
    fn test_coinstake_detection() {
        // Coinstake: null prevout, empty first output
        let inputs = vec![CTxIn {
            prevout: Some(COutPoint {
                hash: "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
                n: 0xffffffff,
            }),
            script_sig: CScript { script: vec![] },
            sequence: 0,
            index: 0,
            coinbase: None,
        }];
        
        let outputs = vec![
            CTxOut {
                value: 0, // Empty output
                script_pubkey: CScript { script: vec![] }, // Empty script
                script_length: 0,
                index: 0,
                address: vec![],
            },
            CTxOut {
                value: 100_0000_0000, // Stake reward
                script_pubkey: CScript { script: vec![0x76, 0xa9] },
                script_length: 2,
                index: 1,
                address: vec![],
            },
        ];
        
        assert_eq!(detect_type_from_components(&inputs, &outputs), TransactionType::Coinstake);
    }
    
    #[test]
    fn test_normal_transaction() {
        // Normal: real prevout
        let inputs = vec![CTxIn {
            prevout: Some(COutPoint {
                hash: "abc123def456abc123def456abc123def456abc123def456abc123def456abc1".to_string(),
                n: 0,
            }),
            script_sig: CScript { script: vec![0x47] },
            sequence: 0xffffffff,
            index: 0,
            coinbase: None,
        }];
        
        let outputs = vec![CTxOut {
            value: 10_0000_0000,
            script_pubkey: CScript { script: vec![0x76, 0xa9] },
            script_length: 2,
            index: 0,
            address: vec![],
        }];
        
        assert_eq!(detect_type_from_components(&inputs, &outputs), TransactionType::Normal);
    }
    
    #[test]
    fn test_maturity_coinbase() {
        // Coinbase at height 1000
        assert!(!is_output_spendable(TransactionType::Coinbase, 1000, 1050)); // Only 50 blocks
        assert!(!is_output_spendable(TransactionType::Coinbase, 1000, 1099)); // 99 blocks
        assert!(is_output_spendable(TransactionType::Coinbase, 1000, 1100));  // Exactly 100 blocks
        assert!(is_output_spendable(TransactionType::Coinbase, 1000, 1200));  // 200 blocks
    }
    
    #[test]
    fn test_maturity_coinstake() {
        // Coinstake at height 1000
        assert!(!is_output_spendable(TransactionType::Coinstake, 1000, 1500)); // Only 500 blocks
        assert!(!is_output_spendable(TransactionType::Coinstake, 1000, 1599)); // 599 blocks
        assert!(is_output_spendable(TransactionType::Coinstake, 1000, 1600));  // Exactly 600 blocks
        assert!(is_output_spendable(TransactionType::Coinstake, 1000, 2000));  // 1000 blocks
    }
    
    #[test]
    fn test_maturity_normal() {
        // Normal transactions are always spendable
        assert!(is_output_spendable(TransactionType::Normal, 1000, 1000)); // Same block
        assert!(is_output_spendable(TransactionType::Normal, 1000, 1001)); // Next block
    }
}
