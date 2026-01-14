use std::sync::Arc;
use rocksdb::DB;
use num_bigint::BigUint;
use num_traits::{One, Zero};

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
pub fn calculate_all_chainwork(
    _db: &Arc<DB>,
    blocks_map: &std::collections::HashMap<Vec<u8>, Vec<u8>>,
) -> Result<std::collections::HashMap<Vec<u8>, [u8; 32]>, Box<dyn std::error::Error>> {
    
    use std::collections::{HashMap, VecDeque};
    
    let mut chainwork_map: HashMap<Vec<u8>, [u8; 32]> = HashMap::new();
    
    // Build parent_map and extract n_bits for each block
    let mut parent_map: HashMap<Vec<u8>, Vec<u8>> = HashMap::new(); // hash -> prev_hash
    let mut bits_map: HashMap<Vec<u8>, u32> = HashMap::new();       // hash -> n_bits
    
    for (block_hash, header_bytes) in blocks_map {
        if header_bytes.len() < 80 {
            continue;
        }
        
        let mut prev_hash = [0u8; 32];
        prev_hash.copy_from_slice(&header_bytes[4..36]);
        
        let mut n_bits_bytes = [0u8; 4];
        n_bits_bytes.copy_from_slice(&header_bytes[72..76]);
        let n_bits = u32::from_le_bytes(n_bits_bytes);
        
        parent_map.insert(block_hash.clone(), prev_hash.to_vec());
        bits_map.insert(block_hash.clone(), n_bits);
    }
    
    println!("📊 Calculating chainwork for {} blocks...", parent_map.len());
    
    // Find all genesis blocks (blocks with all-zero prev_hash)
    let mut queue: VecDeque<Vec<u8>> = VecDeque::new();
    for (block_hash, prev_hash) in &parent_map {
        if prev_hash.iter().all(|&b| b == 0) {
            // Genesis block
            let work = calculate_work_from_bits(*bits_map.get(block_hash).unwrap_or(&0));
            chainwork_map.insert(block_hash.clone(), work);
            queue.push_back(block_hash.clone());
        }
    }
    
    println!("  Found {} genesis block(s)", queue.len());
    
    // Build children map for forward traversal
    let mut children_map: HashMap<Vec<u8>, Vec<Vec<u8>>> = HashMap::new();
    for (block_hash, prev_hash) in &parent_map {
        children_map.entry(prev_hash.clone())
            .or_default()
            .push(block_hash.clone());
    }
    
    // Iterative BFS to calculate chainwork
    let mut processed = queue.len();
    let zero_work = [0u8; 32];
    while let Some(current_hash) = queue.pop_front() {
        let current_chainwork = *chainwork_map.get(&current_hash).unwrap_or(&zero_work);
        
        // Process all children
        if let Some(children) = children_map.get(&current_hash) {
            for child_hash in children {
                let child_work = calculate_work_from_bits(*bits_map.get(child_hash).unwrap_or(&0));
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
                        println!("    Calculated chainwork for {} blocks...", processed);
                    }
                }
            }
        }
    }
    
    println!("✅ Chainwork calculated for {} blocks", chainwork_map.len());
    
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
