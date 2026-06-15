use std::sync::Arc;
use rocksdb::DB;
use num_bigint::BigUint;
use num_traits::{One, Zero};
use tracing::{info, debug};

/// Calculate the work (difficulty) represented by a compact target (nBits)
/// 
/// PIVX consensus: The best chain is the one with the most accumulated work.
/// Work for a block = 2^256 / (target + 1)
/// 
/// This is the core mechanism that determines the canonical chain without external validation.
/// Returns work as a 256-bit value stored in a 32-byte array (big-endian).
pub fn calculate_work_from_bits(n_bits: u32) -> [u8; 32] {
    // Extract exponent and mantissa from compact representation
    let exponent = n_bits >> 24;
    let mantissa = n_bits & 0x00ffffff;
    
    // Special case: invalid/negative target
    if mantissa == 0 || exponent == 0 {
        return [0u8; 32];
    }
    
    // Calculate target from compact form using BigUint
    // target = mantissa * 256^(exponent - 3)
    let target = if exponent <= 3 {
        // Small target: shift right
        BigUint::from(mantissa >> (8 * (3 - exponent)))
    } else {
        // Larger target: shift left
        let shift_bytes = exponent - 3;
        BigUint::from(mantissa) << (8 * shift_bytes)
    };
    
    // Avoid division by zero
    if target.is_zero() {
        return [0u8; 32];
    }
    
    // Work calculation using full 256-bit precision
    // Work = 2^256 / (target + 1)
    let numerator = BigUint::one() << 256;  // 2^256
    let denominator = target + BigUint::one();
    let work: BigUint = numerator / denominator;
    
    // Convert to 32-byte array (big-endian)
    let work_bytes = work.to_bytes_be();
    let mut result = [0u8; 32];
    let start = 32 - work_bytes.len();
    result[start..].copy_from_slice(&work_bytes);
    result
}

/// Add two 256-bit chainwork values
fn add_chainwork(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let a_big = BigUint::from_bytes_be(a);
    let b_big = BigUint::from_bytes_be(b);
    let sum = a_big + b_big;
    
    let sum_bytes = sum.to_bytes_be();
    let mut result = [0u8; 32];
    let start = 32 - sum_bytes.len().min(32);
    result[start..].copy_from_slice(&sum_bytes[..sum_bytes.len().min(32)]);
    result
}

/// Compare two 256-bit chainwork values
fn compare_chainwork(a: &[u8; 32], b: &[u8; 32]) -> std::cmp::Ordering {
    a.cmp(b)
}

/// Calculate accumulated chainwork for all blocks in database
/// Returns a map of block_hash -> total_chainwork_from_genesis
/// Uses iterative topological sort to avoid stack overflow with millions of blocks
///
/// `blocks_map` maps block_hash -> (prev_hash, n_bits). These are the ONLY two
/// header fields the chainwork BFS reads, so passing them pre-extracted (rather
/// than the full ~80-byte header) is byte-for-byte equivalent to the previous
/// `(hash -> header_bytes)` form while avoiding a duplicate full-header copy and
/// the internal re-parse. The genesis-detection (all-zero prev_hash), the
/// per-block `calculate_work_from_bits`, the cumulative `add_chainwork`, the
/// keep-maximum fork rule and the BFS order are all unchanged.
pub fn calculate_all_chainwork(
    _db: &Arc<DB>,
    blocks_map: &std::collections::HashMap<[u8; 32], ([u8; 32], u32)>,
    children_map: &std::collections::HashMap<[u8; 32], Vec<[u8; 32]>>,
) -> Result<std::collections::HashMap<[u8; 32], [u8; 32]>, Box<dyn std::error::Error>> {

    use std::collections::{HashMap, VecDeque};

    // Block hashes are fixed 32-byte values; keying maps by `[u8; 32]` (inline,
    // Copy) instead of `Vec<u8>` removes a separate heap allocation + 24-byte
    // header per hash across ~5.5M blocks, cutting RSS and allocator churn with
    // byte-identical behavior. All `.clone()` calls below copy the array.
    let mut chainwork_map: HashMap<[u8; 32], [u8; 32]> = HashMap::new();

    // `blocks_map` already holds (prev_hash, n_bits) per block, so we reference it
    // directly instead of materialising separate parent_map + bits_map copies.
    info!(blocks = blocks_map.len(), "Calculating chainwork for all blocks");

    // Find all genesis blocks (blocks with all-zero prev_hash)
    let mut queue: VecDeque<[u8; 32]> = VecDeque::new();
    for (block_hash, (prev_hash, n_bits)) in blocks_map {
        if prev_hash.iter().all(|&b| b == 0) {
            // Genesis block
            let work = calculate_work_from_bits(*n_bits);
            chainwork_map.insert(block_hash.clone(), work);
            queue.push_back(block_hash.clone());
        }
    }

    debug!(count = queue.len(), "Genesis blocks found");

    // `children_map` is now passed in by the caller (resolve_block_heights builds
    // an identical prev_hash -> [block_hash] map for tip discovery), so we no
    // longer rebuild a duplicate ~5.5M-entry copy that lived in RAM for the whole
    // BFS. The guard inside the loop restricts traversal to children present in
    // `blocks_map`, exactly matching the old internal map (built only from
    // blocks_map keys); child order within a parent's list does not affect the
    // result (each block's chainwork is its unique path sum; forks keep the max).

    // Iterative BFS to calculate chainwork
    let mut processed = queue.len();
    let zero_work = [0u8; 32];
    while let Some(current_hash) = queue.pop_front() {
        let current_chainwork = *chainwork_map.get(&current_hash).unwrap_or(&zero_work);

        // Process all children
        if let Some(children) = children_map.get(&current_hash) {
            for child_hash in children {
                // Skip children absent from blocks_map (the caller's map may
                // include headers < 80 bytes): the previous internal children map
                // was built only from blocks_map keys, so this keeps the traversed
                // set — and the resulting chainwork — byte-identical.
                if !blocks_map.contains_key(child_hash) {
                    continue;
                }
                let child_work = calculate_work_from_bits(
                    blocks_map.get(child_hash).map(|(_, b)| *b).unwrap_or(0)
                );
                let child_chainwork = add_chainwork(&current_chainwork, &child_work);
                
                // Handle forks: keep the maximum chainwork if block was already processed
                let should_update = if let Some(existing) = chainwork_map.get(child_hash) {
                    compare_chainwork(&child_chainwork, existing) == std::cmp::Ordering::Greater
                } else {
                    true
                };
                
                if should_update {
                    chainwork_map.insert(child_hash.clone(), child_chainwork);
                    queue.push_back(child_hash.clone());
                    
                    processed += 1;
                    if processed % 100000 == 0 {
                        debug!(processed = processed, "Chainwork calculation progress");
                    }
                }
            }
        }
    }
    
    info!(blocks = chainwork_map.len(), "Chainwork calculation complete");
    
    Ok(chainwork_map)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_work_calculation() {
        // Example from Bitcoin: difficulty 1 target
        // nBits = 0x1d00ffff (486604799 in decimal)
        let bits = 0x1d00ffff;
        let work = calculate_work_from_bits(bits);
        
        // Work should be non-zero
        assert_ne!(work, [0u8; 32]);
        println!("Work for difficulty 1: {:?}", hex::encode(work));
        
        // Higher difficulty (lower target) should have more work
        let higher_difficulty_bits = 0x1b0404cb; // Example from a later Bitcoin block
        let higher_work = calculate_work_from_bits(higher_difficulty_bits);
        
        assert!(compare_chainwork(&higher_work, &work) == std::cmp::Ordering::Greater, 
                "Higher difficulty should yield more work");
    }
    
    #[test]
    fn test_zero_bits() {
        assert_eq!(calculate_work_from_bits(0), [0u8; 32]);
    }
}
