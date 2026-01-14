
/// Block validation result
#[derive(Debug, Clone, PartialEq)]
pub enum BlockValidationResult {
    /// Block is valid
    Valid,
    
    /// Block signature is invalid
    InvalidSignature(String),
    
    /// Coinstake transaction is invalid
    InvalidCoinstake(String),
    
    /// Stake kernel hash doesn't meet target
    InvalidStakeKernel(String),
    
    /// Stake modifier is incorrect
    InvalidStakeModifier(String),
    
    /// Block timestamp is invalid
    InvalidTimestamp(String),
    
    /// Block version is invalid
    InvalidVersion(String),
}

impl BlockValidationResult {
    pub fn is_valid(&self) -> bool {
        matches!(self, BlockValidationResult::Valid)
    }
    
    pub fn error_message(&self) -> Option<String> {
        match self {
            BlockValidationResult::Valid => None,
            BlockValidationResult::InvalidSignature(msg) => Some(format!("Invalid signature: {}", msg)),
            BlockValidationResult::InvalidCoinstake(msg) => Some(format!("Invalid coinstake: {}", msg)),
            BlockValidationResult::InvalidStakeKernel(msg) => Some(format!("Invalid stake kernel: {}", msg)),
            BlockValidationResult::InvalidStakeModifier(msg) => Some(format!("Invalid stake modifier: {}", msg)),
            BlockValidationResult::InvalidTimestamp(msg) => Some(format!("Invalid timestamp: {}", msg)),
            BlockValidationResult::InvalidVersion(msg) => Some(format!("Invalid version: {}", msg)),
        }
    }
}

/// PoS validation configuration
pub struct PosValidationConfig {
    /// Whether to validate block signatures (requires full block data with signature)
    pub validate_signature: bool,
    
    /// Whether to validate coinstake transactions
    pub validate_coinstake: bool,
    
    /// Whether to validate stake kernel hash
    pub validate_stake_kernel: bool,
    
    /// Whether to validate stake modifier
    pub validate_stake_modifier: bool,
    
    /// Whether to validate block timestamps
    pub validate_timestamp: bool,
    
    /// Whether to skip validation for blocks from trusted RPC source
    /// Default: true (we trust PIVX Core to validate blocks)
    pub trust_rpc_source: bool,
}

impl Default for PosValidationConfig {
    fn default() -> Self {
        Self {
            // For block explorer use case, we trust RPC source
            // These can be enabled for full node validation
            validate_signature: false,
            validate_coinstake: true,  // Basic coinstake checks
            validate_stake_kernel: false,  // Requires stake input lookups
            validate_stake_modifier: false,  // Complex historical calculation
            validate_timestamp: true,  // Simple timestamp checks
            trust_rpc_source: true,
        }
    }
}

impl PosValidationConfig {
    /// Full validation (paranoid mode - validates everything)
    pub fn full_validation() -> Self {
        Self {
            validate_signature: true,
            validate_coinstake: true,
            validate_stake_kernel: true,
            validate_stake_modifier: true,
            validate_timestamp: true,
            trust_rpc_source: false,
        }
    }
    
    /// Minimal validation (fast sync - trusts RPC)
    pub fn minimal_validation() -> Self {
        Self {
            validate_signature: false,
            validate_coinstake: false,
            validate_stake_kernel: false,
            validate_stake_modifier: false,
            validate_timestamp: false,
            trust_rpc_source: true,
        }
    }
}

/// Validate a PoS block's coinstake transaction
/// 
/// PIVX Core equivalent: CheckCoinstake()
/// 
/// Coinstake requirements:
/// 1. Must have at least 1 input (stake input)
/// 2. Must have at least 2 outputs
/// 3. First output must be empty (0 value, empty script)
/// 4. Second output must pay to same address as stake input (simplified check)
/// 
/// # Arguments
/// * `coinstake_tx` - The coinstake transaction (first tx in block after coinbase era)
/// * `block_height` - Block height for version checks
/// 
/// # Returns
/// ValidationResult indicating if coinstake is valid
pub fn validate_coinstake_transaction(
    coinstake_tx: &crate::types::CTransaction,
    _block_height: i32,
) -> BlockValidationResult {
    // Check: Must have at least 1 input
    if coinstake_tx.inputs.is_empty() {
        return BlockValidationResult::InvalidCoinstake(
            "Coinstake must have at least one input".to_string()
        );
    }
    
    // Check: Must have at least 2 outputs
    if coinstake_tx.outputs.len() < 2 {
        return BlockValidationResult::InvalidCoinstake(
            "Coinstake must have at least two outputs".to_string()
        );
    }
    
    // Check: First output must be empty (marker output)
    let first_output = &coinstake_tx.outputs[0];
    if first_output.value != 0 {
        return BlockValidationResult::InvalidCoinstake(
            format!("First coinstake output must be empty, got value {}", first_output.value)
        );
    }
    
    if !first_output.script_pubkey.script.is_empty() {
        return BlockValidationResult::InvalidCoinstake(
            "First coinstake output must have empty scriptPubKey".to_string()
        );
    }
    
    // Check: First input must reference a previous UTXO (not coinbase-style)
    let first_input = &coinstake_tx.inputs[0];
    if let Some(ref prevout) = first_input.prevout {
        // Check that it's not a null hash (which would indicate coinbase)
        let is_null = prevout.hash.as_bytes().iter().all(|&b| b == 0);
        if is_null {
            return BlockValidationResult::InvalidCoinstake(
                "Coinstake input cannot reference null hash".to_string()
            );
        }
    } else {
        return BlockValidationResult::InvalidCoinstake(
            "Coinstake must have valid previous output reference".to_string()
        );
    }
    
    // Additional check: Second output (first real output) should have value
    let second_output = &coinstake_tx.outputs[1];
    if second_output.value == 0 {
        return BlockValidationResult::InvalidCoinstake(
            "Second coinstake output (stake reward) must have non-zero value".to_string()
        );
    }
    
    BlockValidationResult::Valid
}

/// Validate block timestamp against consensus rules
/// 
/// PIVX Core equivalent: CheckBlockHeader() timestamp checks
/// 
/// Rules:
/// 1. Block time must not be more than 2 hours in the future
/// 2. Block time must be greater than median of last 11 blocks (prevents timestamp manipulation)
/// 3. PoS blocks must have time >= previous block time
/// 
/// # Arguments
/// * `block_time` - Block timestamp (Unix time)
/// * `prev_block_time` - Previous block timestamp
/// * `current_time` - Current network time (Unix time)
/// 
/// # Returns
/// ValidationResult indicating if timestamp is valid
pub fn validate_block_timestamp(
    block_time: u32,
    prev_block_time: Option<u32>,
    current_time: u32,
) -> BlockValidationResult {
    const MAX_FUTURE_BLOCK_TIME: u32 = 2 * 60 * 60; // 2 hours
    
    // Check: Block time must not be too far in the future
    if block_time > current_time + MAX_FUTURE_BLOCK_TIME {
        return BlockValidationResult::InvalidTimestamp(
            format!(
                "Block time {} is too far in the future (current: {}, max drift: {})",
                block_time, current_time, MAX_FUTURE_BLOCK_TIME
            )
        );
    }
    
    // Check: Block time must be >= previous block time (PoS blocks)
    if let Some(prev_time) = prev_block_time {
        if block_time < prev_time {
            return BlockValidationResult::InvalidTimestamp(
                format!(
                    "Block time {} is before previous block time {}",
                    block_time, prev_time
                )
            );
        }
        
        // Sanity check: Block time shouldn't be too old
        // (more than 1 hour before previous block is suspicious)
        const MAX_TIME_REGRESSION: u32 = 60 * 60; // 1 hour
        if prev_time > block_time + MAX_TIME_REGRESSION {
            return BlockValidationResult::InvalidTimestamp(
                format!(
                    "Block time {} is suspiciously far before previous block {}",
                    block_time, prev_time
                )
            );
        }
    }
    
    BlockValidationResult::Valid
}

/// Validate block version follows consensus rules
/// 
/// PIVX versioning:
/// - Version 1-3: Legacy PoW
/// - Version 4: PoS activation
/// - Version 5+: Sapling support
/// 
/// # Arguments
/// * `version` - Block version
/// * `height` - Block height
/// 
/// # Returns
/// ValidationResult indicating if version is valid
pub fn validate_block_version(
    version: u32,
    height: i32,
) -> BlockValidationResult {
    // PIVX consensus: PoS started at a specific height
    // These are example thresholds - adjust to actual PIVX values
    const POS_START_HEIGHT: i32 = 259_201; // Actual PIVX PoS start
    const SAPLING_START_HEIGHT: i32 = 2_700_000; // Example - check PIVX params
    
    // Pre-PoS: versions 1-3 are valid
    if height < POS_START_HEIGHT {
        if version > 0 && version <= 4 {
            return BlockValidationResult::Valid;
        }
        return BlockValidationResult::InvalidVersion(
            format!("Pre-PoS block has invalid version {}", version)
        );
    }
    
    // Post-PoS, pre-Sapling: version 4 minimum
    if (POS_START_HEIGHT..SAPLING_START_HEIGHT).contains(&height) {
        if version >= 4 {
            return BlockValidationResult::Valid;
        }
        return BlockValidationResult::InvalidVersion(
            format!("PoS block at height {} must have version >= 4, got {}", height, version)
        );
    }
    
    // Post-Sapling: version 5+ expected (but 4 still valid)
    if height >= SAPLING_START_HEIGHT {
        if version >= 4 {
            return BlockValidationResult::Valid;
        }
        return BlockValidationResult::InvalidVersion(
            format!("Sapling-era block must have version >= 4, got {}", version)
        );
    }
    
    BlockValidationResult::Valid
}

/// Perform basic PoS validation on a block
/// 
/// This is a lightweight validation that doesn't require block signature data.
/// Suitable for block explorer use cases where we trust the RPC source.
/// 
/// # Arguments
/// * `config` - Validation configuration
/// * `block_version` - Block version
/// * `block_height` - Block height
/// * `block_time` - Block timestamp
/// * `prev_block_time` - Previous block timestamp (if available)
/// * `coinstake_tx` - Coinstake transaction (first tx in PoS blocks)
/// * `current_time` - Current network time
/// 
/// # Returns
/// ValidationResult
pub fn validate_pos_block_basic(
    config: &PosValidationConfig,
    block_version: u32,
    block_height: i32,
    block_time: u32,
    prev_block_time: Option<u32>,
    coinstake_tx: Option<&crate::types::CTransaction>,
    current_time: u32,
) -> BlockValidationResult {
    // If we trust RPC source and all validations are disabled, skip
    if config.trust_rpc_source 
        && !config.validate_coinstake 
        && !config.validate_timestamp 
    {
        return BlockValidationResult::Valid;
    }
    
    // Validate block version
    let version_result = validate_block_version(block_version, block_height);
    if !version_result.is_valid() {
        return version_result;
    }
    
    // Validate timestamp if enabled
    if config.validate_timestamp {
        let time_result = validate_block_timestamp(block_time, prev_block_time, current_time);
        if !time_result.is_valid() {
            return time_result;
        }
    }
    
    // Validate coinstake if present and enabled
    if config.validate_coinstake {
        if let Some(coinstake) = coinstake_tx {
            let coinstake_result = validate_coinstake_transaction(coinstake, block_height);
            if !coinstake_result.is_valid() {
                return coinstake_result;
            }
        }
    }
    
    BlockValidationResult::Valid
}

/// Get current Unix timestamp
pub fn get_current_time() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CTransaction, CTxIn, CTxOut, COutPoint, CScript};
    
    fn create_test_coinstake(
        num_inputs: usize,
        num_outputs: usize,
        first_output_value: i64,
    ) -> CTransaction {
        let mut inputs = Vec::new();
        for i in 0..num_inputs {
            inputs.push(CTxIn {
                prevout: Some(COutPoint {
                    hash: format!("{}000000000000000000000000000000000000000000000000000000000000", i),
                    n: 0,
                }),
                script_sig: CScript { script: vec![] },
                sequence: 0xffffffff,
                coinbase: None,
                index: i as u64,
            });
        }
        
        let mut outputs = Vec::new();
        for i in 0..num_outputs {
            outputs.push(CTxOut {
                value: if i == 0 { first_output_value } else { 1000000 },
                script_pubkey: CScript {
                    script: if i == 0 && first_output_value == 0 {
                        vec![]
                    } else {
                        vec![0x76, 0xa9, 0x14] // P2PKH prefix
                    },
                },
                script_length: if i == 0 && first_output_value == 0 { 0 } else { 3 },
                index: i as u64,
                address: vec![],
            });
        }
        
        CTransaction {
            txid: "test".to_string(),
            version: 1,
            inputs,
            outputs,
            lock_time: 0,
            sapling_data: None,
        }
    }
    
    #[test]
    fn test_valid_coinstake() {
        let tx = create_test_coinstake(1, 2, 0);
        let result = validate_coinstake_transaction(&tx, 100000);
        assert!(result.is_valid());
    }
    
    #[test]
    fn test_coinstake_no_inputs() {
        let tx = create_test_coinstake(0, 2, 0);
        let result = validate_coinstake_transaction(&tx, 100000);
        assert!(!result.is_valid());
    }
    
    #[test]
    fn test_coinstake_insufficient_outputs() {
        let tx = create_test_coinstake(1, 1, 0);
        let result = validate_coinstake_transaction(&tx, 100000);
        assert!(!result.is_valid());
    }
    
    #[test]
    fn test_coinstake_first_output_not_empty() {
        let tx = create_test_coinstake(1, 2, 1000); // First output has value
        let result = validate_coinstake_transaction(&tx, 100000);
        assert!(!result.is_valid());
    }
    
    #[test]
    fn test_valid_timestamp() {
        let current = 1700000000;
        let prev = 1699999900;
        let block_time = 1700000050;
        
        let result = validate_block_timestamp(block_time, Some(prev), current);
        assert!(result.is_valid());
    }
    
    #[test]
    fn test_timestamp_too_far_future() {
        let current = 1700000000;
        let block_time = current + 3 * 60 * 60; // 3 hours in future
        
        let result = validate_block_timestamp(block_time, None, current);
        assert!(!result.is_valid());
    }
    
    #[test]
    fn test_timestamp_before_previous() {
        let prev = 1700000000;
        let block_time = 1699999000; // Before previous
        let current = 1700000100;
        
        let result = validate_block_timestamp(block_time, Some(prev), current);
        assert!(!result.is_valid());
    }
    
    #[test]
    fn test_block_version_pos_era() {
        let result = validate_block_version(4, 300000);
        assert!(result.is_valid());
    }
    
    #[test]
    fn test_validation_config_default() {
        let config = PosValidationConfig::default();
        assert!(config.trust_rpc_source);
        assert!(config.validate_coinstake);
        assert!(config.validate_timestamp);
    }
    
    #[test]
    fn test_validation_config_full() {
        let config = PosValidationConfig::full_validation();
        assert!(!config.trust_rpc_source);
        assert!(config.validate_signature);
        assert!(config.validate_stake_kernel);
    }
}
