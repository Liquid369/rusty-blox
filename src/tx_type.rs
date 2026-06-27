/// Transaction Type Detection - PIVX Core Conformant
///
/// This module provides authoritative transaction type classification
/// matching PIVX Core's consensus rules.
///
/// PIVX Transaction Types (primitives/transaction.cpp):
/// 1. Coinbase: IsCoinBase() = vin.size() == 1 && vin[0].prevout.IsNull() && !ContainsZerocoins()
/// 2. Coinstake: IsCoinStake() = !vin.empty()
///       && (!vin[0].prevout.IsNull() || vin[0].IsZerocoinSpend())  // null prevout DISQUALIFIES unless zPoS
///       && vout.size() >= 2 && vout[0].IsEmpty()
///    i.e. a coinstake SPENDS A REAL STAKE OUTPOINT; only zerocoin (zPoS) stakes have a null prevout.
/// 3. Normal: everything else
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

/// Coinstake maturity: PIVX Core's GetBlocksToMaturity() applies COINBASE_MATURITY (100)
/// to both coinbase and coinstake. (600 is nStakeMinDepth — the depth an input needs
/// before it may STAKE — not a spend-maturity rule.)
pub const COINSTAKE_MATURITY: u32 = 100;

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

/// First byte of an input's signature script, regardless of whether the parser
/// stored it in `script_sig` or moved it into the legacy `coinbase` field.
fn input_script_first_byte(input: &CTxIn) -> Option<u8> {
    if !input.script_sig.script.is_empty() {
        input.script_sig.script.first().copied()
    } else {
        input.coinbase.as_ref().and_then(|cb| cb.first().copied())
    }
}

/// CTxIn::IsZerocoinSpend(): null prevout + scriptSig starts with OP_ZEROCOINSPEND (0xc2)
fn is_zerocoin_spend_input(input: &CTxIn) -> bool {
    is_prevout_null(input) && input_script_first_byte(input) == Some(0xc2)
}

/// scriptSig starts with OP_ZEROCOINPUBLICSPEND (0xc3)
fn is_zerocoin_public_spend_input(input: &CTxIn) -> bool {
    input_script_first_byte(input) == Some(0xc3)
}

/// Detect transaction type using PIVX Core's rules
///
/// This matches the exact logic from PIVX Core (primitives/transaction.cpp):
/// - `IsCoinBase()`: `vin.size() == 1 && vin[0].prevout.IsNull() && !ContainsZerocoins()`
/// - `IsCoinStake()`: `!vin.empty()` && (`vin[0].prevout` NOT null, unless `vin[0]` is a
///   zerocoin spend / zPoS) && `vout.size() >= 2 && vout[0].IsEmpty()`
///
/// A coinstake spends a REAL stake outpoint — a null first prevout disqualifies it
/// (except zerocoin stakes). This is the opposite of coinbase.
pub fn detect_transaction_type(tx: &CTransaction) -> TransactionType {
    detect_type_from_components(&tx.inputs, &tx.outputs)
}

/// Detect transaction type from raw input/output data (before CTransaction is built)
///
/// This is used during transaction parsing when we don't have a full CTransaction yet.
pub fn detect_type_from_components(inputs: &[CTxIn], outputs: &[CTxOut]) -> TransactionType {
    if inputs.is_empty() {
        return TransactionType::Normal;
    }

    let first = &inputs[0];
    let first_null = is_prevout_null(first);

    // IsCoinStake(): fAllowNull = vin[0].IsZerocoinSpend()
    if (!first_null || is_zerocoin_spend_input(first))
        && outputs.len() >= 2
        && is_output_empty(&outputs[0])
    {
        return TransactionType::Coinstake;
    }

    // IsCoinBase(): exactly one input with null prevout and no zerocoin content
    // (zerocoin spends/public spends also carry null-ish prevouts but are NOT coinbase)
    let has_zc_mint_output = outputs
        .iter()
        .any(|o| o.script_pubkey.script.first() == Some(&0xc1)); // OP_ZEROCOINMINT
    if inputs.len() == 1
        && first_null
        && !is_zerocoin_spend_input(first)
        && !is_zerocoin_public_spend_input(first)
        && !has_zc_mint_output
    {
        return TransactionType::Coinbase;
    }

    TransactionType::Normal
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
    use crate::types::{COutPoint, CScript};

    #[test]
    fn test_coinbase_detection() {
        // Coinbase: null prevout, non-empty first output
        let inputs = vec![CTxIn {
            prevout: Some(COutPoint {
                hash: "0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
                n: 0xffffffff,
            }),
            script_sig: CScript { script: vec![] },
            sequence: 0,
            index: 0,
            coinbase: None,
        }];

        let outputs = vec![CTxOut {
            value: 50_0000_0000, // 50 PIVX
            script_pubkey: CScript {
                script: vec![0x76, 0xa9],
            }, // Non-empty
            script_length: 2,
            index: 0,
            address: vec![],
        }];

        assert_eq!(
            detect_type_from_components(&inputs, &outputs),
            TransactionType::Coinbase
        );
    }

    #[test]
    fn test_coinstake_detection() {
        // Coinstake: REAL stake prevout (PIVX Core), empty first output
        let inputs = vec![CTxIn {
            prevout: Some(COutPoint {
                hash: "abc123def456abc123def456abc123def456abc123def456abc123def456abc1"
                    .to_string(),
                n: 1,
            }),
            script_sig: CScript { script: vec![0x47] },
            sequence: 0,
            index: 0,
            coinbase: None,
        }];

        let outputs = vec![
            CTxOut {
                value: 0,                                  // Empty output
                script_pubkey: CScript { script: vec![] }, // Empty script
                script_length: 0,
                index: 0,
                address: vec![],
            },
            CTxOut {
                value: 100_0000_0000, // Stake reward
                script_pubkey: CScript {
                    script: vec![0x76, 0xa9],
                },
                script_length: 2,
                index: 1,
                address: vec![],
            },
        ];

        assert_eq!(
            detect_type_from_components(&inputs, &outputs),
            TransactionType::Coinstake
        );
    }

    #[test]
    fn test_normal_transaction() {
        // Normal: real prevout
        let inputs = vec![CTxIn {
            prevout: Some(COutPoint {
                hash: "abc123def456abc123def456abc123def456abc123def456abc123def456abc1"
                    .to_string(),
                n: 0,
            }),
            script_sig: CScript { script: vec![0x47] },
            sequence: 0xffffffff,
            index: 0,
            coinbase: None,
        }];

        let outputs = vec![CTxOut {
            value: 10_0000_0000,
            script_pubkey: CScript {
                script: vec![0x76, 0xa9],
            },
            script_length: 2,
            index: 0,
            address: vec![],
        }];

        assert_eq!(
            detect_type_from_components(&inputs, &outputs),
            TransactionType::Normal
        );
    }

    #[test]
    fn test_maturity_coinbase() {
        // Coinbase at height 1000
        assert!(!is_output_spendable(TransactionType::Coinbase, 1000, 1050)); // Only 50 blocks
        assert!(!is_output_spendable(TransactionType::Coinbase, 1000, 1099)); // 99 blocks
        assert!(is_output_spendable(TransactionType::Coinbase, 1000, 1100)); // Exactly 100 blocks
        assert!(is_output_spendable(TransactionType::Coinbase, 1000, 1200)); // 200 blocks
    }

    #[test]
    fn test_maturity_coinstake() {
        // Coinstake at height 1000 — PIVX Core applies COINBASE_MATURITY (100) to coinstake too
        assert!(!is_output_spendable(TransactionType::Coinstake, 1000, 1050)); // Only 50 blocks
        assert!(!is_output_spendable(TransactionType::Coinstake, 1000, 1099)); // 99 blocks
        assert!(is_output_spendable(TransactionType::Coinstake, 1000, 1100)); // Exactly 100 blocks
        assert!(is_output_spendable(TransactionType::Coinstake, 1000, 2000)); // 1000 blocks
    }

    #[test]
    fn test_zerocoin_stake_detection() {
        // zPoS coinstake: null prevout IS allowed when scriptSig starts with OP_ZEROCOINSPEND
        let inputs = vec![CTxIn {
            prevout: Some(COutPoint {
                hash: "0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
                n: 0xffffffff,
            }),
            script_sig: CScript {
                script: vec![0xc2, 0x01],
            },
            sequence: 0,
            index: 0,
            coinbase: None,
        }];

        let outputs = vec![
            CTxOut {
                value: 0,
                script_pubkey: CScript { script: vec![] },
                script_length: 0,
                index: 0,
                address: vec![],
            },
            CTxOut {
                value: 100_0000_0000,
                script_pubkey: CScript {
                    script: vec![0x76, 0xa9],
                },
                script_length: 2,
                index: 1,
                address: vec![],
            },
        ];

        assert_eq!(
            detect_type_from_components(&inputs, &outputs),
            TransactionType::Coinstake
        );
    }

    #[test]
    fn test_zerocoin_spend_not_coinbase() {
        // A zerocoin spend has a null prevout but is NOT coinbase (Core: !ContainsZerocoins())
        let inputs = vec![CTxIn {
            prevout: Some(COutPoint {
                hash: "0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
                n: 0xffffffff,
            }),
            script_sig: CScript {
                script: vec![0xc2, 0x01],
            },
            sequence: 0,
            index: 0,
            coinbase: None,
        }];

        let outputs = vec![CTxOut {
            value: 10_0000_0000,
            script_pubkey: CScript {
                script: vec![0x76, 0xa9],
            },
            script_length: 2,
            index: 0,
            address: vec![],
        }];

        assert_eq!(
            detect_type_from_components(&inputs, &outputs),
            TransactionType::Normal
        );
    }

    #[test]
    fn test_maturity_normal() {
        // Normal transactions are always spendable
        assert!(is_output_spendable(TransactionType::Normal, 1000, 1000)); // Same block
        assert!(is_output_spendable(TransactionType::Normal, 1000, 1001)); // Next block
    }
}
